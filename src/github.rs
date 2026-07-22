use crate::process::try_gh;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct GitHub;

impl GitHub {
    pub fn comment(pr: &str, body: &str, token: &str) -> String {
        try_gh(&["pr", "comment", pr, "--body", body], token).expect("Failed to comment on PR")
    }

    pub fn close(pr: &str, token: &str) -> String {
        try_gh(&["pr", "close", pr], token).expect("Failed to close PR")
    }

    pub fn add_label(pr: &str, label: &str, token: &str) -> String {
        try_gh(&["pr", "edit", pr, "--add-label", label], token).expect("Failed to add label to PR")
    }

    /// Build the `gh api` argument list for creating a stack from PR numbers
    /// ordered bottom to top. `-F` sends each value as a JSON integer and the
    /// `[]` suffix appends it to the `pull_requests` array in the request body.
    pub fn stack_create_args(owner: &str, repo: &str, pr_numbers: &[u32]) -> Vec<String> {
        let mut args = vec![
            "api".to_string(),
            "--method".to_string(),
            "POST".to_string(),
            format!("repos/{}/{}/stacks", owner, repo),
        ];
        for pr in pr_numbers {
            args.push("-F".to_string());
            args.push(format!("pull_requests[]={}", pr));
        }
        args
    }

    /// Register a chain of pull requests as a stack via GitHub's stacks API
    /// (`POST /repos/{owner}/{repo}/stacks`). `pr_numbers` must be ordered
    /// bottom to top, and each PR's base ref must be the previous PR's head
    /// ref. The API accepts between 2 and 100 pull requests per stack.
    pub fn create_stack(
        owner: &str,
        repo: &str,
        pr_numbers: &[u32],
        token: &str,
    ) -> Result<String, String> {
        if pr_numbers.len() < 2 || pr_numbers.len() > 100 {
            return Err(format!(
                "stacks require between 2 and 100 pull requests, got {}",
                pr_numbers.len()
            ));
        }
        let args = Self::stack_create_args(owner, repo, pr_numbers);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        try_gh(&arg_refs, token)
    }

    pub fn get_pr_base_branch(pr: &str, gh_token: &str) -> String {
        let result = try_gh(&["pr", "view", pr, "--json", "baseRefName"], gh_token);
        if result.is_err() {
            // Log the error and fallback to "main" if we can't get the PR info
            eprintln!(
                "Warning: Failed to get base branch for PR {}: {:?}. Falling back to 'main'",
                pr,
                result.as_ref().err()
            );
            return "main".to_string();
        }
        let json_str = result.unwrap();
        let v: Value = match serde_json::from_str(&json_str) {
            Ok(val) => val,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse PR info JSON for PR {}: {}. Falling back to 'main'",
                    pr, e
                );
                return "main".to_string();
            }
        };
        v["baseRefName"]
            .as_str()
            .unwrap_or_else(|| {
                eprintln!(
                    "Warning: PR {} JSON does not contain 'baseRefName' field. Falling back to 'main'",
                    pr
                );
                "main"
            })
            .to_string()
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GitHubAction {
    repository: String,
    #[serde(rename = "base_ref")]
    pub base_ref: Option<String>,
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
        repo_parts.first().expect("Invalid REPOSITORY format")
    }
    pub fn repo_name(&self) -> &str {
        let repo_parts: Vec<&str> = self.repository.split('/').collect();
        repo_parts.get(1).expect("Invalid REPOSITORY format")
    }

    pub fn base_branch(&self) -> &str {
        self.base_ref.as_deref().unwrap_or("main")
    }
}
