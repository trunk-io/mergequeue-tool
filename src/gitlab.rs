use crate::forge::{Comment, Mergeability};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::Method;
use serde_json::{json, Value};

/// Minimal GitLab REST v4 client scoped to a single project.
pub struct GitLab {
    api: String,
    project_path: String,
    token: String,
    client: Client,
}

impl GitLab {
    /// `base_url` is the instance root (e.g. `https://gitlab.com`) and `project_path`
    /// the project's full path (e.g. `group/sub/project`).
    pub fn new(base_url: &str, project_path: &str, token: &str) -> Self {
        GitLab {
            api: format!("{}/api/v4", base_url.trim_end_matches('/')),
            project_path: project_path.to_string(),
            token: token.to_string(),
            client: Client::new(),
        }
    }

    pub fn project_url(&self) -> String {
        format!(
            "{}/projects/{}",
            self.api,
            self.project_path.replace('/', "%2F")
        )
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}{}", self.project_url(), path))
            .header("PRIVATE-TOKEN", &self.token)
    }

    fn send(&self, req: RequestBuilder) -> Result<(Value, Option<String>), String> {
        let res = req.send().map_err(|e| e.to_string())?;
        let status = res.status();
        let next_page = res
            .headers()
            .get("x-next-page")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let text = res.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("GitLab API returned {}: {}", status, text));
        }
        let value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok((value, next_page))
    }

    fn get_all(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<Value>, String> {
        let mut items = Vec::new();
        let mut page = "1".to_string();
        loop {
            let req = self
                .request(Method::GET, path)
                .query(query)
                .query(&[("per_page", "100"), ("page", page.as_str())]);
            let (value, next_page) = self.send(req)?;
            if let Value::Array(batch) = value {
                items.extend(batch);
            }
            match next_page {
                Some(next) => page = next,
                None => return Ok(items),
            }
        }
    }

    /// Opens a merge request and returns its iid.
    pub fn create_merge_request(
        &self,
        source_branch: &str,
        target_branch: &str,
        title: &str,
        description: &str,
        labels: &[&str],
    ) -> Result<u32, String> {
        let body = json!({
            "source_branch": source_branch,
            "target_branch": target_branch,
            "title": title,
            "description": description,
            "labels": labels.join(","),
            "remove_source_branch": true,
        });
        let (mr, _) = self.send(self.request(Method::POST, "/merge_requests").json(&body))?;
        mr["iid"]
            .as_u64()
            .map(|iid| iid as u32)
            .ok_or_else(|| format!("merge request response had no iid: {}", mr))
    }

    pub fn comment(&self, iid: &str, body: &str) -> Result<(), String> {
        let path = format!("/merge_requests/{}/notes", iid);
        let req = self
            .request(Method::POST, &path)
            .json(&json!({ "body": body }));
        self.send(req).map(|_| ())
    }

    pub fn close(&self, iid: &str) -> Result<(), String> {
        let path = format!("/merge_requests/{}", iid);
        let req = self
            .request(Method::PUT, &path)
            .json(&json!({ "state_event": "close" }));
        self.send(req).map(|_| ())
    }

    pub fn add_label(&self, iid: &str, label: &str) -> Result<(), String> {
        let path = format!("/merge_requests/{}", iid);
        let req = self
            .request(Method::PUT, &path)
            .json(&json!({ "add_labels": label }));
        self.send(req).map(|_| ())
    }

    pub fn merge_request(&self, iid: &str) -> Result<Value, String> {
        let path = format!("/merge_requests/{}", iid);
        self.send(self.request(Method::GET, &path)).map(|(v, _)| v)
    }

    /// The highest merge request iid in the project, or 0 if it has none.
    pub fn latest_iid(&self) -> Result<u32, String> {
        let req = self.request(Method::GET, "/merge_requests").query(&[
            ("state", "all"),
            ("order_by", "created_at"),
            ("sort", "desc"),
            ("per_page", "1"),
        ]);
        let (value, _) = self.send(req)?;
        Ok(value[0]["iid"].as_u64().unwrap_or(0) as u32)
    }

    /// Open merge requests, with `with_merge_status_recheck` asking GitLab to refresh
    /// merge statuses it has not computed since the target branch moved.
    pub fn open_merge_requests(&self) -> Result<Vec<Value>, String> {
        self.get_all(
            "/merge_requests",
            &[("state", "opened"), ("with_merge_status_recheck", "true")],
        )
    }

    /// User-written notes on a merge request, skipping system notes such as
    /// "added 1 commit".
    pub fn notes(&self, iid: &str) -> Result<Vec<Comment>, String> {
        let notes = self.get_all(&format!("/merge_requests/{}/notes", iid), &[])?;
        Ok(notes
            .iter()
            .filter(|n| !n["system"].as_bool().unwrap_or(false))
            .map(|n| Comment {
                body: n["body"].as_str().unwrap_or("").to_string(),
                created_at: n["created_at"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }
}

/// Map a merge request onto the conflict-only notion of mergeability that GitHub's
/// `mergeable` field has: statuses such as `ci_still_running` or `not_approved`
/// block a merge but are not conflicts, so they count as mergeable here.
pub fn mergeability(mr: &Value) -> Mergeability {
    if mr["has_conflicts"].as_bool().unwrap_or(false) {
        return Mergeability::Conflicting;
    }
    let status = mr["detailed_merge_status"]
        .as_str()
        .or_else(|| mr["merge_status"].as_str())
        .unwrap_or("");
    match status {
        "checking"
        | "unchecked"
        | "preparing"
        | "approvals_syncing"
        | "cannot_be_merged_recheck" => Mergeability::Unknown,
        "conflict" | "broken_status" | "cannot_be_merged" => Mergeability::Conflicting,
        _ => Mergeability::Mergeable,
    }
}
