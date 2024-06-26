use confique::toml::{self, FormatOptions};
use confique::Config;
use parse_duration::parse;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    SingleQueue,
    ParallelQueue,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Build {
    None,
    Bazel,
}

#[derive(Config, Serialize)]
pub struct Conf {
    #[config(default = "singlequeue")]
    pub mode: Mode,

    #[config(default = "none")]
    pub build: Build,

    #[config(nested)]
    pub trunk: TrunkConf,

    #[config(nested)]
    pub git: GitConf,

    #[config(nested)]
    pub pullrequest: PullRequestConf,

    #[config(nested)]
    pub test: TestConf,

    #[config(nested)]
    pub merge: MergeConf,
}

#[derive(Config, Serialize)]
pub struct TrunkConf {
    #[config(default = "api.trunk.io")]
    pub api: String,
}

#[derive(Config, Serialize)]
pub struct GitConf {
    #[config(default = "Jane Doe")]
    pub name: String,

    #[config(default = "bot@email.com")]
    pub email: String,
}

#[derive(Config, Serialize)]
pub struct PullRequestConf {
    #[config(default = "")]
    pub labels: String,

    #[config(default = "")]
    pub comment: String,

    #[config(default = "This pull request was generated by the 'mq' tool")]
    pub body: String,

    #[config(default = 0)]
    pub requests_per_hour: u32,

    /// The desired length of time generate should run for attempting to
    /// distribute the requests_per_hour over that time period
    #[config(default = "10 minutes")]
    pub run_generate_for: String,

    #[config(default = 0)]
    pub requests_per_run: u32,

    #[config(default = "bazel/")]
    pub change_code_path: String,

    #[config(default = 1)]
    pub max_deps: usize,

    #[config(default = 1)]
    pub max_impacted_deps: usize,

    #[config(default = 100)]
    pub logical_conflict_every: u32,

    #[config(default = "logical-conflict.txt")]
    pub logical_conflict_file: String,

    #[config(default = ["removed from the merge queue", "To merge this pull request, check the box to the left", "/trunk merge"])]
    pub detect_stale_pr_comments: Vec<String>,

    #[config(default = "4 hours")]
    pub close_stale_after: String,
}

#[derive(Config, Serialize)]
pub struct TestConf {
    #[config(default = 0.1)]
    pub flake_rate: f32,

    #[config(default = "1 second")]
    pub sleep_for: String,
}

#[derive(Config, Serialize)]
pub struct MergeConf {
    #[config(default = "")]
    pub labels: String,

    #[config(default = "")]
    pub comment: String,

    #[config(default = "")]
    pub run: String,
}

impl Conf {
    pub fn print_default() {
        let default_config = toml::template::<Conf>(FormatOptions::default());
        println!("{}", default_config);
    }

    pub fn sleep_duration(&self) -> std::time::Duration {
        parse(&self.test.sleep_for).expect("Failed to parse sleep_for into a Duration")
    }

    pub fn is_generator_disabled(&self) -> bool {
        self.pullrequest.requests_per_hour == 0 && self.pullrequest.requests_per_run == 0
    }

    pub fn close_stale_after_duration(&self) -> std::time::Duration {
        parse(&self.pullrequest.close_stale_after)
            .expect("Failed to parse close_stale_after into a Duration")
    }

    pub fn run_generate_for_duration(&self) -> std::time::Duration {
        parse(&self.pullrequest.run_generate_for)
            .expect("Failed to parse run_generate_for into a Duration")
    }

    pub fn is_valid(&self) -> Result<(), &'static str> {
        if self.test.flake_rate <= 0.0 || self.test.flake_rate > 1.0 {
            return Err("flake_rate must be between 0.0 and 1.0");
        }

        if parse(&self.test.sleep_for).is_err() {
            return Err("sleep_for must be a valid duration string");
        }

        if self.pullrequest.requests_per_hour > 0 && self.pullrequest.requests_per_run > 0 {
            return Err("cannot set both requests_per_hour and requests_per_run");
        }

        Ok(())
    }
}
