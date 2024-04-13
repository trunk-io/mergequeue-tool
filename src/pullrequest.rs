use std::env;

pub fn read_env() {
    let repository = env::var("REPOSITORY").expect("Missing Repo params");
    let target_branch = env::var("TARGET_BRANCH").expect("Missing Repo params");

    let repo_parts: Vec<&str> = repository.split('/').collect();
    let repo_owner = repo_parts.get(0).expect("Invalid REPOSITORY format");
    let repo_name = repo_parts.get(1).expect("Invalid REPOSITORY format");

    let pr_number = env::var("PR_NUMBER").expect("Missing PR params");
    let pr_branch_head_sha = env::var("PR_BRANCH_HEAD_SHA").expect("Missing PR params");

    println!("REPO_OWNER: {}", repo_owner);
    println!("REPO_NAME: {}", repo_name);
    println!("TARGET_BRANCH: {}", target_branch);
    println!("PR_NUMBER: {}", pr_number);
    println!("PR_BRANCH_HEAD_SHA: {}", pr_branch_head_sha);
}
