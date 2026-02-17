use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("graph-order");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(root.join("repos")).expect("create repos dir");

        fs::write(
            root.join(".harmonia").join("config.toml"),
            r#"[workspace]
name = "graph-order-integration"
repos_dir = "repos"

[repos]
"core" = {}
"lib" = {}
"app" = {}
"#,
        )
        .expect("write workspace config");

        Self::write_repo(&root, "core", &[]);
        Self::write_repo(&root, "lib", &["core"]);
        Self::write_repo(&root, "app", &["lib"]);

        Self { root }
    }

    fn new_with_workspace_declared_dependencies() -> Self {
        let root = unique_temp_dir("graph-order-workspace-deps");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(root.join("repos")).expect("create repos dir");

        fs::write(
            root.join(".harmonia").join("config.toml"),
            r#"[workspace]
name = "graph-order-workspace-deps"
repos_dir = "repos"

[repos]
"core" = {}
"lib" = { depends_on = ["core"] }
"app" = { depends_on = ["lib"] }
"#,
        )
        .expect("write workspace config");

        for repo in ["core", "lib", "app"] {
            fs::create_dir_all(root.join("repos").join(repo)).expect("create repo dir");
        }

        Self { root }
    }

    fn write_repo(root: &Path, name: &str, deps: &[&str]) {
        let repo_path = root.join("repos").join(name);
        fs::create_dir_all(repo_path.join("src")).expect("create repo src dir");

        let dependency_lines = deps
            .iter()
            .map(|dep| format!(r#"{dep} = "0.1.0""#))
            .collect::<Vec<_>>()
            .join("\n");

        let cargo = if dependency_lines.is_empty() {
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
            )
        } else {
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{dependency_lines}\n"
            )
        };

        fs::write(repo_path.join("Cargo.toml"), cargo).expect("write Cargo.toml");
        fs::write(
            repo_path.join("src").join("lib.rs"),
            format!("pub fn name() -> &'static str {{ \"{name}\" }}\n"),
        )
        .expect("write src/lib.rs");
        fs::write(
            repo_path.join(".harmonia.toml"),
            format!(
                "[package]\nname = \"{name}\"\necosystem = \"rust\"\n\n[dependencies]\nfile = \"Cargo.toml\"\n"
            ),
        )
        .expect("write .harmonia.toml");

        init_git_repo(&repo_path);
    }

    fn mark_repo_changed(&self, repo: &str) {
        let changed_path = self.root.join("repos").join(repo).join("CHANGED.md");
        fs::write(changed_path, "changed\n").expect("write changed marker");
    }

    fn graph_order(&self, changed: bool) -> Vec<String> {
        let mut cmd = Command::new(harmonia_bin());
        cmd.arg("--workspace")
            .arg(&self.root)
            .arg("graph")
            .arg("order")
            .arg("--json");
        if changed {
            cmd.arg("--changed");
        }

        let output = cmd.output().expect("run harmonia graph order");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "graph order command failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );

        serde_json::from_slice(&output.stdout).expect("parse graph order json")
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn harmonia_bin() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_harmonia") {
        return PathBuf::from(path);
    }

    let current_exe = std::env::current_exe().expect("resolve current test binary path");
    let target_dir = current_exe
        .parent()
        .and_then(|path| path.parent())
        .expect("derive cargo target dir from test binary path");
    let bin_name = if cfg!(windows) {
        "harmonia.exe"
    } else {
        "harmonia"
    };
    let fallback = target_dir.join(bin_name);

    if fallback.is_file() {
        fallback
    } else {
        panic!(
            "CARGO_BIN_EXE_harmonia is not set and fallback binary not found at {}",
            fallback.display()
        );
    }
}

fn init_git_repo(repo_path: &Path) {
    run_git(repo_path, &["init", "--quiet"]);
    run_git(repo_path, &["config", "user.name", "Harmonia Test"]);
    run_git(
        repo_path,
        &["config", "user.email", "harmonia-test@example.com"],
    );
    run_git(repo_path, &["add", "-A"]);
    run_git(repo_path, &["commit", "--quiet", "-m", "Initial commit"]);
}

fn run_git(repo_path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(args)
        .output()
        .expect("run git command");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "git command failed in {}: git {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        repo_path.display(),
        args.join(" ")
    );
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("harmonia-{prefix}-{pid}-{nanos}"))
}

#[test]
fn graph_order_is_dependency_first_for_workspace() {
    let workspace = TestWorkspace::new();
    let order = workspace.graph_order(false);
    assert_eq!(order, vec!["core", "lib", "app"]);
}

#[test]
fn graph_order_changed_uses_dependency_first_merge_order() {
    let workspace = TestWorkspace::new();
    workspace.mark_repo_changed("app");
    let order = workspace.graph_order(true);
    assert_eq!(order, vec!["core", "lib", "app"]);
}

#[test]
fn graph_order_uses_workspace_declared_dependencies() {
    let workspace = TestWorkspace::new_with_workspace_declared_dependencies();
    let order = workspace.graph_order(false);
    assert_eq!(order, vec!["core", "lib", "app"]);
}
