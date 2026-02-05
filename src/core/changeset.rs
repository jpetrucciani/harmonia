use std::collections::HashMap;

use crate::core::repo::RepoId;
use crate::forge::{Issue, MergeRequest};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChangesetId(String);

impl ChangesetId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

#[derive(Debug, Clone)]
pub struct Changeset {
    pub id: ChangesetId,
    pub branch: String,
    pub repos: Vec<RepoId>,
    pub merge_order: Vec<RepoId>,
    pub mrs: HashMap<RepoId, MergeRequest>,
    pub tracking_issue: Option<Issue>,
}
