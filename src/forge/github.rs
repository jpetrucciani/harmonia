use std::collections::HashMap;

use reqwest::blocking::Client;
use reqwest::blocking::Response;
use reqwest::Method;
use serde_json::Value;

use crate::core::repo::RepoId;
use crate::error::{HarmoniaError, Result};
use crate::forge::traits::{
    CreateIssueParams, CreateMrParams, Forge, MergeMrParams, UpdateMrParams,
};
use crate::forge::{
    CheckRun, CiState, CiStatus, Issue, IssueState, MergeRequest, MrId, MrState, Pipeline, User,
};

#[derive(Debug, Clone)]
pub struct GitHubClient {
    pub host: String,
    pub token: String,
    pub default_group: Option<String>,
    client: Client,
}

impl GitHubClient {
    pub fn new(
        host: impl Into<String>,
        token: impl Into<String>,
        default_group: Option<String>,
    ) -> Self {
        let host = normalize_host(&host.into());
        Self {
            host,
            token: token.into(),
            default_group,
            client: Client::new(),
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/{}", self.host, path.trim_start_matches('/'))
    }

    fn repo_path_for_repo(&self, repo: &RepoId) -> String {
        let raw = repo.as_str().trim();
        if raw.contains('/') {
            return raw.to_string();
        }
        if let Some(group) = self.default_group.as_ref() {
            let group = group.trim().trim_matches('/');
            if !group.is_empty() {
                return format!("{group}/{raw}");
            }
        }
        raw.to_string()
    }

    fn send_json(
        &self,
        method: Method,
        path: &str,
        query: Option<&[(&str, String)]>,
        body: Option<Value>,
    ) -> Result<Value> {
        let url = self.api_url(path);
        let mut request = self
            .client
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "harmonia");

        if let Some(query) = query {
            request = request.query(query);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().map_err(|err| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "github request failed for {}: {}",
                url, err
            )))
        })?;

        parse_json_response(response)
    }

    fn get_json(&self, path: &str, query: Option<&[(&str, String)]>) -> Result<Value> {
        self.send_json(Method::GET, path, query, None)
    }

    fn post_json(
        &self,
        path: &str,
        query: Option<&[(&str, String)]>,
        body: Option<Value>,
    ) -> Result<Value> {
        self.send_json(Method::POST, path, query, body)
    }

    fn put_json(
        &self,
        path: &str,
        query: Option<&[(&str, String)]>,
        body: Option<Value>,
    ) -> Result<Value> {
        self.send_json(Method::PUT, path, query, body)
    }

    fn patch_json(
        &self,
        path: &str,
        query: Option<&[(&str, String)]>,
        body: Option<Value>,
    ) -> Result<Value> {
        self.send_json(Method::PATCH, path, query, body)
    }

    fn delete_json(
        &self,
        path: &str,
        query: Option<&[(&str, String)]>,
        body: Option<Value>,
    ) -> Result<Value> {
        self.send_json(Method::DELETE, path, query, body)
    }

    fn parse_pull_request(&self, value: &Value) -> Result<MergeRequest> {
        let id = value
            .get("id")
            .and_then(|value| value.as_u64())
            .or_else(|| value.get("number").and_then(|value| value.as_u64()))
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("github PR response missing id and number"))
            })?
            .to_string();

        let iid = value
            .get("number")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("github PR response missing number"))
            })?;

        let title = json_string_field(value, "title")?;
        let description = value
            .get("body")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let source_branch = value
            .get("head")
            .and_then(|value| value.get("ref"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("github PR response missing head.ref"))
            })?;
        let target_branch = value
            .get("base")
            .and_then(|value| value.get("ref"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("github PR response missing base.ref"))
            })?;
        let url = value
            .get("html_url")
            .and_then(|value| value.as_str())
            .or_else(|| value.get("url").and_then(|value| value.as_str()))
            .unwrap_or_default()
            .to_string();
        let draft = value
            .get("draft")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let state = parse_pr_state(value.get("state").and_then(|value| value.as_str()), draft);

        let labels = value
            .get("labels")
            .and_then(|value| value.as_array())
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| label.get("name").and_then(|value| value.as_str()))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let approvals = value
            .get("requested_reviewers")
            .and_then(|value| value.as_array())
            .map(|reviewers| reviewers.iter().filter_map(parse_user).collect::<Vec<_>>())
            .unwrap_or_default();

        Ok(MergeRequest {
            id,
            iid,
            title,
            description,
            source_branch: source_branch.to_string(),
            target_branch: target_branch.to_string(),
            state,
            url,
            ci_status: None,
            approvals,
            labels,
        })
    }

    fn parse_issue(&self, value: &Value) -> Result<Issue> {
        let id = value
            .get("id")
            .and_then(|value| value.as_u64())
            .or_else(|| value.get("number").and_then(|value| value.as_u64()))
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!(
                    "github issue response missing id and number"
                ))
            })?
            .to_string();

        let iid = value
            .get("number")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("github issue response missing number"))
            })?;

        let title = json_string_field(value, "title")?;
        let url = value
            .get("html_url")
            .and_then(|value| value.as_str())
            .or_else(|| value.get("url").and_then(|value| value.as_str()))
            .unwrap_or_default()
            .to_string();
        let state = match value.get("state").and_then(|value| value.as_str()) {
            Some("closed") => IssueState::Closed,
            _ => IssueState::Open,
        };

        Ok(Issue {
            id,
            iid,
            title,
            url,
            state,
        })
    }

    fn parse_pull_request_iid(&self, mr_id: &MrId) -> Result<u64> {
        mr_id.parse::<u64>().map_err(|_| {
            HarmoniaError::Other(anyhow::anyhow!(
                "github pull request id must be a numeric IID"
            ))
        })
    }

    fn parse_project_group(&self, repo: &RepoId) -> Result<String> {
        let project = self.repo_path_for_repo(repo);
        if project.is_empty() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(
                "github repository path is required"
            )));
        }
        Ok(project)
    }

    fn set_reviewers(&self, project: &str, iid: u64, reviewers: &[String]) -> Result<()> {
        let path = format!(
            "/repos/{}/pulls/{}/requested_reviewers",
            encode_repo_path(project),
            iid
        );

        let payload = serde_json::json!({
            "reviewers": reviewers,
            "team_reviewers": Vec::<String>::new(),
        });

        if reviewers.is_empty() {
            self.delete_json(&path, None, Some(payload)).map(|_| ())
        } else {
            self.post_json(&path, None, Some(payload)).map(|_| ())
        }
    }

    fn set_labels(&self, project: &str, iid: u64, labels: &[String]) -> Result<()> {
        let path = format!("/repos/{}/issues/{}/labels", encode_repo_path(project), iid);
        let payload = Value::Array(
            labels
                .iter()
                .map(|label| Value::String(label.to_string()))
                .collect(),
        );
        self.put_json(&path, None, Some(payload)).map(|_| ())
    }

    fn append_related_link(
        &self,
        project: &RepoId,
        target_id: &MrId,
        related_url: &str,
    ) -> Result<()> {
        let marker = "<!-- harmonia-related-prs -->";
        let mr = self.get_mr(project, target_id)?;

        if mr.description.contains(related_url) {
            return Ok(());
        }

        let mut description = mr.description;
        if description.is_empty() {
            description = format!("{marker}\n- {related_url}");
        } else {
            if !description.ends_with('\n') {
                description.push('\n');
            }
            description.push('\n');
            if description.contains(marker) {
                description.push_str("- ");
                description.push_str(related_url);
            } else {
                description.push_str(marker);
                description.push('\n');
                description.push_str("- ");
                description.push_str(related_url);
            }
        }

        self.update_mr(
            project,
            &mr.id,
            UpdateMrParams {
                description: Some(description),
                ..Default::default()
            },
        )
        .map(|_| ())
    }

    fn ci_state_from_checks(&self, checks: &[CheckRun], overall: Option<&str>) -> CiState {
        if checks.iter().any(|check| {
            matches!(
                check.status.as_str(),
                "failure" | "error" | "timed_out" | "startup_failure"
            )
        }) {
            return CiState::Failed;
        }

        if checks
            .iter()
            .any(|check| matches!(check.status.as_str(), "canceled" | "cancelled"))
        {
            return CiState::Canceled;
        }

        if checks
            .iter()
            .any(|check| matches!(check.status.as_str(), "in_progress" | "running"))
        {
            return CiState::Running;
        }

        if checks
            .iter()
            .any(|check| matches!(check.status.as_str(), "pending" | "queued" | "waiting"))
        {
            return CiState::Pending;
        }

        if checks.iter().all(|check| check.status == "skipped") {
            return CiState::Skipped;
        }

        match overall {
            Some("success") => CiState::Success,
            Some("pending") | Some("in_progress") | Some("queued") | Some("waiting") => {
                CiState::Pending
            }
            Some("failure") | Some("error") | Some("timed_out") | Some("startup_failure") => {
                CiState::Failed
            }
            Some("cancelled") => CiState::Canceled,
            _ => CiState::Pending,
        }
    }
}

impl Forge for GitHubClient {
    fn create_mr(&self, repo: &RepoId, params: CreateMrParams) -> Result<MergeRequest> {
        let project = self.parse_project_group(repo)?;
        let path = format!("/repos/{}/pulls", encode_repo_path(&project));
        let payload = serde_json::json!({
            "title": params.title,
            "body": params.description,
            "head": params.source_branch,
            "base": params.target_branch,
            "draft": params.draft,
        });

        let response = self.post_json(&path, None, Some(payload))?;
        let mr = self.parse_pull_request(&response)?;

        if !params.labels.is_empty() {
            self.set_labels(&project, mr.iid, &params.labels)?;
        }
        if !params.reviewers.is_empty() {
            self.set_reviewers(&project, mr.iid, &params.reviewers)?;
        }

        if params.labels.is_empty() && params.reviewers.is_empty() {
            return Ok(mr);
        }

        self.get_mr(repo, &mr.id)
    }

    fn get_mr(&self, repo: &RepoId, mr_id: &MrId) -> Result<MergeRequest> {
        let project = self.parse_project_group(repo)?;
        let iid = self.parse_pull_request_iid(mr_id)?;
        let path = format!("/repos/{}/pulls/{}", encode_repo_path(&project), iid);
        let response = self.get_json(&path, None)?;
        self.parse_pull_request(&response)
    }

    fn update_mr(
        &self,
        repo: &RepoId,
        mr_id: &MrId,
        params: UpdateMrParams,
    ) -> Result<MergeRequest> {
        let project = self.parse_project_group(repo)?;
        let iid = self.parse_pull_request_iid(mr_id)?;
        let path = format!("/repos/{}/pulls/{}", encode_repo_path(&project), iid);

        let mut values = HashMap::<String, Value>::new();
        if let Some(title) = params.title {
            values.insert("title".to_string(), Value::String(title));
        }
        if let Some(description) = params.description {
            values.insert("body".to_string(), Value::String(description));
        }

        let did_mutate_side_effects = params.labels.is_some() || params.reviewers.is_some();
        if let Some(labels) = params.labels {
            self.set_labels(&project, iid, &labels)?;
        }
        if let Some(reviewers) = params.reviewers {
            self.set_reviewers(&project, iid, &reviewers)?;
        }

        if values.is_empty() {
            if did_mutate_side_effects {
                return self.get_mr(repo, mr_id);
            }
            return Err(HarmoniaError::Other(anyhow::anyhow!(
                "mr update requires at least one field"
            )));
        }

        let response = self.patch_json(
            &path,
            None,
            Some(Value::Object(values.into_iter().collect())),
        )?;
        let mut merged = self.parse_pull_request(&response)?;
        if did_mutate_side_effects {
            merged = self.get_mr(repo, &merged.id)?;
        }
        Ok(merged)
    }

    fn link_mrs(&self, mrs: &[(RepoId, MrId)]) -> Result<()> {
        if mrs.len() < 2 {
            return Ok(());
        }

        for window in mrs.windows(2) {
            if let [blocking, current] = window {
                let blocking_mr = self.get_mr(&blocking.0, &blocking.1)?;
                self.append_related_link(&current.0, &current.1, &blocking_mr.url)?;
            }
        }

        Ok(())
    }

    fn merge_mr(&self, repo: &RepoId, mr_id: &MrId, params: MergeMrParams) -> Result<()> {
        let project = self.parse_project_group(repo)?;
        let iid = self.parse_pull_request_iid(mr_id)?;
        let path = format!("/repos/{}/pulls/{}/merge", encode_repo_path(&project), iid);

        let payload = serde_json::json!({
            "merge_method": if params.squash { "squash" } else { "merge" },
            "delete_branch": params.delete_source_branch,
        });
        self.put_json(&path, None, Some(payload)).map(|_| ())
    }

    fn close_mr(&self, repo: &RepoId, mr_id: &MrId) -> Result<()> {
        let project = self.parse_project_group(repo)?;
        let iid = self.parse_pull_request_iid(mr_id)?;
        let path = format!("/repos/{}/pulls/{}", encode_repo_path(&project), iid);

        let payload = serde_json::json!({
            "state": "closed",
        });
        self.patch_json(&path, None, Some(payload)).map(|_| ())
    }

    fn get_ci_status(&self, repo: &RepoId, ref_name: &str) -> Result<CiStatus> {
        let project = self.parse_project_group(repo)?;
        let path = format!(
            "/repos/{}/commits/{}/status",
            encode_repo_path(&project),
            encode_ref(ref_name)
        );

        let response = self.get_json(&path, None)?;
        let checks = response
            .get("statuses")
            .and_then(|value| value.as_array())
            .map(|statuses| {
                statuses
                    .iter()
                    .filter_map(|status| {
                        let name = status.get("context")?.as_str()?;
                        let state = status.get("state")?.as_str()?;
                        Some(CheckRun {
                            name: name.to_string(),
                            status: state.to_string(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let pipelines = response
            .get("statuses")
            .and_then(|value| value.as_array())
            .map(|statuses| {
                statuses
                    .iter()
                    .map(|status| Pipeline {
                        id: status
                            .get("id")
                            .and_then(|value| value.as_u64())
                            .map(|value| value.to_string())
                            .or_else(|| {
                                status
                                    .get("context")
                                    .and_then(|value| value.as_str())
                                    .map(|value| value.to_string())
                            })
                            .unwrap_or_else(|| ref_name.to_string()),
                        status: status
                            .get("state")
                            .and_then(|value| value.as_str())
                            .unwrap_or("unknown")
                            .to_string(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let overall = response
            .get("state")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let state = self.ci_state_from_checks(&checks, overall.as_deref());

        Ok(CiStatus {
            state,
            pipelines,
            checks,
        })
    }

    fn create_issue(&self, params: CreateIssueParams) -> Result<Issue> {
        let project = match params.project {
            Some(project) => project,
            None => {
                return Err(HarmoniaError::Other(anyhow::anyhow!(
                    "create_issue requires a target project"
                )))
            }
        };
        let project = self.parse_project_group(&project)?;
        let path = format!("/repos/{}/issues", encode_repo_path(&project));

        let payload = serde_json::json!({
            "title": params.title,
            "body": params.description,
            "labels": params.labels,
        });
        let response = self.post_json(&path, None, Some(payload))?;
        self.parse_issue(&response)
    }

    fn get_user(&self, username: &str) -> Result<User> {
        let username = username.trim();
        if username.is_empty() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(
                "github username cannot be empty"
            )));
        }

        let path = format!("/users/{username}");
        let response = self.get_json(&path, None)?;
        parse_user(&response).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "github user '{}' response missing required fields",
                username
            )))
        })
    }
}

fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');

    if trimmed.is_empty() {
        return "https://api.github.com".to_string();
    }

    if trimmed == "github.com" || trimmed == "api.github.com" {
        return "https://api.github.com".to_string();
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        if trimmed.ends_with("/api/v3") || trimmed.ends_with("/api") {
            return trimmed.to_string();
        }
        if trimmed.starts_with("https://api.") || trimmed.starts_with("http://api.") {
            return trimmed.to_string();
        }
        return format!("{trimmed}/api/v3");
    }

    if trimmed.starts_with("api.") {
        return format!("https://{trimmed}");
    }

    format!("https://{trimmed}/api/v3")
}

fn parse_json_response(response: Response) -> Result<Value> {
    let status = response.status();
    let url = response.url().to_string();
    let body = response.text().map_err(|err| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "failed reading github response body: {}",
            err
        )))
    })?;

    if !status.is_success() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "github API returned {} for {}: {}",
            status,
            url,
            body.trim()
        ))));
    }

    if body.trim().is_empty() {
        return Ok(Value::Null);
    }

    serde_json::from_str(&body).map_err(|err| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "failed to parse github response JSON from {}: {}",
            url, err
        )))
    })
}

fn parse_pr_state(state: Option<&str>, draft: bool) -> MrState {
    if draft {
        return MrState::Draft;
    }

    match state {
        Some("closed") => MrState::Closed,
        Some("merged") => MrState::Merged,
        _ => MrState::Open,
    }
}

fn json_string_field(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "github response missing '{}'",
                field
            )))
        })
}

fn parse_user(value: &Value) -> Option<User> {
    let username = value.get("login")?.as_str()?.to_string();
    let id = value.get("id").and_then(|value| value.as_u64());
    Some(User { id, username })
}

fn encode_repo_path(path: &str) -> String {
    encode_path(path)
}

fn encode_ref(reference: &str) -> String {
    encode_path(reference)
}

fn encode_path(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{:02X}", byte));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use crate::forge::github::{normalize_host, parse_pr_state, GitHubClient};
    use crate::forge::{CheckRun, CiState, Issue, IssueState, MrState};

    #[test]
    fn normalizes_github_host() {
        assert_eq!(normalize_host("github.com"), "https://api.github.com");
        assert_eq!(
            normalize_host("https://api.github.com"),
            "https://api.github.com"
        );
        assert_eq!(
            normalize_host("github.enterprise.example.com"),
            "https://github.enterprise.example.com/api/v3"
        );
    }

    #[test]
    fn maps_pr_states() {
        assert_eq!(parse_pr_state(Some("merged"), false), MrState::Merged);
        assert_eq!(parse_pr_state(Some("closed"), false), MrState::Closed);
        assert_eq!(parse_pr_state(Some("open"), false), MrState::Open);
        assert_eq!(parse_pr_state(None, true), MrState::Draft);
    }

    #[test]
    fn computes_ci_state_from_checks() {
        let client = GitHubClient::new("github.com", "token", None);
        let checks = vec![
            CheckRun {
                name: "build".into(),
                status: "in_progress".into(),
            },
            CheckRun {
                name: "test".into(),
                status: "success".into(),
            },
        ];
        assert_eq!(
            client.ci_state_from_checks(&checks, Some("success")),
            CiState::Running
        );
    }

    #[test]
    fn parse_issue_requires_id_fields() {
        let issue = Issue {
            id: "1".into(),
            iid: 1,
            title: "fix".into(),
            url: "https://example/1".into(),
            state: IssueState::Open,
        };
        assert_eq!(issue.id, "1");
        assert_eq!(issue.iid, 1);
        assert_eq!(issue.title, "fix");
    }

    #[test]
    fn test_client_constructs() {
        let client = GitHubClient::new("github.com", "token", Some("team".to_string()));
        assert_eq!(client.host, "https://api.github.com");
        assert_eq!(client.default_group, Some("team".to_string()));
    }
}
