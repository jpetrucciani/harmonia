pub mod bitbucket;
pub mod gitea;
pub mod github;
pub mod gitlab;
pub mod traits;

pub type MrId = String;
pub type IssueId = String;

#[derive(Debug, Clone)]
pub struct User {
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct MergeRequest {
    pub id: MrId,
    pub iid: u64,
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
    pub state: MrState,
    pub url: String,
    pub ci_status: Option<CiStatus>,
    pub approvals: Vec<User>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MrState {
    Open,
    Merged,
    Closed,
    Draft,
}

#[derive(Debug, Clone)]
pub struct CiStatus {
    pub state: CiState,
    pub pipelines: Vec<Pipeline>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiState {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub id: IssueId,
    pub iid: u64,
    pub title: String,
    pub url: String,
    pub state: IssueState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueState {
    Open,
    Closed,
}
