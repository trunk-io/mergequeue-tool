use gen::forge::Mergeability;
use gen::gitlab::{mergeability, GitLab};
use gen::repo::{parse_remote_url, RepoRef};
use gen::trunk::GitLabMergeRequestEnv;
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

fn repo(host: &str, owner: &str, name: &str) -> RepoRef {
    RepoRef {
        host: host.to_string(),
        owner: owner.to_string(),
        name: name.to_string(),
    }
}

#[test]
fn parses_github_remotes() {
    assert_eq!(
        parse_remote_url("https://github.com/merge-demo/mergequeue-st.git"),
        Some(repo("github.com", "merge-demo", "mergequeue-st"))
    );
    assert_eq!(
        parse_remote_url("git@github.com:merge-demo/mergequeue-st.git"),
        Some(repo("github.com", "merge-demo", "mergequeue-st"))
    );
}

#[test]
fn keeps_gitlab_subgroups_in_owner() {
    assert_eq!(
        parse_remote_url("https://gitlab.com/trunk-io/demos/mergequeue.git"),
        Some(repo("gitlab.com", "trunk-io/demos", "mergequeue"))
    );
    assert_eq!(
        parse_remote_url("git@gitlab.com:trunk-io/demos/mergequeue.git"),
        Some(repo("gitlab.com", "trunk-io/demos", "mergequeue"))
    );
}

#[test]
fn strips_ci_credentials_and_keeps_https_port() {
    assert_eq!(
        parse_remote_url(
            "https://gitlab-ci-token:secret@gitlab.example.com:8443/group/project.git"
        ),
        Some(repo("gitlab.example.com:8443", "group", "project"))
    );
    assert_eq!(
        parse_remote_url("ssh://git@gitlab.example.com:2222/group/project.git"),
        Some(repo("gitlab.example.com", "group", "project"))
    );
}

#[test]
fn rejects_remote_without_owner() {
    assert_eq!(parse_remote_url("https://gitlab.com/project.git"), None);
}

#[test]
fn maps_gitlab_merge_status_onto_conflict_mergeability() {
    let cases = [
        (
            json!({"detailed_merge_status": "mergeable"}),
            Mergeability::Mergeable,
        ),
        (
            json!({"detailed_merge_status": "ci_still_running"}),
            Mergeability::Mergeable,
        ),
        (
            json!({"detailed_merge_status": "not_approved"}),
            Mergeability::Mergeable,
        ),
        (
            json!({"detailed_merge_status": "checking"}),
            Mergeability::Unknown,
        ),
        (
            json!({"detailed_merge_status": "unchecked"}),
            Mergeability::Unknown,
        ),
        (
            json!({"detailed_merge_status": "conflict"}),
            Mergeability::Conflicting,
        ),
        (
            json!({"detailed_merge_status": "ci_must_pass", "has_conflicts": true}),
            Mergeability::Conflicting,
        ),
        (
            json!({"merge_status": "cannot_be_merged"}),
            Mergeability::Conflicting,
        ),
        (
            json!({"merge_status": "can_be_merged"}),
            Mergeability::Mergeable,
        ),
    ];
    for (mr, expected) in cases {
        assert_eq!(mergeability(&mr), expected, "for {}", mr);
    }
}

fn mr_pipeline_vars() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("CI_PROJECT_PATH", "trunk-io/demos/mergequeue"),
        ("CI_MERGE_REQUEST_IID", "42"),
        ("CI_COMMIT_SHA", "headsha"),
        ("CI_MERGE_REQUEST_TARGET_BRANCH_NAME", "main"),
        ("CI_MERGE_REQUEST_DESCRIPTION", "body\n\ndeps=[a,b]\n"),
    ])
}

fn from(vars: &HashMap<&'static str, &'static str>) -> Result<GitLabMergeRequestEnv, String> {
    GitLabMergeRequestEnv::from_vars(|name| vars.get(name).map(|v| v.to_string()))
}

#[test]
fn reads_merge_request_from_ci_vars() {
    assert_eq!(
        from(&mr_pipeline_vars()),
        Ok(GitLabMergeRequestEnv {
            project_path: "trunk-io/demos/mergequeue".to_string(),
            iid: 42,
            sha: "headsha".to_string(),
            target_branch: "main".to_string(),
            description: Some("body\n\ndeps=[a,b]\n".to_string()),
        })
    );
}

#[test]
fn prefers_source_sha_in_merged_results_pipelines() {
    let mut vars = mr_pipeline_vars();
    vars.insert("CI_COMMIT_SHA", "mergecommitsha");
    vars.insert("CI_MERGE_REQUEST_SOURCE_BRANCH_SHA", "headsha");
    assert_eq!(from(&vars).unwrap().sha, "headsha");
}

#[test]
fn drops_truncated_description() {
    let mut vars = mr_pipeline_vars();
    vars.insert("CI_MERGE_REQUEST_DESCRIPTION_IS_TRUNCATED", "true");
    assert_eq!(from(&vars).unwrap().description, None);
}

#[test]
fn requires_a_merge_request_pipeline() {
    let mut vars = mr_pipeline_vars();
    vars.remove("CI_MERGE_REQUEST_IID");
    assert!(from(&vars).unwrap_err().contains("CI_MERGE_REQUEST_IID"));
}

struct Recorded {
    request_line: String,
    headers: Vec<String>,
    body: String,
}

/// Serves one canned `(extra headers, body)` response per connection, in order, and
/// returns the requests it saw once all responses are sent.
fn stub_server(
    responses: Vec<(&'static str, String)>,
) -> (String, thread::JoinHandle<Vec<Recorded>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for (extra_headers, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end().to_string();
                if line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_lowercase().strip_prefix("content-length: ") {
                    content_length = v.parse().unwrap();
                }
                headers.push(line);
            }
            let mut req_body = vec![0; content_length];
            reader.read_exact(&mut req_body).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n{}Content-Length: {}\r\n\r\n{}",
                extra_headers,
                body.len(),
                body
            )
            .unwrap();
            seen.push(Recorded {
                request_line: request_line.trim_end().to_string(),
                headers,
                body: String::from_utf8(req_body).unwrap(),
            });
        }
        seen
    });
    (base, handle)
}

#[test]
fn creates_merge_request_on_encoded_project_path() {
    let (base, server) = stub_server(vec![("", json!({"iid": 17}).to_string())]);
    let gl = GitLab::new(&base, "trunk-io/demos/mergequeue", "glpat-test");

    let iid = gl
        .create_merge_request("stack-change/abcde-1", "main", "title", "deps=[a]", &["mq"])
        .unwrap();

    assert_eq!(iid, 17);
    let seen = server.join().unwrap();
    assert_eq!(
        seen[0].request_line,
        "POST /api/v4/projects/trunk-io%2Fdemos%2Fmergequeue/merge_requests HTTP/1.1"
    );
    assert!(seen[0]
        .headers
        .iter()
        .any(|h| h.to_lowercase() == "private-token: glpat-test"));
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["source_branch"], "stack-change/abcde-1");
    assert_eq!(body["target_branch"], "main");
    assert_eq!(body["labels"], "mq");
}

#[test]
fn follows_pagination_for_open_merge_requests() {
    let (base, server) = stub_server(vec![
        ("x-next-page: 2\r\n", json!([{"iid": 1}]).to_string()),
        ("x-next-page: \r\n", json!([{"iid": 2}]).to_string()),
    ]);
    let gl = GitLab::new(&base, "group/project", "t");

    let mrs = gl.open_merge_requests().unwrap();

    let iids: Vec<u64> = mrs.iter().map(|mr| mr["iid"].as_u64().unwrap()).collect();
    assert_eq!(iids, vec![1, 2]);
    let seen = server.join().unwrap();
    assert!(seen[0].request_line.contains("state=opened"));
    assert!(seen[1].request_line.contains("page=2"));
}
