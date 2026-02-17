use crate::core::repo::RepoId;
use crate::error::Result;
use crate::forge::{CiStatus, Issue, MergeRequest, MrId, User};

#[derive(Debug, Clone, Default)]
pub struct CreateMrParams {
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
    pub draft: bool,
    pub labels: Vec<String>,
    pub reviewers: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateMrParams {
    pub title: Option<String>,
    pub description: Option<String>,
    pub labels: Option<Vec<String>>,
    pub reviewers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct MergeMrParams {
    pub squash: bool,
    pub delete_source_branch: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CreateIssueParams {
    pub project: Option<RepoId>,
    pub title: String,
    pub description: String,
    pub labels: Vec<String>,
}

pub trait Forge: Send + Sync {
    fn create_mr(&self, repo: &RepoId, params: CreateMrParams) -> Result<MergeRequest>;

    fn get_mr(&self, repo: &RepoId, mr_id: &MrId) -> Result<MergeRequest>;

    fn update_mr(
        &self,
        repo: &RepoId,
        mr_id: &MrId,
        params: UpdateMrParams,
    ) -> Result<MergeRequest>;

    fn link_mrs(&self, mrs: &[(RepoId, MrId)]) -> Result<()>;

    fn merge_mr(&self, repo: &RepoId, mr_id: &MrId, params: MergeMrParams) -> Result<()>;

    fn close_mr(&self, repo: &RepoId, mr_id: &MrId) -> Result<()>;

    fn get_ci_status(&self, repo: &RepoId, ref_name: &str) -> Result<CiStatus>;

    fn create_issue(&self, params: CreateIssueParams) -> Result<Issue>;

    fn get_user(&self, username: &str) -> Result<User>;
}
