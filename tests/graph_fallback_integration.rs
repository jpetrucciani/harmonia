use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("graph-fallback");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(root.join("repos")).expect("create repos dir");

        fs::write(
            root.join(".harmonia").join("config.toml"),
            r#"[workspace]
name = "graph-fallback-integration"
repos_dir = "repos"

[repos]
"core" = {}
"app" = {}
"#,
        )
        .expect("write workspace config");

        Self::write_repo(&root, "core", &[]);
        Self::write_repo(&root, "app", &["core"]);

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
        // Intentionally omit [dependencies] config to verify fallback to ecosystem defaults.
        fs::write(
            repo_path.join(".harmonia.toml"),
            format!("[package]\nname = \"{name}\"\necosystem = \"rust\"\n"),
        )
        .expect("write .harmonia.toml");

        init_git_repo(&repo_path);
    }

    fn run_harmonia(&self, args: &[&str]) -> std::process::Output {
        Command::new(harmonia_bin())
            .arg("--workspace")
            .arg(&self.root)
            .args(args)
            .output()
            .expect("run harmonia")
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn harmonia_bin() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_BIN_EXE_harmonia")
            .expect("CARGO_BIN_EXE_harmonia is not set for integration test"),
    )
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
fn graph_deps_uses_ecosystem_default_dependency_file_when_not_configured() {
    let workspace = TestWorkspace::new();
    let output = workspace.run_harmonia(&["graph", "deps", "app", "--json"]);

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "graph deps failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let deps: Vec<String> = serde_json::from_slice(&output.stdout).expect("parse json deps");
    assert_eq!(deps, vec!["core"]);
}
