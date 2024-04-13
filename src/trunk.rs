use reqwest::header::{HeaderMap, CONTENT_TYPE};
use serde_json::json;

async fn upload_targets(
    repo_owner: &str,
    repo_name: &str,
    pr_number: u32,
    pr_sha: &str,
    target_branch: &str,
    impacted_targets: Vec<String>,
    api_token: &str,
) -> Result<(), reqwest::Error> {
    let client = reqwest::Client::new();

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
        "targetBranch": "main",
        "impactedTargets": impacted_targets,
    });

    let res = client
        .post("https://api.trunk.io:443/v1/setImpactedTargets")
        .headers(headers)
        .body(body.to_string())
        .send()
        .await;

    println!("Response: {:?}", res);

    Ok(())
}
