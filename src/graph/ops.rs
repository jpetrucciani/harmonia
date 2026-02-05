use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{anyhow, Result};

use crate::core::repo::{Dependency, Repo, RepoId};
use crate::graph::DependencyGraph;

#[derive(Debug, Clone)]
pub struct MissingDependency {
    pub from: RepoId,
    pub dependency: Dependency,
}

#[derive(Debug, Clone)]
pub struct ResolvedGraph {
    pub edges: HashMap<RepoId, Vec<RepoId>>,
    pub missing: Vec<MissingDependency>,
}

pub fn dependencies_for(graph: &DependencyGraph, repo: &RepoId) -> Vec<Dependency> {
    graph.edges.get(repo).cloned().unwrap_or_else(Vec::new)
}

pub fn internal_dependencies_for(graph: &DependencyGraph, repo: &RepoId) -> Vec<Dependency> {
    dependencies_for(graph, repo)
        .into_iter()
        .filter(|dep| dep.is_internal)
        .collect()
}

pub fn dependents_of(graph: &DependencyGraph, package: &str) -> Vec<RepoId> {
    graph
        .edges
        .iter()
        .filter(|(_, deps)| {
            deps.iter()
                .any(|dep| dep.is_internal && dep.name == package)
        })
        .map(|(repo, _)| repo.clone())
        .collect()
}

pub fn package_map(repos: &HashMap<RepoId, Repo>) -> HashMap<String, RepoId> {
    let mut map = HashMap::new();
    for (id, repo) in repos {
        let name = repo
            .package_name
            .clone()
            .unwrap_or_else(|| id.as_str().to_string());
        map.insert(name, id.clone());
    }
    map
}

pub fn resolve_internal_edges(
    graph: &DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
) -> ResolvedGraph {
    let map = package_map(repos);
    let mut edges = HashMap::new();
    let mut missing = Vec::new();

    for (repo_id, deps) in &graph.edges {
        let mut internal = Vec::new();
        for dep in deps {
            if !dep.is_internal {
                continue;
            }
            if let Some(target) = map.get(&dep.name) {
                internal.push(target.clone());
            } else {
                missing.push(MissingDependency {
                    from: repo_id.clone(),
                    dependency: dep.clone(),
                });
            }
        }
        edges.insert(repo_id.clone(), internal);
    }

    ResolvedGraph { edges, missing }
}

pub fn transitive_dependencies(
    graph: &DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
    repo: &RepoId,
) -> Vec<RepoId> {
    let resolved = resolve_internal_edges(graph, repos);
    let mut seen = HashSet::new();
    let mut stack = Vec::new();
    if let Some(deps) = resolved.edges.get(repo) {
        for dep in deps {
            stack.push(dep.clone());
        }
    }
    while let Some(current) = stack.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        if let Some(next) = resolved.edges.get(&current) {
            for dep in next {
                stack.push(dep.clone());
            }
        }
    }
    let mut out: Vec<_> = seen.into_iter().collect();
    out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    out
}

pub fn transitive_dependents(
    graph: &DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
    repo: &RepoId,
) -> Vec<RepoId> {
    let resolved = resolve_internal_edges(graph, repos);
    let mut reverse: HashMap<RepoId, Vec<RepoId>> = HashMap::new();
    for (from, deps) in &resolved.edges {
        for dep in deps {
            reverse.entry(dep.clone()).or_default().push(from.clone());
        }
    }

    let mut seen = HashSet::new();
    let mut stack = Vec::new();
    if let Some(deps) = reverse.get(repo) {
        for dep in deps {
            stack.push(dep.clone());
        }
    }
    while let Some(current) = stack.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        if let Some(next) = reverse.get(&current) {
            for dep in next {
                stack.push(dep.clone());
            }
        }
    }
    let mut out: Vec<_> = seen.into_iter().collect();
    out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    out
}

pub fn topological_order(
    graph: &DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
) -> Result<Vec<RepoId>> {
    let resolved = resolve_internal_edges(graph, repos);
    topological_order_with_nodes(&resolved.edges, resolved.edges.keys().cloned().collect())
}

pub fn merge_order(
    graph: &DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
    targets: &[RepoId],
) -> Result<Vec<RepoId>> {
    let resolved = resolve_internal_edges(graph, repos);
    let mut nodes: HashSet<RepoId> = HashSet::new();
    for repo in targets {
        nodes.insert(repo.clone());
        for dep in transitive_dependencies(graph, repos, repo) {
            nodes.insert(dep);
        }
    }
    topological_order_with_nodes(&resolved.edges, nodes)
}

pub fn find_cycles(graph: &DependencyGraph, repos: &HashMap<RepoId, Repo>) -> Vec<Vec<RepoId>> {
    let resolved = resolve_internal_edges(graph, repos);
    let mut state: HashMap<RepoId, VisitState> = HashMap::new();
    let mut stack: Vec<RepoId> = Vec::new();
    let mut cycles = Vec::new();

    for node in resolved.edges.keys() {
        if state.contains_key(node) {
            continue;
        }
        visit_node(node, &resolved.edges, &mut state, &mut stack, &mut cycles);
    }

    cycles
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Visited,
}

fn visit_node(
    node: &RepoId,
    edges: &HashMap<RepoId, Vec<RepoId>>,
    state: &mut HashMap<RepoId, VisitState>,
    stack: &mut Vec<RepoId>,
    cycles: &mut Vec<Vec<RepoId>>,
) {
    if let Some(existing) = state.get(node) {
        if *existing == VisitState::Visiting {
            if let Some(pos) = stack.iter().position(|id| id == node) {
                cycles.push(stack[pos..].to_vec());
            }
        }
        return;
    }

    state.insert(node.clone(), VisitState::Visiting);
    stack.push(node.clone());
    if let Some(deps) = edges.get(node) {
        for dep in deps {
            visit_node(dep, edges, state, stack, cycles);
        }
    }
    stack.pop();
    state.insert(node.clone(), VisitState::Visited);
}

fn topological_order_with_nodes(
    edges: &HashMap<RepoId, Vec<RepoId>>,
    nodes: HashSet<RepoId>,
) -> Result<Vec<RepoId>> {
    let mut indegree: HashMap<RepoId, usize> = HashMap::new();
    let mut adjacency: HashMap<RepoId, Vec<RepoId>> = HashMap::new();

    for node in nodes.iter() {
        indegree.entry(node.clone()).or_insert(0);
        adjacency.entry(node.clone()).or_default();
    }

    for (from, deps) in edges {
        if !nodes.contains(from) {
            continue;
        }
        for dep in deps {
            if !nodes.contains(dep) {
                continue;
            }
            adjacency.entry(from.clone()).or_default().push(dep.clone());
            let entry = indegree.entry(dep.clone()).or_insert(0);
            *entry += 1;
        }
    }

    let mut queue: VecDeque<RepoId> = indegree
        .iter()
        .filter_map(|(node, &count)| if count == 0 { Some(node.clone()) } else { None })
        .collect();
    let mut order = Vec::new();

    while let Some(node) = queue.pop_front() {
        order.push(node.clone());
        if let Some(deps) = adjacency.get(&node) {
            for dep in deps {
                if let Some(count) = indegree.get_mut(dep) {
                    if *count > 0 {
                        *count -= 1;
                        if *count == 0 {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }
        }
    }

    if order.len() != nodes.len() {
        return Err(anyhow!("cycle detected in dependency graph"));
    }

    Ok(order)
}
