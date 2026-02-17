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

impl ViolationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ViolationType::Unsatisfied => "unsatisfied",
            ViolationType::ExactPin => "exact_pin",
            ViolationType::UpperBound => "upper_bound",
            ViolationType::Circular => "circular",
        }
    }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::core::repo::{Dependency, Repo, RepoId};
    use crate::core::version::{Version, VersionKind, VersionReq};
    use crate::ecosystem::EcosystemId;
    use crate::graph::constraint::{check_constraints, ViolationType};
    use crate::graph::DependencyGraph;

    fn repo(id: &str, package_name: &str) -> (RepoId, Repo) {
        let repo_id = RepoId::new(id.to_string());
        (
            repo_id.clone(),
            Repo {
                id: repo_id,
                path: std::path::PathBuf::from(format!("/tmp/{id}")),
                remote_url: String::new(),
                default_branch: "main".to_string(),
                package_name: Some(package_name.to_string()),
                depends_on: Vec::new(),
                ecosystem: Some(EcosystemId::Rust),
                config: None,
                external: false,
                ignored: false,
            },
        )
    }

    #[test]
    fn check_constraints_detects_unsatisfied_exact_and_upper_bound() {
        let mut repos = HashMap::new();
        let (core_id, core_repo) = repo("core", "core");
        repos.insert(core_id.clone(), core_repo);
        let (unsat_id, unsat_repo) = repo("app-unsat", "app-unsat");
        repos.insert(unsat_id.clone(), unsat_repo);
        let (exact_id, exact_repo) = repo("app-exact", "app-exact");
        repos.insert(exact_id.clone(), exact_repo);
        let (upper_id, upper_repo) = repo("app-upper", "app-upper");
        repos.insert(upper_id.clone(), upper_repo);

        let mut graph = DependencyGraph::new();
        graph.edges.insert(
            unsat_id,
            vec![Dependency {
                name: "core".to_string(),
                constraint: VersionReq::new("^2.0.0"),
                is_internal: true,
            }],
        );
        graph.edges.insert(
            exact_id,
            vec![Dependency {
                name: "core".to_string(),
                constraint: VersionReq::new("=1.2.3"),
                is_internal: true,
            }],
        );
        graph.edges.insert(
            upper_id,
            vec![Dependency {
                name: "core".to_string(),
                constraint: VersionReq::new("<2.0.0"),
                is_internal: true,
            }],
        );
        graph.edges.entry(core_id.clone()).or_default();

        let mut versions = HashMap::new();
        versions.insert(core_id, Version::new("1.2.3", VersionKind::Semver));

        let report = check_constraints(&graph, &repos, &versions);
        assert!(report
            .violations
            .iter()
            .any(|violation| violation.violation_type == ViolationType::Unsatisfied));
        assert!(report
            .violations
            .iter()
            .any(|violation| violation.violation_type == ViolationType::ExactPin));
        assert!(report
            .violations
            .iter()
            .any(|violation| violation.violation_type == ViolationType::UpperBound));
    }
}
