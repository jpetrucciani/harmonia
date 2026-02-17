use crate::core::repo::{Dependency, Repo, RepoId};
use crate::core::version::VersionReq;
use crate::ecosystem::plugin_for;
use crate::error::Result;
use crate::graph::DependencyGraph;
use std::collections::{HashMap, HashSet};

pub fn build_graph(repos: &HashMap<RepoId, Repo>) -> Result<DependencyGraph> {
    let mut edges: HashMap<RepoId, Vec<Dependency>> = HashMap::new();

    let mut package_map: HashMap<String, RepoId> = HashMap::new();
    let mut repo_name_map: HashMap<String, String> = HashMap::new();
    for (id, repo) in repos {
        let name = repo
            .package_name
            .clone()
            .unwrap_or_else(|| id.as_str().to_string());
        package_map.insert(name.clone(), id.clone());
        repo_name_map.insert(id.as_str().to_string(), name);
    }

    for (id, repo) in repos {
        if repo.ignored {
            continue;
        }
        let deps = parse_repo_dependencies(repo, &package_map, &repo_name_map)?;
        edges.insert(id.clone(), deps);
    }

    Ok(DependencyGraph { edges })
}

fn parse_repo_dependencies(
    repo: &Repo,
    package_map: &HashMap<String, RepoId>,
    repo_name_map: &HashMap<String, String>,
) -> Result<Vec<Dependency>> {
    let deps_cfg = repo
        .config
        .as_ref()
        .and_then(|cfg| cfg.dependencies.as_ref());
    let mut parsed = Vec::new();

    if let Some(ecosystem) = repo.ecosystem.as_ref() {
        let path = dependency_file_for_repo(repo, deps_cfg, ecosystem);
        if let Some(path) = path.filter(|path| path.is_file()) {
            let content = std::fs::read_to_string(&path)?;
            let plugin = plugin_for(ecosystem);
            parsed = plugin.parse_dependencies(&path, &content)?;
        }
    }

    let internal_packages = deps_cfg
        .and_then(|cfg| cfg.internal_packages.as_ref())
        .map(|list| list.to_vec())
        .unwrap_or_default();
    let internal_pattern = deps_cfg
        .and_then(|cfg| cfg.internal_pattern.as_ref())
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

    append_workspace_declared_dependencies(repo, &mut parsed, package_map, repo_name_map);

    Ok(parsed)
}

fn append_workspace_declared_dependencies(
    repo: &Repo,
    parsed: &mut Vec<Dependency>,
    package_map: &HashMap<String, RepoId>,
    repo_name_map: &HashMap<String, String>,
) {
    let mut existing: HashSet<String> = parsed.iter().map(|dep| dep.name.clone()).collect();
    for declared in &repo.depends_on {
        let normalized = normalize_declared_dependency(declared, package_map, repo_name_map);
        if existing.contains(&normalized) {
            continue;
        }
        parsed.push(Dependency {
            name: normalized.clone(),
            constraint: VersionReq::new("*"),
            is_internal: true,
        });
        existing.insert(normalized);
    }
}

fn normalize_declared_dependency(
    declared: &str,
    package_map: &HashMap<String, RepoId>,
    repo_name_map: &HashMap<String, String>,
) -> String {
    if package_map.contains_key(declared) {
        return declared.to_string();
    }
    if let Some(name) = repo_name_map.get(declared) {
        return name.clone();
    }
    declared.to_string()
}

fn dependency_file_for_repo(
    repo: &Repo,
    deps_cfg: Option<&crate::config::DepsConfig>,
    ecosystem: &crate::ecosystem::EcosystemId,
) -> Option<std::path::PathBuf> {
    if let Some(configured_file) = deps_cfg.and_then(|cfg| cfg.file.as_ref()) {
        return Some(repo.path.join(configured_file));
    }

    let plugin = plugin_for(ecosystem);
    for pattern in plugin.file_patterns() {
        let candidate = repo.path.join(pattern);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::core::repo::{Repo, RepoId};
    use crate::ecosystem::EcosystemId;
    use crate::graph::builder::build_graph;

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let pid = std::process::id();
        std::env::temp_dir().join(format!("harmonia-{prefix}-{pid}-{nanos}"))
    }

    fn mk_repo(
        id: &str,
        path: std::path::PathBuf,
        package_name: &str,
        ecosystem: EcosystemId,
        depends_on: Vec<&str>,
    ) -> (RepoId, Repo) {
        let repo_id = RepoId::new(id.to_string());
        (
            repo_id.clone(),
            Repo {
                id: repo_id,
                path,
                remote_url: String::new(),
                default_branch: "main".to_string(),
                package_name: Some(package_name.to_string()),
                depends_on: depends_on.into_iter().map(str::to_string).collect(),
                ecosystem: Some(ecosystem),
                config: None,
                external: false,
                ignored: false,
            },
        )
    }

    #[test]
    fn build_graph_marks_internal_dependencies_from_package_map() {
        let root = unique_temp_dir("graph-builder");
        fs::create_dir_all(root.join("core")).expect("create core dir");
        fs::create_dir_all(root.join("app")).expect("create app dir");

        fs::write(
            root.join("core").join("Cargo.toml"),
            "[package]\nname = \"core\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
        )
        .expect("write core Cargo.toml");
        fs::write(
            root.join("app").join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ncore = \"^0.1\"\nserde = \"1\"\n",
        )
        .expect("write app Cargo.toml");

        let mut repos = HashMap::new();
        let (core_id, core_repo) = mk_repo(
            "core",
            root.join("core"),
            "core-package",
            EcosystemId::Rust,
            Vec::new(),
        );
        repos.insert(core_id, core_repo);
        let (app_id, app_repo) = mk_repo(
            "app",
            root.join("app"),
            "app",
            EcosystemId::Rust,
            Vec::new(),
        );
        repos.insert(app_id.clone(), app_repo);

        let graph = build_graph(&repos).expect("build graph");
        let app_deps = graph.edges.get(&app_id).expect("app deps");
        let core_dep = app_deps
            .iter()
            .find(|dep| dep.name == "core")
            .expect("core dependency");
        let serde_dep = app_deps
            .iter()
            .find(|dep| dep.name == "serde")
            .expect("serde dependency");
        assert!(core_dep.is_internal);
        assert!(!serde_dep.is_internal);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn build_graph_includes_workspace_declared_dependencies() {
        let root = unique_temp_dir("graph-builder-workspace-deps");
        fs::create_dir_all(root.join("core")).expect("create core dir");
        fs::create_dir_all(root.join("api")).expect("create api dir");

        let mut repos = HashMap::new();
        let (core_id, core_repo) = mk_repo(
            "core",
            root.join("core"),
            "core-package",
            EcosystemId::Rust,
            Vec::new(),
        );
        repos.insert(core_id, core_repo);
        let (api_id, api_repo) = mk_repo(
            "api",
            root.join("api"),
            "service-api",
            EcosystemId::Rust,
            vec!["core"],
        );
        repos.insert(api_id.clone(), api_repo);

        let graph = build_graph(&repos).expect("build graph");
        let api_deps = graph.edges.get(&api_id).expect("api deps");
        assert!(
            api_deps
                .iter()
                .any(|dep| dep.name == "core-package" && dep.is_internal),
            "workspace declared depends_on should create internal edge"
        );

        let _ = fs::remove_dir_all(root);
    }
}
