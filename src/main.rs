use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, thread};

use chrono::{DateTime, Utc};
use clap::Parser;
use confique::Config;
use gen::cli::{Cli, Subcommands};
use gen::config::{Conf, EnqueueTrigger, Forge};
use gen::config_error::handle_config_load_error;
use gen::edit::edit_files_for_pr;
use gen::forge::{ForgeClient, Mergeability};
use gen::github::GitHub;
use gen::gitlab::GitLab;
use gen::process::{git, run_cmd, try_git, try_git_quiet};
use gen::repo::{self, RepoRef};
use gen::trunk::{submit_pull_request, upload_targets};
use rand::Rng;
use serde_json::{to_string_pretty, Value};
use walkdir::WalkDir;

/// One client per configured token, for round-robin use; exits if there are none.
fn forge_clients(config: &Conf, cli: &Cli) -> Vec<ForgeClient> {
    let clients: Vec<ForgeClient> = match config.forge {
        Forge::GitHub => cli
            .get_github_tokens()
            .into_iter()
            .map(|token| ForgeClient::GitHub { token })
            .collect(),
        Forge::GitLab => {
            let project = repo::origin().unwrap_or_else(|e| {
                eprintln!("Failed to get repository information: {}", e);
                std::process::exit(1);
            });
            cli.get_gitlab_tokens()
                .into_iter()
                .map(|token| {
                    ForgeClient::GitLab(GitLab::new(&config.gitlab.url, &project.path(), &token))
                })
                .collect()
        }
    };
    if clients.is_empty() {
        match config.forge {
            Forge::GitHub => eprintln!(
                "No GitHub tokens provided. Use --gh-token to specify at least one token."
            ),
            Forge::GitLab => eprintln!(
                "No GitLab tokens provided. Use --gitlab-token or GITLAB_TOKEN to specify at least one token."
            ),
        }
        std::process::exit(1);
    }
    clients
}

/// The repo as Trunk's API identifies it: origin's path on the configured forge host.
fn trunk_repo(config: &Conf) -> Result<RepoRef, String> {
    let origin = repo::origin()?;
    Ok(RepoRef {
        host: config.forge_host()?,
        ..origin
    })
}

fn get_txt_files(config: &Conf) -> std::io::Result<Vec<PathBuf>> {
    let mut path = std::env::current_dir()?;
    path.push(&config.pullrequest.change_code_path);
    let mut paths = Vec::new();
    for entry in WalkDir::new(&path) {
        let entry = entry?;
        if entry.file_type().is_file()
            && entry.path().extension().and_then(std::ffi::OsStr::to_str) == Some("txt")
        {
            paths.push(entry.path().to_path_buf());
        }
    }
    Ok(paths)
}

fn housekeeping(config: &Conf, forge: &ForgeClient) {
    for _ in 0..3 {
        let requests = forge.open_requests().expect("Failed to list PRs");

        let mut has_unknown = false;
        let mut requeued: HashSet<String> = HashSet::new();
        for item in &requests {
            let pr = &item.number;
            match item.mergeability {
                Mergeability::Unknown => {
                    has_unknown = true;
                }
                Mergeability::Conflicting => {
                    forge.close(pr);
                    println!("closed pr: {} (had merge conflicts)", pr);
                }
                Mergeability::Mergeable => {
                    if requeued.contains(pr) {
                        continue;
                    }
                    'comment: for comment in item.comments.iter() {
                        let body = comment.body.as_str();
                        let created_at_str = comment.created_at.as_str();

                        if created_at_str.is_empty() {
                            continue 'comment;
                        }

                        let stale_age = Utc::now() - config.close_stale_after_duration();
                        let created_at = DateTime::parse_from_rfc3339(created_at_str)
                            .expect("Unable to parse datetime")
                            .with_timezone(&Utc);

                        if created_at > stale_age {
                            continue 'comment; // The datetime was less than stale age
                        }

                        if config
                            .pullrequest
                            .detect_stale_pr_comments
                            .iter()
                            .any(|s| body.contains(s))
                        {
                            forge.close(pr);
                            println!("closed stale pr: {}", pr);
                            requeued.insert(pr.to_owned());
                            break 'comment;
                        }
                    }
                }
            }
        }

        if has_unknown {
            thread::sleep(Duration::from_secs(10));
        } else {
            return;
        }
    }
}

fn configure_git(config: &Conf) {
    git(&["config", "user.email", &config.git.email]);
    git(&["config", "user.name", &config.git.name]);
}

/// Enqueues a PR, including the tip of a multi-PR stack — with `merge.trigger = comment`,
/// stacks post `merge.comment` (typically `/trunk merge`) just like any other PR.
fn enqueue(pr: &str, config: &Conf, cli: &Cli, forge: &ForgeClient) {
    match config.merge.trigger {
        EnqueueTrigger::Comment => {
            let body = config.merge.comment.as_str();
            if body.is_empty() {
                eprintln!("Cannot enqueue PR because merge 'trigger' is set to comment but no comment was provided");
                return;
            }
            forge.comment(pr, body);
        }
        EnqueueTrigger::Label => {
            if config.merge.labels.is_empty() {
                eprintln!("Cannot enqueue PR because merge 'trigger' is set to label but no labels were provided");
                return;
            }
            let labels: Vec<&str> = config.merge.labels.split(',').map(|s| s.trim()).collect();
            for lbl in &labels {
                forge.add_label(pr, lbl);
            }
        }
        EnqueueTrigger::Run => {
            if config.merge.run.is_empty() {
                eprintln!("Cannot enqueue PR because merge 'trigger' is set to run but no run command was provided");
                return;
            }
            // perform token replacement for pr
            let cmd = config.merge.run.replace("{{PR_NUMBER}}", pr);
            println!("run commd {}", cmd);
            let result = run_cmd(&cmd);
            println!("merge run results: {}", result);
        }

        EnqueueTrigger::Api => {
            // TRUNK_TOKEN will be checked at runtime in submit_pull_request
            match trunk_repo(config) {
                Ok(repo) => {
                    let pr_number: u32 = match pr.parse() {
                        Ok(num) => num,
                        Err(_) => {
                            eprintln!("Invalid PR number: {}", pr);
                            return;
                        }
                    };
                    // Get the PR's base branch
                    let target_branch = forge.base_branch(pr);
                    println!("Enqueuing PR {} targeting branch: {}", pr, target_branch);
                    match submit_pull_request(
                        &repo,
                        pr_number,
                        &target_branch,
                        None, // Default priority, could be made configurable
                        &config.trunk.api,
                        cli,
                    ) {
                        Ok(_) => println!(
                            "Successfully submitted PR {} to Trunk merge queue (target: {})",
                            pr, target_branch
                        ),
                        Err(e) => {
                            eprintln!("Failed to submit PR {} to Trunk merge queue: {}", pr, e);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to get repository information: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}

fn simulate_test(config: &Conf) -> bool {
    let is_merge_str = env::var("IS_MERGE").unwrap_or_else(|_| String::from("false"));
    let is_merge = is_merge_str.to_lowercase() == "true";

    if !is_merge {
        println!("no flake or sleep when running on pr branch");
        return true;
    }

    let sleep = config.sleep_duration();
    println!(
        "sleeping for {:.1} seconds (sleep_for: {})",
        sleep.as_secs_f64(),
        config.test.sleep_for
    );
    thread::sleep(sleep);

    if !config.pullrequest.logical_conflict_file.is_empty()
        && Path::new(&config.pullrequest.logical_conflict_file).exists()
    {
        // this pull request is simulating a "logical merge conflict" and should always fail
        println!("Simulating logical merge conflict - failing test");
        return false;
    }

    let mut rng = rand::thread_rng();
    let random_float = rng.gen_range(0.0..1.0);

    println!("Random float: {}", random_float);
    println!("Flake rate: {}", config.test.flake_rate);

    random_float > config.test.flake_rate
}

fn maybe_add_logical_merge_conflict(last_pr: u32, config: &Conf) -> bool {
    if config.pullrequest.logical_conflict_file.is_empty()
        || config.pullrequest.logical_conflict_every == 0
    {
        return false;
    }

    // check if we should simulate a logical merge conflict with this pull request
    if (last_pr + 1) % config.pullrequest.logical_conflict_every != 0 {
        return false;
    }

    println!(
        "logical conflict every {} prs",
        config.pullrequest.logical_conflict_every
    );

    // create logical conflict
    let filename = &config.pullrequest.logical_conflict_file;
    std::fs::write(filename, "simulate logical merge conflict")
        .expect("Unable to write logical merge conflict file");

    git(&["add", &config.pullrequest.logical_conflict_file]);
    true
}

fn checkout_branch(branch: &str) -> Result<(), String> {
    // Check if branch exists locally (quietly - we expect this to fail if branch doesn't exist)
    let branch_exists_locally =
        try_git_quiet(&["rev-parse", "--verify", &format!("refs/heads/{}", branch)]).is_ok();

    // Switch to the branch
    if branch_exists_locally {
        // Branch exists locally, just checkout
        if let Err(e) = try_git(&["checkout", branch]) {
            return Err(format!(
                "Failed to checkout existing branch '{}': {}",
                branch, e
            ));
        }
    } else {
        // Branch doesn't exist locally, fetch from origin to check if it exists there
        let _ = try_git(&["fetch", "origin", branch]);

        // Check if it exists on origin (quietly - we expect this to fail if branch doesn't exist)
        let remote_branch = format!("origin/{}", branch);
        let remote_exists =
            gen::process::try_git_quiet(&["rev-parse", "--verify", &remote_branch]).is_ok();

        if remote_exists {
            // Branch exists on origin, create local tracking branch
            if let Err(e) = try_git(&["checkout", "-b", branch, &remote_branch]) {
                return Err(format!(
                    "Failed to create local branch '{}' tracking {}: {}",
                    branch, remote_branch, e
                ));
            }
        } else {
            return Err(format!(
                "Base branch '{}' does not exist locally or on origin.",
                branch
            ));
        }
    }

    Ok(())
}

/// `stack-change/{stack_id}-{position}`. Stacking stores the returned name in `current_base`.
fn head_branch_for_stack(stack_id: &str, position: usize) -> String {
    format!("stack-change/{}-{}", stack_id, position)
}

/// Random 5-char `[a-z0-9]` identifier for a stack (e.g. `a1f3c`).
fn new_stack_id() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..5)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Build and open one generated PR (new branch, edits, commit, push, open the PR/MR).
fn create_pull_request(
    filenames: &[String],
    last_pr: u32,
    config: &Conf,
    dry_run: bool,
    forge: &ForgeClient,
    base_branch: &str,
    stack_info: Option<(usize, usize)>,
    stack_parent_pr: Option<u32>,
    stack_id: &str,
    position: usize,
) -> Result<(String, usize, String), String> {
    // When stacking above the base PR, pass that parent's GitHub PR number (None at stack bottom).
    let current_branch = git(&["branch", "--show-current"]);

    // Checkout the base branch (will fetch from origin if needed).
    // For stacked PRs, base_branch is the previous PR's head branch
    // rather than a protected branch.
    checkout_branch(base_branch)?;

    // Pull latest changes
    let _ = try_git(&["pull"]);

    // Now edit the files to create changes (after we're on the correct base branch)
    let next_pr_number = last_pr + 1;
    let words = edit_files_for_pr(filenames, next_pr_number, config);
    let deps_count = words.len();

    let branch_name = head_branch_for_stack(stack_id, position);
    git(&["checkout", "-b", &branch_name]);

    // Create logical conflict file if needed (after we're on the new branch)
    let lc = maybe_add_logical_merge_conflict(last_pr, config);

    // Stage only the files that were modified (not all changes)
    for filename in filenames {
        let _ = try_git(&["add", filename]);
    }

    let commit_msg = format!("Moving words {}", words.join(", "));
    if let Err(e) = try_git(&["commit", "--no-verify", "-m", &commit_msg]) {
        return Err(format!("Failed to commit changes: {}", e));
    }

    if !dry_run {
        let result = try_git(&["push", "--set-upstream", "origin", "HEAD"]);
        if result.is_err() {
            git(&["checkout", &current_branch]);
            git(&["pull"]);
            return Err("could not push to origin".to_owned());
        }
    }

    // Solo PRs: keep merge target in the title (usually a short protected branch like main).
    // Stacked PRs: base_branch is a generated `stack-change/{id}-{position}` branch — putting that
    // in the title is noisy; stack position and the edited words are enough (full base is in the PR body).
    let words_joined = words.join(", ");
    let mut title = match stack_info {
        Some((position, depth)) => {
            format!("[stack {}/{}] {}", position, depth, words_joined)
        }
        None => format!("[{}] {}", base_branch, words_joined),
    };
    if lc {
        title = format!("{} (logical-conflict)", title);
    }

    let mut body = config.pullrequest.body.to_string();
    body.push_str("\n\n[test]\n");
    body.push_str(&format!("flake rate: {}\n", config.test.flake_rate));
    body.push_str(&format!(
        "logical conflict every: {}\n",
        config.pullrequest.logical_conflict_every
    ));
    body.push_str(&format!("sleep for: {}\n", config.test.sleep_for));
    body.push_str(&format!(
        "close stale after: {}\n",
        config.pullrequest.close_stale_after
    ));
    body.push_str(&format!(
        "\n\n[pullrequest]\nrequests per hour: {}\ntarget branch: {}\n",
        config.pullrequest.requests_per_hour, base_branch
    ));

    let mut first_letters: Vec<_> = words
        .iter()
        .map(|word| word.chars().next().unwrap().to_string())
        .collect();

    first_letters.sort();
    first_letters.dedup();

    body.push_str(&format!(
        "\n\ndeps=[{}]\n",
        first_letters.into_iter().collect::<Vec<_>>().join(",")
    ));

    if let Some((position, depth)) = stack_info {
        body.push_str("\n[stack]\n");
        body.push_str(&format!("position: {}\ndepth: {}\n", position, depth));
        if position > 1 {
            if let Some(pn) = stack_parent_pr {
                body.push_str(&format!(
                    "parent_branch: {}\nparent_pr: {}\n",
                    base_branch, pn
                ));
            }
        }
        body.push_str(&format!("based on: {}\n", base_branch));
    }

    let labels: Vec<&str> = config
        .pullrequest
        .labels
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    if dry_run {
        git(&["checkout", &current_branch]);
        git(&["pull"]);
        return Ok(((last_pr + 1).to_string(), deps_count, branch_name));
    }

    let result = forge.create_request(&branch_name, base_branch, &title, &body, &labels);

    // no matter what is result - need to reset checkout and clean up
    git(&["checkout", &current_branch]);
    // Clean up any uncommitted changes and untracked files
    let _ = try_git(&["reset", "--hard", "HEAD"]);
    let _ = try_git(&["clean", "-fd"]);
    git(&["pull"]);

    match result {
        Ok(pr_number) => Ok((pr_number, deps_count, branch_name)),
        Err(e) => Err(format!("could not create pull request: {}", e)),
    }
}

/// Register a fully-created stack with GitHub's stacks API, passing PR numbers
/// ordered bottom to top. Failures are logged but not fatal: the PRs are
/// already chained branch-on-branch, so generation continues even when the
/// stacks API is unavailable (e.g. the feature is not enabled on the repo).
fn register_stack(pr_numbers: &[u32], depth: usize, dry_run: bool, forge: &ForgeClient) {
    let ForgeClient::GitHub { token: gh_token } = forge else {
        println!(
            "GitLab has no stacks API; MRs {:?} are chained by target branch only",
            pr_numbers
        );
        return;
    };
    if dry_run {
        println!("dry-run: would register gh stack for PRs {:?}", pr_numbers);
        return;
    }
    if pr_numbers.len() != depth {
        eprintln!(
            "skipping gh stack registration: expected {} PR numbers, have {:?}",
            depth, pr_numbers
        );
        return;
    }
    match repo::origin() {
        Ok(origin) => match GitHub::create_stack(&origin.owner, &origin.name, pr_numbers, gh_token)
        {
            Ok(_) => println!("registered gh stack for PRs {:?}", pr_numbers),
            Err(e) => eprintln!(
                "failed to register gh stack for PRs {:?}: {}",
                pr_numbers, e
            ),
        },
        Err(e) => eprintln!("failed to register gh stack: {}", e),
    }
}

fn generate(config: &Conf, cli: &Cli) -> anyhow::Result<()> {
    if config.is_generator_disabled() {
        println!("generator is disabled pull requests per hour is set to 0");
        return Ok(());
    }

    configure_git(config);

    let pull_requests_to_make: usize;
    let pull_request_every: u64;

    if config.pullrequest.requests_per_run > 0 {
        pull_requests_to_make = config.pullrequest.requests_per_run as usize;
        pull_request_every = 1;

        println!(
            "will generate {} requests in burst mode",
            pull_requests_to_make
        );
    } else {
        let dur = config.run_generate_for_duration();
        let hours = dur.as_secs() as f32 / 3600.0;

        pull_requests_to_make =
            (config.pullrequest.requests_per_hour as f32 * hours).ceil() as usize;
        // assuming that generating a pr doesn't take any time we will project to sleep every
        pull_request_every = (dur.as_secs() as f32 / pull_requests_to_make as f32).ceil() as u64;

        println!(
            "will generate pull request every {} seconds",
            pull_request_every
        );
    }

    // Seed `last_pr` from the forge so the *first* PR in this run uses a realistic `last_pr + 1` for
    // `edit_files_for_pr` / logical-conflict cadence; later PRs use the number the forge returns on creation.
    let forges = forge_clients(config, cli);
    let mut last_pr = forges[0].latest_number();

    let mut prs: Vec<String> = Vec::new();

    if cli.dry_run {
        println!("dry-run set - no actual pull requests will be generated");
    }

    println!("Using {} token(s) in round-robin fashion", forges.len());

    let stack_plan = config.plan_stacks(pull_requests_to_make);
    if config.pullrequest.stacks_distribution.is_some() {
        println!(
            "stack plan ({} stacks for {} PRs): {:?}",
            stack_plan.len(),
            pull_requests_to_make,
            stack_plan
        );
    }

    let mut pr_index = 0usize;
    for (stack_index, depth) in stack_plan.iter().enumerate() {
        let protected_branches = &config.pullrequest.protected_branches;
        let protected_base: String =
            protected_branches[stack_index % protected_branches.len()].clone();

        // For a stack, subsequent PRs base on the previous PR's branch.
        let mut current_base = protected_base.clone();
        let mut stack_parent_pr_number: Option<u32> = None;
        // PR numbers created for this stack, bottom to top, for stack registration.
        let mut stack_pr_numbers: Vec<u32> = Vec::new();

        // 5-char [a-z0-9] id shared by every PR branch in this stack (~60M possibilities).
        let stack_id = new_stack_id();

        println!(
            "stack {} ({}) of {}: depth {} — first PR will target '{}'",
            stack_index + 1,
            stack_id,
            stack_plan.len(),
            depth,
            protected_base
        );

        for position in 1..=*depth {
            // Defensive: the first PR in every stack must target the protected branch for this
            // stack, never the previous stack's tip. If this ever fires, something regressed.
            if position == 1 && current_base != protected_base {
                eprintln!(
                    "correcting stack base: expected '{}', was '{}'",
                    protected_base, current_base
                );
                current_base = protected_base.clone();
            }
            let start = Instant::now();
            let files = get_txt_files(config)?;
            let mut filenames: Vec<String> = files
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();

            filenames.sort();

            // Select token for this PR (round-robin)
            let forge = &forges[pr_index % forges.len()];

            let stack_info = if *depth > 1 {
                Some((position, *depth))
            } else {
                None
            };

            let pr_result = create_pull_request(
                &filenames,
                last_pr,
                config,
                cli.dry_run,
                forge,
                &current_base,
                stack_info,
                stack_parent_pr_number,
                &stack_id,
                position,
            );
            let (pr, deps_count, head_branch) = match pr_result {
                Ok(created) => created,
                Err(e) => {
                    println!("problem created pr for files: {:?}: {}", filenames, e);
                    // Abort the rest of this stack: without this PR's branch,
                    // we can't base the next one on top of it.
                    break;
                }
            };
            let duration = start.elapsed();
            let pr_number = pr.parse::<u32>().ok();
            if let Some(n) = pr_number {
                stack_pr_numbers.push(n);
            }
            let stack_tag = if *depth > 1 {
                format!(" [stack {}/{}]", position, *depth)
            } else {
                String::new()
            };
            println!(
                "created pr: {} (target: {}, deps: {}){} in {}s // waiting: {} mins",
                pr,
                current_base,
                deps_count,
                stack_tag,
                duration.as_secs(),
                (pull_request_every as f32 / 60.0)
            );
            thread::sleep(Duration::from_secs(pull_request_every) / 2);
            // For stacks, only enqueue the top PR (position == depth). Lower
            // PRs can't merge on their own until the tip is resolved, so
            // enqueueing them just churns the queue.
            let is_top_of_stack = position == *depth;
            if is_top_of_stack {
                // Register the stack with GitHub once every PR in it exists,
                // before enqueueing the tip so the queue sees a complete stack.
                if *depth > 1 {
                    register_stack(&stack_pr_numbers, *depth, cli.dry_run, forge);
                }
                enqueue(&pr, config, cli, forge);
            } else {
                println!(
                    "skipping enqueue for pr {} (stack {}/{})",
                    pr, position, *depth
                );
            }
            thread::sleep(Duration::from_secs(pull_request_every) / 2);
            // Keep in sync with GitHub's assigned number for the next `last_pr + 1` edit sequence.
            if let Some(n) = pr_number {
                last_pr = n;
                stack_parent_pr_number = Some(n);
            } else {
                last_pr = last_pr.saturating_add(1);
                stack_parent_pr_number = None;
            }
            prs.push(pr);
            pr_index += 1;

            // Next PR in this stack bases on the branch we just pushed.
            current_base = head_branch;
        }

        // Leave git on the protected branch after each stack so the next stack always forks from
        // a known base, even if the user started generate from another local branch.
        if let Err(e) = checkout_branch(&protected_base) {
            eprintln!(
                "warning: could not check out protected branch '{}' after stack: {}",
                protected_base, e
            );
        } else {
            let _ = try_git(&["pull"]);
        }
    }

    Ok(())
}

#[cfg(test)]
mod stack_branch_tests {
    use super::{head_branch_for_stack, new_stack_id};

    #[test]
    fn branch_uses_stack_id_and_position() {
        assert_eq!(head_branch_for_stack("a1f3c", 1), "stack-change/a1f3c-1");
        assert_eq!(head_branch_for_stack("00000", 4), "stack-change/00000-4");
    }

    #[test]
    fn stack_id_is_five_lowercase_alphanumeric() {
        let id = new_stack_id();
        assert_eq!(id.len(), 5);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }
}

fn run() -> anyhow::Result<()> {
    let cli: Cli = Cli::parse();

    if cli.subcommand.is_none() {
        println!("Subcommand required. run 'mq help'");
        return Ok(());
    }

    if let Some(Subcommands::Defaultconfig {}) = &cli.subcommand {
        Conf::print_default();
        return Ok(());
    }

    let config = Conf::builder()
        .env()
        .file("mq.toml")
        .file(".config/mq.toml")
        .load()
        .unwrap_or_else(|err| handle_config_load_error(err));

    config.is_valid(Some(&cli)).unwrap_or_else(|err| {
        eprintln!("Invalid config:\n    {}", err);
        std::process::exit(1);
    });

    match &cli.subcommand {
        Some(Subcommands::Housekeeping {}) => {
            let forges = forge_clients(&config, &cli);
            housekeeping(&config, &forges[0]);
            Ok(())
        }
        Some(Subcommands::TestSim {}) => {
            if !simulate_test(&config) {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Subcommands::Config { path }) => {
            if let Some(path) = path {
                let path = path.trim();
                if path.is_empty() {
                    // If path is empty, print full config
                    let config_json =
                        to_string_pretty(&config).expect("Failed to serialize config to JSON");
                    println!("{}", config_json);
                    return Ok(());
                }

                // Extract a single value from the config
                let config_json: Value =
                    serde_json::to_value(&config).expect("Failed to serialize config to JSON");

                let parts: Vec<&str> = path.split('.').collect();
                let mut current = &config_json;

                for part in parts {
                    match current.get(part) {
                        Some(value) => {
                            current = value;
                        }
                        None => {
                            eprintln!("Config path '{}' not found", path);
                            std::process::exit(1);
                        }
                    }
                }

                // Output the value - if it's a string, output without quotes, otherwise output as JSON
                match current {
                    Value::String(s) => println!("{}", s),
                    _ => println!("{}", serde_json::to_string(current).unwrap()),
                }
            } else {
                // Print full config as JSON
                let config_json =
                    to_string_pretty(&config).expect("Failed to serialize config to JSON");
                println!("{}", config_json);
            }
            Ok(())
        }
        Some(Subcommands::Generate {}) => generate(&config, &cli),
        Some(Subcommands::UploadTargets(ut)) => {
            // upload_targets(&cli, &gen::pullrequest::get_json()); // &ut.github_json);
            upload_targets(&config, &cli, ut.github_json.as_deref());
            Ok(())
        }
        Some(Subcommands::Enqueue(enqueue_args)) => {
            println!("Enqueuing PR: {}", enqueue_args.pr);
            let forges = forge_clients(&config, &cli);
            enqueue(&enqueue_args.pr, &config, &cli, &forges[0]);
            Ok(())
        }
        _ => {
            // Handle other cases here
            Err(anyhow::anyhow!("Subcommand not supported"))
        }
    }
}

fn main() {
    env_logger::init();

    match run() {
        Ok(_) => (),
        Err(err) => {
            log::error!("{}", err);
            std::process::exit(1);
        }
    }
}
