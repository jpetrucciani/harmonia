use std::path::PathBuf;

#[derive(Debug, Default, Clone)]
pub struct StatusSummary {
    pub staged: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub untracked: Vec<PathBuf>,
    pub conflicts: Vec<PathBuf>,
}

impl StatusSummary {
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty()
            && self.modified.is_empty()
            && self.untracked.is_empty()
            && self.conflicts.is_empty()
    }
}
