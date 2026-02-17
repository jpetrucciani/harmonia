use std::collections::HashMap;

use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
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
pub struct GitLabClient {
    pub host: String,
    pub token: String,
    pub default_group: Option<String>,
    client: Client,
}

impl GitLabClient {
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
        format!("{}/api/v4{}", self.host, path)
    }

    fn project_path_for_repo(&self, repo: &RepoId) -> String {
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
            .header("PRIVATE-TOKEN", &self.token)
            .header("Accept", "application/json");
        if let Some(query) = query {
            request = request.query(query);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().map_err(|err| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "gitlab request failed for {}: {}",
                url, err
            )))
        })?;
        parse_json_response(response)
    }

    fn put_json(
        &self,
        path: &str,
        query: Option<&[(&str, String)]>,
        body: Option<Value>,
    ) -> Result<Value> {
        self.send_json(Method::PUT, path, query, body)
    }

    fn post_json(
        &self,
        path: &str,
        query: Option<&[(&str, String)]>,
        body: Option<Value>,
    ) -> Result<Value> {
        self.send_json(Method::POST, path, query, body)
    }

    fn get_json(&self, path: &str, query: Option<&[(&str, String)]>) -> Result<Value> {
        self.send_json(Method::GET, path, query, None)
    }

    fn parse_merge_request(&self, value: &Value) -> Result<MergeRequest> {
        let id = value
            .get("iid")
            .and_then(|value| value.as_u64())
            .map(|iid| iid.to_string())
            .or_else(|| {
                value
                    .get("id")
                    .and_then(|value| value.as_i64())
                    .map(|id| id.to_string())
            })
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("gitlab MR response missing id"))
            })?;
        let iid = value
            .get("iid")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("gitlab MR response missing iid"))
            })?;
        let title = json_string_field(value, "title")?;
        let description = value
            .get("description")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let source_branch = json_string_field(value, "source_branch")?;
        let target_branch = json_string_field(value, "target_branch")?;
        let url = value
            .get("web_url")
            .and_then(|value| value.as_str())
            .or_else(|| value.get("url").and_then(|value| value.as_str()))
            .unwrap_or_default()
            .to_string();
        let draft = value
            .get("draft")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let state = parse_mr_state(value.get("state").and_then(|value| value.as_str()), draft);

        let labels = value
            .get("labels")
            .and_then(|value| value.as_array())
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| label.as_str())
                    .map(|label| label.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let approvals = value
            .get("reviewers")
            .and_then(|value| value.as_array())
            .map(|reviewers| reviewers.iter().filter_map(parse_user).collect::<Vec<_>>())
            .unwrap_or_default();

        Ok(MergeRequest {
            id,
            iid,
            title,
            description,
            source_branch,
            target_branch,
            state,
            url,
            ci_status: None,
            approvals,
            labels,
        })
    }

    fn parse_issue(&self, value: &Value) -> Result<Issue> {
        let id = value
            .get("iid")
            .and_then(|value| value.as_u64())
            .map(|iid| iid.to_string())
            .or_else(|| {
                value
                    .get("id")
                    .and_then(|value| value.as_i64())
                    .map(|id| id.to_string())
            })
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("gitlab issue response missing id"))
            })?;
        let iid = value
            .get("iid")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!("gitlab issue response missing iid"))
            })?;
        let title = json_string_field(value, "title")?;
        let url = value
            .get("web_url")
            .and_then(|value| value.as_str())
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

    fn parse_reviewer_ids(&self, reviewers: &[String]) -> Result<Vec<u64>> {
        let mut ids = Vec::new();
        for reviewer in reviewers {
            let user = self.get_user(reviewer)?;
            let id = user.id.ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!(format!(
                    "gitlab user '{}' did not include an id",
                    reviewer
                )))
            })?;
            ids.push(id);
        }
        Ok(ids)
    }

    fn parse_mr_iid(&self, mr_id: &MrId) -> Result<u64> {
        mr_id.parse::<u64>().map_err(|_| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "mr id '{}' is not a valid GitLab IID",
                mr_id
            )))
        })
    }

    fn link_pair(&self, current: &(RepoId, MrId), blocking: &(RepoId, MrId)) -> Result<()> {
        let current_project = self.project_path_for_repo(&current.0);
        let blocking_project = self.project_path_for_repo(&blocking.0);
        let current_iid = self.parse_mr_iid(&current.1)?;
        let blocking_iid = self.parse_mr_iid(&blocking.1)?;

        let path = format!(
            "/projects/{}/merge_requests/{}/blocks",
            encode_project_path(&current_project),
            current_iid
        );
        let mut query = vec![("blocking_merge_request_iid", blocking_iid.to_string())];
        if current_project != blocking_project {
            query.push(("blocking_project_id", blocking_project));
        }

        let url = self.api_url(&path);
        let request = self
            .client
            .post(&url)
            .header("PRIVATE-TOKEN", &self.token)
            .header("Accept", "application/json")
            .query(&query);
        let response = request.send().map_err(|err| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "gitlab request failed for {}: {}",
                url, err
            )))
        })?;

        if response.status() == StatusCode::CONFLICT {
            return Ok(());
        }
        parse_json_response(response).map(|_| ())
    }
}

impl Forge for GitLabClient {
    fn create_mr(&self, repo: &RepoId, params: CreateMrParams) -> Result<MergeRequest> {
        let project = self.project_path_for_repo(repo);
        let path = format!("/projects/{}/merge_requests", encode_project_path(&project));
        let title = if params.draft && !params.title.to_ascii_lowercase().starts_with("draft:") {
            format!("Draft: {}", params.title)
        } else {
            params.title
        };

        let reviewer_ids = self.parse_reviewer_ids(&params.reviewers)?;

        let mut payload = serde_json::json!({
            "title": title,
            "description": params.description,
            "source_branch": params.source_branch,
            "target_branch": params.target_branch,
        });

        if let Some(object) = payload.as_object_mut() {
            if !params.labels.is_empty() {
                object.insert("labels".to_string(), Value::String(params.labels.join(",")));
            }
            if !reviewer_ids.is_empty() {
                object.insert(
                    "reviewer_ids".to_string(),
                    Value::Array(
                        reviewer_ids
                            .iter()
                            .map(|id| Value::Number((*id).into()))
                            .collect(),
                    ),
                );
            }
        }

        let response = self.post_json(&path, None, Some(payload))?;
        self.parse_merge_request(&response)
    }

    fn get_mr(&self, repo: &RepoId, mr_id: &MrId) -> Result<MergeRequest> {
        let project = self.project_path_for_repo(repo);
        let iid = self.parse_mr_iid(mr_id)?;
        let path = format!(
            "/projects/{}/merge_requests/{}",
            encode_project_path(&project),
            iid
        );
        let response = self.get_json(&path, None)?;
        self.parse_merge_request(&response)
    }

    fn update_mr(
        &self,
        repo: &RepoId,
        mr_id: &MrId,
        params: UpdateMrParams,
    ) -> Result<MergeRequest> {
        let project = self.project_path_for_repo(repo);
        let iid = self.parse_mr_iid(mr_id)?;
        let path = format!(
            "/projects/{}/merge_requests/{}",
            encode_project_path(&project),
            iid
        );

        let reviewer_ids = match params.reviewers {
            Some(reviewers) => Some(self.parse_reviewer_ids(&reviewers)?),
            None => None,
        };

        let mut values: HashMap<String, Value> = HashMap::new();
        if let Some(title) = params.title {
            values.insert("title".to_string(), Value::String(title));
        }
        if let Some(description) = params.description {
            values.insert("description".to_string(), Value::String(description));
        }
        if let Some(labels) = params.labels {
            values.insert("labels".to_string(), Value::String(labels.join(",")));
        }
        if let Some(reviewer_ids) = reviewer_ids {
            values.insert(
                "reviewer_ids".to_string(),
                Value::Array(
                    reviewer_ids
                        .iter()
                        .map(|id| Value::Number((*id).into()))
                        .collect(),
                ),
            );
        }

        if values.is_empty() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(
                "mr update requires at least one field"
            )));
        }

        let response = self.put_json(
            &path,
            None,
            Some(Value::Object(values.into_iter().collect())),
        )?;
        self.parse_merge_request(&response)
    }

    fn link_mrs(&self, mrs: &[(RepoId, MrId)]) -> Result<()> {
        if mrs.len() < 2 {
            return Ok(());
        }

        for window in mrs.windows(2) {
            if let [blocking, current] = window {
                self.link_pair(current, blocking)?;
            }
        }
        Ok(())
    }

    fn merge_mr(&self, repo: &RepoId, mr_id: &MrId, params: MergeMrParams) -> Result<()> {
        let project = self.project_path_for_repo(repo);
        let iid = self.parse_mr_iid(mr_id)?;
        let path = format!(
            "/projects/{}/merge_requests/{}/merge",
            encode_project_path(&project),
            iid
        );

        let payload = serde_json::json!({
            "squash": params.squash,
            "should_remove_source_branch": params.delete_source_branch,
        });
        self.put_json(&path, None, Some(payload)).map(|_| ())
    }

    fn close_mr(&self, repo: &RepoId, mr_id: &MrId) -> Result<()> {
        let project = self.project_path_for_repo(repo);
        let iid = self.parse_mr_iid(mr_id)?;
        let path = format!(
            "/projects/{}/merge_requests/{}",
            encode_project_path(&project),
            iid
        );

        let payload = serde_json::json!({
            "state_event": "close",
        });
        self.put_json(&path, None, Some(payload)).map(|_| ())
    }

    fn get_ci_status(&self, repo: &RepoId, ref_name: &str) -> Result<CiStatus> {
        let project = self.project_path_for_repo(repo);
        let path = format!("/projects/{}/pipelines", encode_project_path(&project));
        let query = vec![
            ("ref", ref_name.to_string()),
            ("per_page", "20".to_string()),
            ("order_by", "id".to_string()),
            ("sort", "desc".to_string()),
        ];

        let response = self.get_json(&path, Some(&query))?;
        let pipelines_array = response.as_array().ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(
                "gitlab pipelines response was not an array"
            ))
        })?;

        let pipelines = pipelines_array
            .iter()
            .map(|pipeline| Pipeline {
                id: pipeline
                    .get("id")
                    .and_then(|value| value.as_u64())
                    .map(|value| value.to_string())
                    .or_else(|| {
                        pipeline
                            .get("id")
                            .and_then(|value| value.as_i64())
                            .map(|value| value.to_string())
                    })
                    .unwrap_or_default(),
                status: pipeline
                    .get("status")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
            })
            .collect::<Vec<_>>();

        let checks = latest_pipeline_job_checks(
            self,
            &project,
            pipelines_array.first().and_then(pipeline_id_from_value),
        )?;
        let state = aggregate_ci_state(&pipelines);
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
                )));
            }
        };
        let project = self.project_path_for_repo(&project);
        let path = format!("/projects/{}/issues", encode_project_path(&project));

        let mut payload = serde_json::json!({
            "title": params.title,
            "description": params.description,
        });
        if let Some(object) = payload.as_object_mut() {
            if !params.labels.is_empty() {
                object.insert("labels".to_string(), Value::String(params.labels.join(",")));
            }
        }

        let response = self.post_json(&path, None, Some(payload))?;
        self.parse_issue(&response)
    }

    fn get_user(&self, username: &str) -> Result<User> {
        let query = vec![("username", username.to_string())];
        let response = self.get_json("/users", Some(&query))?;
        let users = response.as_array().ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!("gitlab users response was not an array"))
        })?;
        let first = users.first().ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "gitlab user '{}' was not found",
                username
            )))
        })?;

        parse_user(first).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "gitlab user '{}' response missing required fields",
                username
            )))
        })
    }
}

fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

fn parse_json_response(response: reqwest::blocking::Response) -> Result<Value> {
    let status = response.status();
    let body = response.text().map_err(|err| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "failed reading gitlab response body: {}",
            err
        )))
    })?;

    if !status.is_success() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "gitlab API returned {}: {}",
            status,
            body.trim()
        ))));
    }

    if body.trim().is_empty() {
        return Ok(Value::Null);
    }

    serde_json::from_str(&body).map_err(|err| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "failed to parse gitlab response JSON: {}",
            err
        )))
    })
}

fn parse_mr_state(state: Option<&str>, draft: bool) -> MrState {
    if draft {
        return MrState::Draft;
    }
    match state {
        Some("merged") => MrState::Merged,
        Some("closed") => MrState::Closed,
        _ => MrState::Open,
    }
}

fn parse_user(value: &Value) -> Option<User> {
    let username = value.get("username")?.as_str()?.to_string();
    let id = value.get("id").and_then(|value| value.as_u64());
    Some(User { id, username })
}

fn encode_project_path(path: &str) -> String {
    percent_encode(path)
}

fn percent_encode(value: &str) -> String {
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

fn json_string_field(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "gitlab response missing '{}'",
                field
            )))
        })
}

fn aggregate_ci_state(pipelines: &[Pipeline]) -> CiState {
    if pipelines.is_empty() {
        return CiState::Pending;
    }

    if pipelines
        .iter()
        .any(|pipeline| matches!(pipeline.status.as_str(), "running"))
    {
        return CiState::Running;
    }

    if pipelines.iter().any(|pipeline| {
        matches!(
            pipeline.status.as_str(),
            "pending" | "created" | "preparing" | "waiting_for_resource" | "scheduled" | "manual"
        )
    }) {
        return CiState::Pending;
    }

    if pipelines
        .iter()
        .any(|pipeline| matches!(pipeline.status.as_str(), "failed"))
    {
        return CiState::Failed;
    }

    if pipelines
        .iter()
        .any(|pipeline| matches!(pipeline.status.as_str(), "canceled"))
    {
        return CiState::Canceled;
    }

    if pipelines
        .iter()
        .all(|pipeline| matches!(pipeline.status.as_str(), "skipped"))
    {
        return CiState::Skipped;
    }

    CiState::Success
}

fn pipeline_id_from_value(value: &Value) -> Option<u64> {
    value
        .get("id")
        .and_then(|value| value.as_u64())
        .or_else(|| {
            value
                .get("id")
                .and_then(|value| value.as_i64())
                .map(|id| id as u64)
        })
}

fn latest_pipeline_job_checks(
    client: &GitLabClient,
    project: &str,
    pipeline_id: Option<u64>,
) -> Result<Vec<CheckRun>> {
    let Some(pipeline_id) = pipeline_id else {
        return Ok(Vec::new());
    };

    let path = format!(
        "/projects/{}/pipelines/{}/jobs",
        encode_project_path(project),
        pipeline_id
    );
    let query = vec![
        ("per_page", "100".to_string()),
        ("scope[]", "pending".to_string()),
        ("scope[]", "running".to_string()),
        ("scope[]", "success".to_string()),
        ("scope[]", "failed".to_string()),
        ("scope[]", "canceled".to_string()),
        ("scope[]", "skipped".to_string()),
    ];
    let response = client.get_json(&path, Some(&query))?;
    let jobs = response.as_array().ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(
            "gitlab pipeline jobs response was not an array"
        ))
    })?;

    let mut checks = Vec::new();
    for job in jobs {
        let Some(name) = job.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        let status = job
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        checks.push(CheckRun {
            name: name.to_string(),
            status: status.to_string(),
        });
    }
    checks.sort_by(|a, b| a.name.cmp(&b.name));
    checks.dedup_by(|a, b| a.name == b.name && a.status == b.status);
    Ok(checks)
}

#[cfg(test)]
mod tests {
    use crate::core::repo::RepoId;
    use crate::forge::gitlab::{aggregate_ci_state, encode_project_path, GitLabClient};
    use crate::forge::{CiState, Pipeline};

    #[test]
    fn project_path_uses_default_group_when_repo_is_unqualified() {
        let client = GitLabClient::new("gitlab.com", "token", Some("platform".to_string()));
        let path = client.project_path_for_repo(&RepoId::new("service-a"));
        assert_eq!(path, "platform/service-a");
    }

    #[test]
    fn project_path_keeps_qualified_repo() {
        let client = GitLabClient::new("gitlab.com", "token", Some("platform".to_string()));
        let path = client.project_path_for_repo(&RepoId::new("team/service-a"));
        assert_eq!(path, "team/service-a");
    }

    #[test]
    fn encodes_project_path_for_gitlab_routes() {
        assert_eq!(encode_project_path("group/sub/repo"), "group%2Fsub%2Frepo");
    }

    #[test]
    fn aggregates_ci_state_by_priority() {
        let pipelines = vec![
            Pipeline {
                id: "1".to_string(),
                status: "success".to_string(),
            },
            Pipeline {
                id: "2".to_string(),
                status: "running".to_string(),
            },
        ];
        assert_eq!(aggregate_ci_state(&pipelines), CiState::Running);
    }
}
