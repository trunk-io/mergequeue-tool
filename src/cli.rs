use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[clap(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: Option<Subcommands>,

    #[clap(long = "gh-token")]
    #[arg(help = "GitHub token (can be specified multiple times)", global = true)]
    pub gh_token: Vec<String>,

    #[clap(long = "gitlab-token")]
    #[arg(
        help = "GitLab token with api scope (can be specified multiple times)",
        global = true
    )]
    pub gitlab_token: Vec<String>,

    #[clap(long = "trunk-token", env = "TRUNK_TOKEN")]
    #[arg(default_value_t = String::from(""), global = true)]
    pub trunk_token: String,

    #[clap(long = "trunk-repo-id", env = "TRUNK_REPO_ID")]
    #[arg(
        help = "Trunk repository ID, used in place of the repo's host/owner/name when uploading impacted targets. Required on GitLab",
        global = true
    )]
    pub trunk_repo_id: Option<String>,

    #[clap(long = "dry-run")]
    #[arg(default_value_t = false, global = true)]
    pub dry_run: bool,
}

impl Cli {
    /// Get all GitHub tokens (from CLI args and environment variable)
    pub fn get_github_tokens(&self) -> Vec<String> {
        let mut tokens = self.gh_token.clone();

        // Add GH_TOKEN from environment if no CLI tokens provided
        if tokens.is_empty() {
            if let Ok(env_token) = std::env::var("GH_TOKEN") {
                if !env_token.is_empty() {
                    tokens.push(env_token);
                }
            }
        }

        tokens
    }

    pub fn trunk_repo_id(&self) -> Option<String> {
        self.trunk_repo_id.clone().filter(|id| !id.is_empty())
    }

    /// Get all GitLab tokens (from CLI args, or the GITLAB_TOKEN environment variable)
    pub fn get_gitlab_tokens(&self) -> Vec<String> {
        let mut tokens = self.gitlab_token.clone();
        if tokens.is_empty() {
            if let Ok(env_token) = std::env::var("GITLAB_TOKEN") {
                if !env_token.is_empty() {
                    tokens.push(env_token);
                }
            }
        }
        tokens
    }
}

#[derive(Subcommand, Debug)]
pub enum Subcommands {
    /// Generate default configuration content for generator
    Defaultconfig,
    /// Print configuration content to json, or a single value if path is provided (e.g., "trunk.api")
    Config {
        /// Optional path to a specific config value (e.g., "trunk.api")
        #[arg(value_name = "PATH")]
        path: Option<String>,
    },
    /// Clean out conflicting PRs and requeue failed PRs
    Housekeeping,
    /// Simulate a test with flake rate in consideration
    TestSim,
    /// Generate pull requests
    Generate,
    /// upload targets
    UploadTargets(UploadTargets),
    /// Enqueue a pull request
    Enqueue(Enqueue),
}

#[derive(Parser, Debug)]
pub struct UploadTargets {
    /// Path to file that contains github-json block. Required on GitHub; on GitLab
    /// the merge request is read from the CI_* variables of a merge request pipeline.
    #[clap(long = "github-json")]
    pub github_json: Option<String>,

    /// Optional: Path to JSON file containing array of targets to upload directly
    /// If provided, targets will be read from this file instead of extracting from PR body
    /// File should contain a JSON array: ["target1", "target2", "target3"]
    #[clap(long = "targets-json")]
    pub targets_json: Option<String>,
}

#[derive(Parser, Debug)]
pub struct Enqueue {
    /// Pull request number to enqueue
    #[clap(short, long)]
    pub pr: String,
}
