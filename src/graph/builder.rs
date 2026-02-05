use crate::core::repo::{Dependency, Repo, RepoId};
use crate::ecosystem::plugin_for;
use crate::error::Result;
use crate::graph::DependencyGraph;
use std::collections::HashMap;

pub fn build_graph(repos: &HashMap<RepoId, Repo>) -> Result<DependencyGraph> {
    let mut edges: HashMap<RepoId, Vec<Dependency>> = HashMap::new();

    let mut package_map: HashMap<String, RepoId> = HashMap::new();
    for (id, repo) in repos {
        let name = repo
            .package_name
            .clone()
            .unwrap_or_else(|| id.as_str().to_string());
        package_map.insert(name, id.clone());
    }

    for (id, repo) in repos {
        if repo.ignored {
            continue;
        }
        let deps = match repo
            .config
            .as_ref()
            .and_then(|cfg| cfg.dependencies.as_ref())
        {
            Some(deps_cfg) => {
                let file = match deps_cfg.file.as_ref() {
                    Some(file) => file,
                    None => {
                        edges.insert(id.clone(), Vec::new());
                        continue;
                    }
                };
                let path = repo.path.join(file);
                if !path.is_file() {
                    edges.insert(id.clone(), Vec::new());
                    continue;
                }
                let content = std::fs::read_to_string(&path)?;
                let ecosystem = match repo.ecosystem.as_ref() {
                    Some(id) => id,
                    None => {
                        edges.insert(id.clone(), Vec::new());
                        continue;
                    }
                };
                let plugin = plugin_for(ecosystem);
                let mut parsed = plugin.parse_dependencies(&path, &content)?;
                let internal_packages = deps_cfg
                    .internal_packages
                    .as_ref()
                    .map(|list| list.to_vec())
                    .unwrap_or_default();
                let internal_pattern = deps_cfg
                    .internal_pattern
                    .as_ref()
                    .and_then(|pat| regex::Regex::new(pat).ok());
                for dep in &mut parsed {
                    let is_internal = internal_packages.contains(&dep.name)
                        || internal_pattern
                            .as_ref()
                            .map(|re| re.is_match(&dep.name))
                            .unwrap_or(false)
                        || package_map.contains_key(&dep.name);
                    dep.is_internal = is_internal;
                }
                parsed
            }
            None => Vec::new(),
        };
        edges.insert(id.clone(), deps);
    }

    Ok(DependencyGraph { edges })
}
