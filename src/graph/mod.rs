use std::collections::HashMap;

use crate::core::repo::{Dependency, RepoId};

pub mod builder;
pub mod constraint;
pub mod ops;
pub mod viz;

#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<RepoId, Vec<Dependency>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }
}
