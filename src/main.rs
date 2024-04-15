use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, thread};

use chrono::{DateTime, Utc};
use clap::Parser;
use confique::Config;
use gen::cli::{Cli, Subcommands};
use gen::config::Conf;
use gen::edit::change_file;
use gen::github::GitHub;
use gen::process::{gh, git, try_gh, try_git};
use gen::trunk::upload_targets;
use rand::Rng;
use regex::Regex;
use serde_json::{to_string_pretty, Value};
use walkdir::WalkDir;

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

fn housekeeping(config: &Conf) {
    for _ in 0..3 {
        let json_str = gh(&[
            "pr",
            "list",
            "--limit=1000",
            "--json",
            "number,mergeable,comments",
        ]);
        let v: Value = serde_json::from_str(&json_str).expect("Failed to parse JSON");

        let mut has_unknown = false;
        let mut requeued: HashSet<String> = HashSet::new();
        if let Some(array) = v.as_array() {
            for item in array {
                let mergeable = item["mergeable"].as_str().unwrap_or("");
                let pr = item["number"].as_i64().unwrap_or(0).to_string();
                match mergeable {
                    "UNKNOWN" => {
                        has_unknown = true;
                    }
                    "CONFLICTING" => {
                        GitHub::close(&pr);
                        println!("closed pr: {} (had merge conflicts)", &pr);
                    }
                    "MERGEABLE" => {
                        if requeued.contains(&pr) {
                            continue;
                        }
                        let comments = item["comments"].as_array().unwrap();
                        'comment: for comment in comments.iter() {
                            let body = comment["body"].as_str().unwrap_or("");
                            let created_at_str = comment["createdAt"].as_str().unwrap_or("");

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
                                //enqueue(&pr, config);
                                GitHub::close(&pr);
                                println!("closed stale pr: {}", &pr);
                                requeued.insert(pr.to_owned());
                            }
                        }
                    }
                    _ => {
                        // handle other states
                    }
                }
            }

            if has_unknown {
                thread::sleep(Duration::from_secs(10));
            } else {
                return;
            }
        } else {
            return;
        }
    }
}

fn configure_git(config: &Conf) {
    git(&["config", "user.email", &config.git.email]);
    git(&["config", "user.name", &config.git.name]);
}

fn enqueue(pr: &str, config: &Conf) {
    if !config.merge.comment.is_empty() {
        GitHub::comment(pr, &config.merge.comment);
    }
    if !config.merge.labels.is_empty() {
        let labels: Vec<&str> = config.merge.labels.split(',').map(|s| s.trim()).collect();
        for lbl in &labels {
            GitHub::add_label(pr, lbl);
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

    println!("sleeping for {} seconds", config.sleep_duration().as_secs());
    thread::sleep(config.sleep_duration());

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

fn get_last_pr() -> u32 {
    let result = try_gh(&["pr", "list", "--limit=1", "--json", "number"]);
    if result.is_err() {
        return 0;
    }
    let json_str = result.unwrap();

    let v: Value = serde_json::from_str(&json_str).expect("Failed to parse JSON");

    let array = v.as_array();
    match array {
        Some(pr_array) => {
            if pr_array.is_empty() {
                // No PRs in the system. return 0
                return 0;
            }
            let first = pr_array.first().cloned();
            match first {
                Some(pr) => pr["number"].as_u64().unwrap_or(0) as u32,
                None => 0,
            }
        }
        None => 0,
    }
}

fn create_pull_request(
    words: &[String],
    last_pr: u32,
    config: &Conf,
    dry_run: bool,
) -> Result<String, String> {
    let lc = maybe_add_logical_merge_conflict(last_pr, config);

    let current_branch = git(&["branch", "--show-current"]);

    let branch_name = format!("change/{}", words.join("-"));
    git(&["checkout", "-t", "-b", &branch_name]);

    let commit_msg = format!("Moving words {}", words.join(", "));
    git(&["commit", "--no-verify", "-am", &commit_msg]);

    if !dry_run {
        let result = try_git(&["push", "--set-upstream", "origin", "HEAD"]);
        if result.is_err() {
            git(&["checkout", &current_branch]);
            git(&["pull"]);
            return Err("could not push to origin".to_owned());
        }
    }

    let mut title = words.join(", ");
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
    body.push_str(&format!(
        "sleep for: {}s\n",
        config.sleep_duration().as_secs()
    ));
    body.push_str(&format!(
        "close stale after: {}\n",
        config.pullrequest.close_stale_after
    ));
    body.push_str(&format!(
        "\n\n[pullrequest]\nrequests per hour: {}\n",
        config.pullrequest.requests_per_hour
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

    let mut args: Vec<&str> = vec!["pr", "create", "--title", &title, "--body", &body];

    for lbl in config.pullrequest.labels.split(',') {
        args.push("--label");
        args.push(lbl.trim());
    }

    if dry_run {
        git(&["checkout", &current_branch]);
        git(&["pull"]);
        return Ok((last_pr + 1).to_string());
    }

    let result = try_gh(args.as_slice());

    // no matter what is result - need to reset checkout
    git(&["checkout", &current_branch]);
    git(&["pull"]);

    if result.is_err() {
        return Err("could not create pull request".to_owned());
    }

    let pr_url = result.unwrap();
    let re = Regex::new(r"(.*)/pull/(\d+)$").unwrap();
    let caps = re.captures(pr_url.trim()).unwrap();
    let pr_number = caps.get(2).map_or("", |m| m.as_str());

    Ok(pr_number.to_string())
}

fn generate(config: &Conf, cli: &Cli) -> anyhow::Result<()> {
    if config.pullrequest.requests_per_hour == 0 {
        println!("generator is disabled pull requests per hour is set to 0");
        return Ok(());
    }

    configure_git(&config);

    let dur = config.run_generate_for_duration();
    let hours = dur.as_secs() as f32 / 3600.0;

    let pull_requests_to_make =
        (config.pullrequest.requests_per_hour as f32 * hours).ceil() as usize;
    // assuming that generating a pr doesn't take any time we will project to sleep every
    let pull_request_every = (dur.as_secs() as f32 / pull_requests_to_make as f32).ceil() as u64;

    println!(
        "will generate pull request every {} seconds",
        pull_request_every
    );

    // get the most recent PR to be created (used for creating logical merge conflicts)
    let mut last_pr = get_last_pr();

    let mut prs: Vec<String> = Vec::new();

    if cli.dry_run {
        println!("dry-run set - no actual pull requests will be generated");
    }

    for _ in 0..pull_requests_to_make {
        let start = Instant::now();
        let files = get_txt_files(&config)?;
        let mut filenames: Vec<String> = files
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();

        filenames.sort();
        let filenames: Vec<String> = filenames
            .into_iter()
            .take(config.pullrequest.max_deps)
            .collect();

        let max_impacted_deps = config.pullrequest.max_impacted_deps as u32; // Convert usize to u32
        let words = change_file(&filenames, max_impacted_deps); // Use the converted value

        let pr_result = create_pull_request(&words, last_pr, &config, cli.dry_run);
        if pr_result.is_err() {
            println!("problem created pr for {:?}", words);
            continue;
        }
        let duration = start.elapsed();
        let pr = pr_result.unwrap();
        println!(
            "created pr: {} in {:?} // waiting: {} mins",
            pr,
            duration,
            (pull_request_every as f32 / 60.0)
        );
        thread::sleep(Duration::from_secs(pull_request_every) / 2);
        enqueue(&pr, config); // Change the argument type to accept a String
        thread::sleep(Duration::from_secs(pull_request_every) / 2);
        prs.push(pr);
        last_pr += 1;
    }

    Ok(())
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
        .unwrap_or_else(|err| {
            eprintln!("Generator cannot run: {}", err);
            std::process::exit(1);
        });

    match &cli.subcommand {
        Some(Subcommands::Housekeeping {}) => {
            housekeeping(&config);
            Ok(())
        }
        Some(Subcommands::TestSim {}) => {
            if !simulate_test(&config) {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Subcommands::Config {}) => {
            let config_json =
                to_string_pretty(&config).expect("Failed to serialize config to JSON");
            println!("{}", config_json);
            Ok(())
        }
        Some(Subcommands::Generate {}) => generate(&config, &cli),
        Some(Subcommands::UploadTargets(ut)) => {
            // upload_targets(&cli, &gen::pullrequest::get_json()); // &ut.github_json);
            upload_targets(&config, &cli, &ut.github_json);
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
