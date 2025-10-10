use gen::github::GitHubAction;
use regex::Regex;

#[test]
fn test_parse_deps_from_pr_body() {
    // Test data that mimics a GitHub PR event JSON
    let github_json = r#"{
        "repository": "owner/repo",
        "event": {
            "pull_request": {
                "number": 123,
                "head": {
                    "sha": "abc123def456"
                },
                "body": "This is a test PR\n\nSome description here\n\ndeps=[a,b]\n\nMore content"
            }
        }
    }"#;

    // Parse the GitHub action data
    let ga = GitHubAction::from_json(github_json);

    // Verify the PR body was parsed correctly
    assert!(ga.event.pull_request.body.is_some());
    let body = ga.event.pull_request.body.as_ref().unwrap();
    assert!(body.contains("deps=[a,b]"));

    // Test the regex parsing logic (same as in src/trunk.rs)
    let re = Regex::new(r".*deps=\[(.*?)\].*").unwrap();
    let mut impacted_targets: Vec<String> = Vec::new();

    if let Some(caps) = re.captures(body) {
        impacted_targets = caps[1]
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect::<Vec<String>>();
    }

    // Verify the extracted dependencies
    assert_eq!(impacted_targets.len(), 2);
    assert_eq!(impacted_targets[0], "a");
    assert_eq!(impacted_targets[1], "b");
}

#[test]
fn test_parse_deps_with_spaces() {
    // Test with spaces around the dependencies
    let github_json = r#"{
        "repository": "owner/repo",
        "event": {
            "pull_request": {
                "number": 123,
                "head": {
                    "sha": "abc123def456"
                },
                "body": "deps=[ a , b , c ]"
            }
        }
    }"#;

    let ga = GitHubAction::from_json(github_json);
    let body = ga.event.pull_request.body.as_ref().unwrap();

    let re = Regex::new(r".*deps=\[(.*?)\].*").unwrap();
    let mut impacted_targets: Vec<String> = Vec::new();

    if let Some(caps) = re.captures(body) {
        impacted_targets = caps[1]
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect::<Vec<String>>();
    }

    // Verify spaces are trimmed
    assert_eq!(impacted_targets.len(), 3);
    assert_eq!(impacted_targets[0], "a");
    assert_eq!(impacted_targets[1], "b");
    assert_eq!(impacted_targets[2], "c");
}

#[test]
fn test_parse_deps_single_dependency() {
    // Test with a single dependency
    let github_json = r#"{
        "repository": "owner/repo",
        "event": {
            "pull_request": {
                "number": 123,
                "head": {
                    "sha": "abc123def456"
                },
                "body": "Some text\ndeps=[single-target]\nMore text"
            }
        }
    }"#;

    let ga = GitHubAction::from_json(github_json);
    let body = ga.event.pull_request.body.as_ref().unwrap();

    let re = Regex::new(r".*deps=\[(.*?)\].*").unwrap();
    let mut impacted_targets: Vec<String> = Vec::new();

    if let Some(caps) = re.captures(body) {
        impacted_targets = caps[1]
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect::<Vec<String>>();
    }

    assert_eq!(impacted_targets.len(), 1);
    assert_eq!(impacted_targets[0], "single-target");
}

#[test]
fn test_parse_deps_no_match() {
    // Test when no deps pattern is found
    let github_json = r#"{
        "repository": "owner/repo",
        "event": {
            "pull_request": {
                "number": 123,
                "head": {
                    "sha": "abc123def456"
                },
                "body": "This PR has no deps information"
            }
        }
    }"#;

    let ga = GitHubAction::from_json(github_json);
    let body = ga.event.pull_request.body.as_ref().unwrap();

    let re = Regex::new(r".*deps=\[(.*?)\].*").unwrap();
    let mut impacted_targets: Vec<String> = Vec::new();

    if let Some(caps) = re.captures(body) {
        impacted_targets = caps[1]
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect::<Vec<String>>();
    }

    // Should be empty when no match is found
    assert_eq!(impacted_targets.len(), 0);
}

#[test]
fn test_parse_deps_empty_brackets() {
    // Test with empty deps brackets
    let github_json = r#"{
        "repository": "owner/repo",
        "event": {
            "pull_request": {
                "number": 123,
                "head": {
                    "sha": "abc123def456"
                },
                "body": "deps=[]"
            }
        }
    }"#;

    let ga = GitHubAction::from_json(github_json);
    let body = ga.event.pull_request.body.as_ref().unwrap();

    let re = Regex::new(r".*deps=\[(.*?)\].*").unwrap();
    let mut impacted_targets: Vec<String> = Vec::new();

    if let Some(caps) = re.captures(body) {
        impacted_targets = caps[1]
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect::<Vec<String>>();
    }

    // Should have one empty string when brackets are empty
    assert_eq!(impacted_targets.len(), 1);
    assert_eq!(impacted_targets[0], "");
}
