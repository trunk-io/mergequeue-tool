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
        Ok(()) => println!("Response: {:?}", "fe"),
        Err(e) => eprintln!("Error: {:?}", e),
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
) -> Result<(), reqwest::Error> {
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
        .send();

    println!("Response: {:?}", res);

    Ok(())
}
