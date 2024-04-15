use crate::process::gh;
use serde::{Deserialize, Serialize};

pub struct GitHub;

impl GitHub {
    pub fn comment(pr: &str, body: &str) -> String {
        gh(&["pr", "comment", pr, "--body", body])
    }

    pub fn close(pr: &str) -> String {
        gh(&["pr", "close", pr])
    }

    pub fn add_label(pr: &str, label: &str) -> String {
        gh(&["pr", "edit", pr, "--add-label", label])
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GitHubAction {
    repository: String,
    pub event: Event,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct Event {
    pub pull_request: PullRequest,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PullRequest {
    pub number: u32,
    pub head: Head,
    pub body: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Head {
    pub sha: String,
}

impl GitHubAction {
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap()
    }

    pub fn repo_owner(&self) -> &str {
        let repo_parts: Vec<&str> = self.repository.split('/').collect();
        repo_parts.get(0).expect("Invalid REPOSITORY format")
    }
    pub fn repo_name(&self) -> &str {
        let repo_parts: Vec<&str> = self.repository.split('/').collect();
        repo_parts.get(1).expect("Invalid REPOSITORY format")
    }
}
