use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[clap(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: Option<Subcommands>,

    #[clap(long = "gh-token")]
    #[arg(default_value_t = String::from(""))]
    pub gh_token: String,

    #[clap(long = "trunk-token", env = "TRUNK_TOKEN")]
    #[arg(default_value_t = String::from(""))]
    pub trunk_token: String,

    #[clap(long = "dry-run")]
    #[arg(default_value_t = false)]
    pub dry_run: bool,
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
}

#[derive(Parser, Debug)]
pub struct UploadTargets {
    // Path to file that contains github-json block
    #[clap(long = "github-json")]
    pub github_json: String,
}
