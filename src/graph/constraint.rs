use std::collections::HashMap;

use crate::core::repo::{Repo, RepoId};
use crate::core::version::{Version, VersionReq};
use crate::graph::ops::{find_cycles, package_map, resolve_internal_edges, MissingDependency};
use crate::graph::DependencyGraph;

#[derive(Debug, Clone)]
pub struct ConstraintViolation {
    pub from_repo: RepoId,
    pub to_repo: RepoId,
    pub constraint: VersionReq,
    pub actual_version: Version,
    pub violation_type: ViolationType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationType {
    Unsatisfied,
    ExactPin,
    UpperBound,
    Circular,
}

#[derive(Debug, Clone)]
pub struct ConstraintReport {
    pub violations: Vec<ConstraintViolation>,
    pub missing: Vec<MissingDependency>,
    pub cycles: Vec<Vec<RepoId>>,
}

pub fn check_constraints(
    graph: &DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
    versions: &HashMap<RepoId, Version>,
) -> ConstraintReport {
    let resolved = resolve_internal_edges(graph, repos);
    let cycles = find_cycles(graph, repos);
    let map = package_map(repos);
    let mut violations = Vec::new();

    for (from_repo, deps) in &graph.edges {
        for dep in deps {
            if !dep.is_internal {
                continue;
            }
            let target = match map.get(&dep.name) {
                Some(repo) => repo,
                None => continue,
            };
            let actual = match versions.get(target) {
                Some(version) => version,
                None => continue,
            };
            let req = match dep.constraint.semver.as_ref() {
                Some(req) => req,
                None => continue,
            };
            let actual_semver = match actual.semver.as_ref() {
                Some(actual) => actual,
                None => continue,
            };

            if !req.matches(actual_semver) {
                violations.push(ConstraintViolation {
                    from_repo: from_repo.clone(),
                    to_repo: target.clone(),
                    constraint: dep.constraint.clone(),
                    actual_version: actual.clone(),
                    violation_type: ViolationType::Unsatisfied,
                });
                continue;
            }

            if is_exact_pin(req) {
                violations.push(ConstraintViolation {
                    from_repo: from_repo.clone(),
                    to_repo: target.clone(),
                    constraint: dep.constraint.clone(),
                    actual_version: actual.clone(),
                    violation_type: ViolationType::ExactPin,
                });
            }

            if has_upper_bound(req) {
                violations.push(ConstraintViolation {
                    from_repo: from_repo.clone(),
                    to_repo: target.clone(),
                    constraint: dep.constraint.clone(),
                    actual_version: actual.clone(),
                    violation_type: ViolationType::UpperBound,
                });
            }
        }
    }

    ConstraintReport {
        violations,
        missing: resolved.missing,
        cycles,
    }
}

pub fn validate_bump(
    graph: &DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
    _versions: &HashMap<RepoId, Version>,
    repo: &RepoId,
    new_version: &Version,
) -> Vec<ConstraintViolation> {
    let map = package_map(repos);
    let package_name = map
        .iter()
        .find_map(|(name, id)| if id == repo { Some(name.clone()) } else { None });
    let package_name = match package_name {
        Some(name) => name,
        None => return Vec::new(),
    };
    let mut violations = Vec::new();

    for (from_repo, deps) in &graph.edges {
        for dep in deps {
            if !dep.is_internal || dep.name != package_name {
                continue;
            }
            let req = match dep.constraint.semver.as_ref() {
                Some(req) => req,
                None => continue,
            };
            let new_semver = match new_version.semver.as_ref() {
                Some(version) => version,
                None => continue,
            };

            if !req.matches(new_semver) {
                violations.push(ConstraintViolation {
                    from_repo: from_repo.clone(),
                    to_repo: repo.clone(),
                    constraint: dep.constraint.clone(),
                    actual_version: new_version.clone(),
                    violation_type: ViolationType::Unsatisfied,
                });
                continue;
            }

            if is_exact_pin(req) {
                violations.push(ConstraintViolation {
                    from_repo: from_repo.clone(),
                    to_repo: repo.clone(),
                    constraint: dep.constraint.clone(),
                    actual_version: new_version.clone(),
                    violation_type: ViolationType::ExactPin,
                });
            }

            if has_upper_bound(req) {
                violations.push(ConstraintViolation {
                    from_repo: from_repo.clone(),
                    to_repo: repo.clone(),
                    constraint: dep.constraint.clone(),
                    actual_version: new_version.clone(),
                    violation_type: ViolationType::UpperBound,
                });
            }
        }
    }

    violations
}

fn is_exact_pin(req: &semver::VersionReq) -> bool {
    if req.comparators.len() != 1 {
        return false;
    }
    let comp = &req.comparators[0];
    if comp.op != semver::Op::Exact {
        return false;
    }
    comp.minor.is_some() && comp.patch.is_some()
}

fn has_upper_bound(req: &semver::VersionReq) -> bool {
    req.comparators
        .iter()
        .any(|comp| matches!(comp.op, semver::Op::Less | semver::Op::LessEq))
}
