use crate::github::GitHub;
use crate::gitlab::{self, GitLab};
use crate::process::try_gh;
use regex::Regex;
use serde_json::Value;

/// Whether a pull/merge request can merge cleanly into its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeability {
    /// The forge has not finished computing it yet.
    Unknown,
    Conflicting,
    Mergeable,
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub body: String,
    /// RFC 3339 timestamp.
    pub created_at: String,
}

#[derive(Debug)]
pub struct OpenRequest {
    pub number: String,
    pub mergeability: Mergeability,
    /// Only populated for mergeable requests; housekeeping reads comments to find
    /// stale ones and closes conflicting ones regardless.
    pub comments: Vec<Comment>,
}

/// A GitHub pull request or GitLab merge request host, authenticated with one token.
/// Request numbers are GitHub PR numbers or GitLab MR iids.
pub enum ForgeClient {
    GitHub { token: String },
    GitLab(GitLab),
}

impl ForgeClient {
    pub fn comment(&self, number: &str, body: &str) {
        match self {
            ForgeClient::GitHub { token } => {
                GitHub::comment(number, body, token);
            }
            ForgeClient::GitLab(gl) => gl
                .comment(number, body)
                .expect("Failed to comment on merge request"),
        }
    }

    pub fn close(&self, number: &str) {
        match self {
            ForgeClient::GitHub { token } => {
                GitHub::close(number, token);
            }
            ForgeClient::GitLab(gl) => gl.close(number).expect("Failed to close merge request"),
        }
    }

    pub fn add_label(&self, number: &str, label: &str) {
        match self {
            ForgeClient::GitHub { token } => {
                GitHub::add_label(number, label, token);
            }
            ForgeClient::GitLab(gl) => gl
                .add_label(number, label)
                .expect("Failed to add label to merge request"),
        }
    }

    /// The branch the request merges into, falling back to `main` if it cannot be read.
    pub fn base_branch(&self, number: &str) -> String {
        match self {
            ForgeClient::GitHub { token } => GitHub::get_pr_base_branch(number, token),
            ForgeClient::GitLab(gl) => match gl.merge_request(number) {
                Ok(mr) => mr["target_branch"].as_str().unwrap_or("main").to_string(),
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to get target branch for MR {}: {}. Falling back to 'main'",
                        number, e
                    );
                    "main".to_string()
                }
            },
        }
    }

    /// Opens a request from `head` into `base` and returns its number.
    pub fn create_request(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
        labels: &[&str],
    ) -> Result<String, String> {
        match self {
            ForgeClient::GitHub { token } => {
                let mut args: Vec<&str> = vec![
                    "pr", "create", "--title", title, "--body", body, "--base", base,
                ];
                for lbl in labels {
                    args.push("--label");
                    args.push(lbl);
                }
                let pr_url = try_gh(&args, token)?;
                let re = Regex::new(r"(.*)/pull/(\d+)$").unwrap();
                re.captures(pr_url.trim())
                    .and_then(|caps| caps.get(2))
                    .map(|m| m.as_str().to_string())
                    .ok_or_else(|| format!("unexpected output from gh pr create: {}", pr_url))
            }
            ForgeClient::GitLab(gl) => gl
                .create_merge_request(head, base, title, body, labels)
                .map(|iid| iid.to_string()),
        }
    }

    /// The highest request number in the repository, or 0 if it cannot be read.
    pub fn latest_number(&self) -> u32 {
        match self {
            // Max over a page of PRs because `--limit=1` alone is an arbitrary row
            // (often the most recently *updated*), not the largest number.
            ForgeClient::GitHub { token } => try_gh(
                &[
                    "pr", "list", "--state", "all", "-L", "5000", "--json", "number",
                ],
                token,
            )
            .ok()
            .and_then(|json_str| serde_json::from_str::<Value>(&json_str).ok())
            .and_then(|v| {
                v.as_array()?
                    .iter()
                    .filter_map(|pr| pr["number"].as_u64())
                    .max()
            })
            .unwrap_or(0) as u32,
            ForgeClient::GitLab(gl) => gl.latest_iid().unwrap_or_else(|e| {
                eprintln!("Warning: Failed to read latest MR iid: {}", e);
                0
            }),
        }
    }

    pub fn open_requests(&self) -> Result<Vec<OpenRequest>, String> {
        match self {
            ForgeClient::GitHub { token } => {
                let json_str = try_gh(
                    &[
                        "pr",
                        "list",
                        "--limit=1000",
                        "--json",
                        "number,mergeable,comments",
                    ],
                    token,
                )?;
                let v: Value = serde_json::from_str(&json_str).map_err(|e| e.to_string())?;
                Ok(v.as_array()
                    .map(|prs| prs.iter().map(github_open_request).collect())
                    .unwrap_or_default())
            }
            ForgeClient::GitLab(gl) => {
                let mut requests = Vec::new();
                for mr in gl.open_merge_requests()? {
                    // Trunk's own draft test MRs come and go with each test run.
                    let source = mr["source_branch"].as_str().unwrap_or("");
                    if source.starts_with("trunk-merge/") {
                        continue;
                    }
                    let number = mr["iid"].as_u64().unwrap_or(0).to_string();
                    let mergeability = gitlab::mergeability(&mr);
                    let comments = if mergeability == Mergeability::Mergeable {
                        gl.notes(&number)?
                    } else {
                        Vec::new()
                    };
                    requests.push(OpenRequest {
                        number,
                        mergeability,
                        comments,
                    });
                }
                Ok(requests)
            }
        }
    }
}

fn github_open_request(pr: &Value) -> OpenRequest {
    let mergeability = match pr["mergeable"].as_str().unwrap_or("") {
        "MERGEABLE" => Mergeability::Mergeable,
        "CONFLICTING" => Mergeability::Conflicting,
        _ => Mergeability::Unknown,
    };
    let comments = pr["comments"]
        .as_array()
        .map(|cs| {
            cs.iter()
                .map(|c| Comment {
                    body: c["body"].as_str().unwrap_or("").to_string(),
                    created_at: c["createdAt"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    OpenRequest {
        number: pr["number"].as_i64().unwrap_or(0).to_string(),
        mergeability,
        comments,
    }
}
