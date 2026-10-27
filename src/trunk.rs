use crate::cli::Cli;
use crate::config::{Conf, Forge};
use crate::github::{GitHub, GitHubAction};
use crate::gitlab::GitLab;
use crate::repo::RepoRef;
use regex::Regex;
use reqwest::header::{HeaderMap, CONTENT_TYPE};
use serde_json::json;
use std::fs;

/// Extracts dependency targets from a PR body string.
/// Looks for the pattern `deps=[target1,target2,target3]` and returns a vector of targets.
/// Returns an empty vector if no deps pattern is found.
pub fn get_targets(pr_body: &str) -> Vec<String> {
    let re = Regex::new(r".*deps=\[(.*?)\].*").unwrap();

    if let Some(caps) = re.captures(pr_body) {
        caps[1]
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect::<Vec<String>>()
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_targets_basic() {
        let body = "This is a test PR\ndeps=[a,b]\nMore content";
        let targets = get_targets(body);
        assert_eq!(targets, vec!["a", "b"]);
    }

    #[test]
    fn test_get_targets_with_spaces() {
        let body = "deps=[ a , b , c ]";
        let targets = get_targets(body);
        assert_eq!(targets, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_get_targets_single() {
        let body = "deps=[single-target]";
        let targets = get_targets(body);
        assert_eq!(targets, vec!["single-target"]);
    }

    #[test]
    fn test_get_targets_empty() {
        let body = "deps=[]";
        let targets = get_targets(body);
        assert_eq!(targets, vec![""]);
    }

    #[test]
    fn test_get_targets_no_match() {
        let body = "This PR has no deps information";
        let targets = get_targets(body);
        assert_eq!(targets, Vec::<String>::new());
    }
}

pub fn upload_targets(config: &Conf, cli: &Cli, github_json_path: Option<&str>) {
    // Check for TRUNK_TOKEN at runtime
    if cli.trunk_token.is_empty() {
        eprintln!("TRUNK_TOKEN is required for upload-targets subcommand");
        eprintln!("Provide it via --trunk-token flag or TRUNK_TOKEN environment variable");
        std::process::exit(1);
    }

    match (config.forge, github_json_path) {
        (Forge::GitLab, _) => upload_targets_gitlab(config, cli),
        (Forge::GitHub, Some(path)) => upload_targets_github(config, cli, path),
        (Forge::GitHub, None) => {
            eprintln!("--github-json is required for upload-targets on GitHub");
            std::process::exit(1);
        }
    }
}

/// The parts of a GitLab merge request pipeline that `upload-targets` needs.
#[derive(Debug, PartialEq)]
pub struct GitLabMergeRequestEnv {
    pub project_path: String,
    pub iid: u32,
    pub sha: String,
    pub target_branch: String,
    /// `None` when GitLab did not provide the description or truncated it.
    pub description: Option<String>,
}

impl GitLabMergeRequestEnv {
    /// Read GitLab's predefined CI variables through `var`.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let required = |name: &str| {
            var(name)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("{} is not set; run this in a merge request pipeline", name))
        };
        let iid = required("CI_MERGE_REQUEST_IID")?;
        // In merged-results pipelines CI_COMMIT_SHA is the merge commit, not the MR head.
        let sha = var("CI_MERGE_REQUEST_SOURCE_BRANCH_SHA")
            .filter(|v| !v.is_empty())
            .map_or_else(|| required("CI_COMMIT_SHA"), Ok)?;
        let truncated = var("CI_MERGE_REQUEST_DESCRIPTION_IS_TRUNCATED").as_deref() == Some("true");
        Ok(GitLabMergeRequestEnv {
            project_path: required("CI_PROJECT_PATH")?,
            iid: iid
                .parse()
                .map_err(|_| format!("CI_MERGE_REQUEST_IID is not a number: {}", iid))?,
            sha,
            target_branch: required("CI_MERGE_REQUEST_TARGET_BRANCH_NAME")?,
            description: var("CI_MERGE_REQUEST_DESCRIPTION").filter(|_| !truncated),
        })
    }
}

fn upload_targets_gitlab(config: &Conf, cli: &Cli) {
    let mr =
        GitLabMergeRequestEnv::from_vars(|name| std::env::var(name).ok()).unwrap_or_else(|e| {
            eprintln!("{}", e);
            std::process::exit(1);
        });

    let description = match mr.description.clone() {
        Some(d) => d,
        None => {
            let Some(token) = cli.get_gitlab_tokens().into_iter().next() else {
                eprintln!("CI_MERGE_REQUEST_DESCRIPTION is missing or truncated; provide --gitlab-token or GITLAB_TOKEN to read it from the API");
                std::process::exit(1);
            };
            let gl = GitLab::new(&config.gitlab.url, &mr.project_path, &token);
            let fetched = gl
                .merge_request(&mr.iid.to_string())
                .expect("Failed to read merge request");
            fetched["description"].as_str().unwrap_or("").to_string()
        }
    };

    let impacted_targets = get_targets(&description);
    if impacted_targets.is_empty() {
        println!("No deps listed in MR description like deps=[a,b,c] - skipping upload");
        return;
    }

    let Some(repo_id) = cli.trunk_repo_id() else {
        eprintln!("TRUNK_REPO_ID is required for upload-targets on GitLab");
        eprintln!("Provide the Trunk repository ID via --trunk-repo-id flag or TRUNK_REPO_ID environment variable");
        std::process::exit(1);
    };
    println!(
        "Uploading {} impacted targets: {:?} (target branch: {})",
        impacted_targets.len(),
        impacted_targets,
        mr.target_branch
    );
    report_upload(post_targets(
        &TrunkRepo::Id(repo_id),
        mr.iid,
        &mr.sha,
        &mr.target_branch,
        impacted_targets,
        &config.trunk.api,
        &cli.trunk_token,
    ));
}

fn report_upload(result: Result<(), Box<dyn std::error::Error>>) {
    match result {
        Ok(_) => println!("Successfully uploaded impacted targets"),
        Err(e) => {
            eprintln!("Failed to upload impacted targets: {:?}", e);
            // Exit with error code to fail the workflow
            std::process::exit(1);
        }
    }
}

fn upload_targets_github(config: &Conf, cli: &Cli, github_json_path: &str) {
    let github_json = fs::read_to_string(github_json_path).expect("Failed to read file");
    let ga = GitHubAction::from_json(&github_json);

    // Extract targets from PR body
    if !&ga.event.pull_request.body.is_some() {
        println!("No PR body content found - skipping target upload");
        println!("The PR body is required to extract dependency information using the format: deps=[target1,target2,target3]");
        return;
    }

    let body = ga.event.pull_request.body.clone().unwrap();
    let impacted_targets = get_targets(&body);

    if impacted_targets.is_empty() {
        println!("No deps listed in PR body like deps=[a,b,c]");
    }

    // Validate that we have targets to upload
    if impacted_targets.is_empty() {
        println!("No impacted targets found - skipping upload");
        return;
    }

    // Get the PR's base branch using GitHub API
    let github_tokens = cli.get_github_tokens();
    let gh_token = github_tokens.first().map(|t| t.as_str()).unwrap_or("");
    let pr_number_str = ga.event.pull_request.number.to_string();
    let target_branch = if !gh_token.is_empty() {
        GitHub::get_pr_base_branch(&pr_number_str, gh_token)
    } else {
        // Fallback to base_ref from JSON if no token available
        ga.base_branch().to_string()
    };
    println!(
        "Uploading {} impacted targets: {:?} (target branch: {})",
        impacted_targets.len(),
        impacted_targets,
        target_branch
    );

    let repo = match cli.trunk_repo_id() {
        Some(id) => TrunkRepo::Id(id),
        None => TrunkRepo::Ref(RepoRef {
            host: "github.com".to_string(),
            owner: ga.repo_owner().to_string(),
            name: ga.repo_name().to_string(),
        }),
    };
    report_upload(post_targets(
        &repo,
        ga.event.pull_request.number,
        &ga.event.pull_request.head.sha,
        &target_branch,
        impacted_targets,
        &config.trunk.api,
        &cli.trunk_token,
    ));
}

/// How a `setImpactedTargets` request names the repository. Trunk resolves a
/// host/owner/name triple only to a GitHub repo, so a GitLab project goes by its Trunk ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrunkRepo {
    Ref(RepoRef),
    Id(String),
}

impl std::fmt::Display for TrunkRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrunkRepo::Ref(repo) => write!(f, "{}/{}", repo.host, repo.path()),
            TrunkRepo::Id(id) => write!(f, "id {}", id),
        }
    }
}

pub fn set_impacted_targets_body(
    repo: &TrunkRepo,
    pr_number: u32,
    pr_sha: &str,
    target_branch: &str,
    impacted_targets: &[String],
) -> serde_json::Value {
    let mut body = json!({
        "pr": {
            "number": pr_number,
            "sha": pr_sha,
        },
        "targetBranch": target_branch,
        "impactedTargets": impacted_targets,
    });
    match repo {
        TrunkRepo::Ref(repo) => {
            body["repo"] = json!({
                "host": repo.host,
                "owner": repo.owner,
                "name": repo.name,
            })
        }
        TrunkRepo::Id(id) => body["repoId"] = json!(id),
    }
    body
}

pub fn post_targets(
    repo: &TrunkRepo,
    pr_number: u32,
    pr_sha: &str,
    target_branch: &str,
    impacted_targets: Vec<String>,
    api: &str,
    api_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::new();

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    headers.insert("x-api-token", api_token.parse().unwrap());

    let body = set_impacted_targets_body(repo, pr_number, pr_sha, target_branch, &impacted_targets);

    let res = client
        .post(&format!("https://{}:443/v1/setImpactedTargets", api))
        .headers(headers)
        .body(body.to_string())
        .send()?;

    // Check HTTP status code
    if !res.status().is_success() {
        let status = res.status();
        let error_body = res
            .text()
            .unwrap_or_else(|_| "Unable to read error response".to_string());

        // Show debug info on errors
        println!("API request failed:");
        println!("  URL: https://{}:443/v1/setImpactedTargets", api);
        println!("  Repository: {}", repo);
        println!("  PR Number: {}", pr_number);
        println!("  Target Branch: {}", target_branch);
        println!("  Impacted Targets: {:?}", impacted_targets);

        match status.as_u16() {
            400 => {
                return Err(format!(
                    "Bad Request (400): {}. Check request format and parameters.",
                    error_body
                )
                .into())
            }
            401 => {
                return Err(format!("API key rejected (401 Unauthorized): {}", error_body).into())
            }
            403 => return Err(format!("API key forbidden (403 Forbidden): {}", error_body).into()),
            404 => {
                return Err(
                    format!("Pull request not found (404 Not Found): {}", error_body).into(),
                )
            }
            429 => {
                return Err(format!("Rate limited (429 Too Many Requests): {}", error_body).into())
            }
            _ => return Err(format!("HTTP error {}: {}", status, error_body).into()),
        }
    }

    Ok(())
}

pub fn submit_pull_request(
    repo: &RepoRef,
    pr_number: u32,
    target_branch: &str,
    priority: Option<&str>,
    api: &str,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check for TRUNK_TOKEN at runtime
    if cli.trunk_token.is_empty() {
        return Err("TRUNK_TOKEN is required when using API trigger. Provide it via --trunk-token flag or TRUNK_TOKEN environment variable".into());
    }

    // Handle dry-run mode
    if cli.dry_run {
        println!("dry-run: would submit to Trunk API:");
        println!("  - Repository: {}/{}", repo.host, repo.path());
        println!("  - PR Number: {}", pr_number);
        println!("  - Target Branch: {}", target_branch);
        if let Some(priority_value) = priority {
            println!("  - Priority: {}", priority_value);
        }
        println!("  - API Endpoint: {}", api);
        println!(
            "  - Token: {}...",
            &cli.trunk_token[..std::cmp::min(8, cli.trunk_token.len())]
        );
        return Ok(());
    }
    let client = reqwest::blocking::Client::new();

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    headers.insert("x-api-token", cli.trunk_token.parse().unwrap());

    let mut body = json!({
        "repo": {
            "host": repo.host,
            "owner": repo.owner,
            "name": repo.name,
        },
        "pr": {
            "number": pr_number,
        },
        "targetBranch": target_branch,
    });

    // Add priority - use provided value or default to "medium"
    let priority_value = priority.unwrap_or("medium");
    body["priority"] = json!(priority_value);

    let url = format!("https://{}/v1/submitPullRequest", api);
    let body_str = body.to_string();

    let res = client
        .post(&url)
        .headers(headers)
        .body(body_str.clone())
        .send()?;

    // Check HTTP status code
    if !res.status().is_success() {
        let status = res.status();
        let error_body = res
            .text()
            .unwrap_or_else(|_| "Unable to read error response".to_string());

        // Show debug info only on errors
        println!("API request failed:");
        println!("  URL: {}", url);
        println!("  Request body: {}", body_str);
        println!("  Repository: {}/{}", repo.host, repo.path());
        println!("  PR Number: {}", pr_number);
        println!("  Target Branch: {}", target_branch);

        match status.as_u16() {
            400 => {
                return Err(format!(
                    "Bad Request (400): {}. Check request format and parameters.",
                    error_body
                )
                .into())
            }
            401 => {
                return Err(format!("API key rejected (401 Unauthorized): {}", error_body).into())
            }
            403 => return Err(format!("API key forbidden (403 Forbidden): {}", error_body).into()),
            404 => {
                return Err(
                    format!("Pull request not found (404 Not Found): {}", error_body).into(),
                )
            }
            429 => {
                return Err(format!("Rate limited (429 Too Many Requests): {}", error_body).into())
            }
            _ => return Err(format!("HTTP error {}: {}", status, error_body).into()),
        }
    }

    Ok(())
}
