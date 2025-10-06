use crate::cli::Cli;
use crate::config::Conf;
use crate::github::GitHubAction;
use regex::Regex;
use reqwest::header::{HeaderMap, CONTENT_TYPE};
use serde_json::json;
use std::fs;

pub fn upload_targets(config: &Conf, cli: &Cli, github_json_path: &str) {
    let github_json = fs::read_to_string(github_json_path).expect("Failed to read file");
    let ga = GitHubAction::from_json(&github_json);

    if !&ga.event.pull_request.body.is_some() {
        // no body content to pull deps from
        return;
    }

    let re = Regex::new(r".*deps=\[(.*?)\].*").unwrap();
    let body = ga.event.pull_request.body.clone().unwrap();
    let mut impacted_targets: Vec<String> = Vec::new();
    if let Some(caps) = re.captures(&body) {
        impacted_targets = caps[1]
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect::<Vec<String>>();
    } else {
        println!("No deps listed in PR body like deps=[a,b,c]");
    }

    // Validate that we have targets to upload
    if impacted_targets.is_empty() {
        println!("No impacted targets found - skipping upload");
        return;
    }

    println!(
        "Uploading {} impacted targets: {:?}",
        impacted_targets.len(),
        impacted_targets
    );

    let result = post_targets(
        ga.repo_owner(),
        ga.repo_name(),
        ga.event.pull_request.number,
        &ga.event.pull_request.head.sha,
        "main",
        impacted_targets,
        &config.trunk.api,
        &cli.trunk_token,
    );

    match result {
        Ok(_) => println!("Successfully uploaded impacted targets"),
        Err(e) => {
            eprintln!("Failed to upload impacted targets: {:?}", e);
            // Exit with error code to fail the workflow
            std::process::exit(1);
        }
    }
}

pub fn post_targets(
    repo_owner: &str,
    repo_name: &str,
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

    let body = json!({
        "repo": {
            "host": "github.com",
            "owner": repo_owner,
            "name": repo_name,
        },
        "pr": {
            "number": pr_number,
            "sha": pr_sha,
        },
        "targetBranch": target_branch,
        "impactedTargets": impacted_targets,
    });

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
        println!("  Repository: {}/{}", repo_owner, repo_name);
        println!("  PR Number: {}", pr_number);
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
    repo_owner: &str,
    repo_name: &str,
    pr_number: u32,
    target_branch: &str,
    priority: Option<&str>,
    api: &str,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error>> {
    // Handle dry-run mode
    if cli.dry_run {
        println!("dry-run: would submit to Trunk API:");
        println!("  - Repository: {}/{}", repo_owner, repo_name);
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
            "host": "github.com",
            "owner": repo_owner,
            "name": repo_name,
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
        println!("  Repository: {}/{}", repo_owner, repo_name);
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
