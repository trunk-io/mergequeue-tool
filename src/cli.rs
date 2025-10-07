use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[clap(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: Option<Subcommands>,

    #[clap(long = "gh-token")]
    #[arg(help = "GitHub token (can be specified multiple times)")]
    pub gh_token: Vec<String>,

    #[clap(long = "trunk-token", env = "TRUNK_TOKEN")]
    #[arg(default_value_t = String::from(""))]
    pub trunk_token: String,

    #[clap(long = "dry-run")]
    #[arg(default_value_t = false)]
    pub dry_run: bool,
}

impl Cli {
    /// Get all GitHub tokens
    pub fn get_github_tokens(&self) -> Vec<String> {
        self.gh_token.clone()
    }
}

#[derive(Subcommand, Debug)]
pub enum Subcommands {
    /// Generate default configuration content for generator
    Defaultconfig,
    /// Print configuration content to json
    Config,
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
    // Path to file that contains github-json block
    #[clap(long = "github-json")]
    pub github_json: String,
}

#[derive(Parser, Debug)]
pub struct Enqueue {
    /// Pull request number to enqueue
    #[clap(short, long)]
    pub pr: String,
}
