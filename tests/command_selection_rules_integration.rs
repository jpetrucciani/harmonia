use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("command-selection");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(root.join("repos")).expect("create repos dir");

        fs::write(
            root.join(".harmonia").join("config.toml"),
            r#"[workspace]
name = "command-selection-integration"
repos_dir = "repos"

[repos]
"core" = {}
"app" = {}
"external-sdk" = { external = true }
"scratch" = { ignored = true }

[groups]
core_group = ["core"]
default = "core_group"
"#,
        )
        .expect("write workspace config");

        Self::write_repo(&root, "core");
        Self::write_repo(&root, "app");
        Self::write_repo(&root, "external-sdk");
        Self::write_repo(&root, "scratch");

        Self { root }
    }

    fn write_repo(root: &Path, name: &str) {
        let repo_path = root.join("repos").join(name);
        fs::create_dir_all(repo_path.join("src")).expect("create repo src dir");
        fs::write(
            repo_path.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
            ),
        )
        .expect("write Cargo.toml");
        fs::write(repo_path.join("src").join("lib.rs"), "pub fn marker() {}\n")
            .expect("write src/lib.rs");
        fs::write(
            repo_path.join(".harmonia.toml"),
            format!(
                "[package]\nname = \"{name}\"\necosystem = \"rust\"\n\n[hooks.custom]\ntest = \"echo {name} >> ../../selected.log\"\nlint = \"echo {name} >> ../../selected.log\"\n"
            ),
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

    fn clear_selection_log(&self) {
        let log = self.root.join("selected.log");
        let _ = fs::remove_file(log);
    }

    fn read_selection_log(&self) -> Vec<String> {
        let path = self.root.join("selected.log");
        let contents = fs::read_to_string(path).unwrap_or_default();
        let mut lines: Vec<String> = contents
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect();
        lines.sort();
        lines
    }

    fn mark_repo_changed(&self, repo: &str) {
        let file = self.root.join("repos").join(repo).join("CHANGED.md");
        fs::write(file, "changed\n").expect("write changed marker");
    }

    fn current_branch(&self, repo: &str) -> String {
        let repo_path = self.root.join("repos").join(repo);
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .expect("run git rev-parse");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "git rev-parse failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        stdout.trim().to_string()
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
    run_git(repo_path, &["branch", "-M", "main"]);
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

fn assert_success(output: &std::process::Output, context: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn test_all_excludes_external_and_ignored_repos() {
    let workspace = TestWorkspace::new();
    workspace.clear_selection_log();

    let output = workspace.run_harmonia(&["test", "--all", "--parallel", "1"]);
    assert_success(&output, "test --all");
    assert_eq!(
        workspace.read_selection_log(),
        vec!["app".to_string(), "core".to_string()]
    );
}

#[test]
fn test_changed_targets_only_changed_repos() {
    let workspace = TestWorkspace::new();
    workspace.mark_repo_changed("app");
    workspace.clear_selection_log();

    let output = workspace.run_harmonia(&["test", "--changed", "--parallel", "1"]);
    assert_success(&output, "test --changed");
    assert_eq!(workspace.read_selection_log(), vec!["app".to_string()]);
}

#[test]
fn lint_changed_targets_only_changed_repos() {
    let workspace = TestWorkspace::new();
    workspace.mark_repo_changed("app");
    workspace.clear_selection_log();

    let output = workspace.run_harmonia(&["lint", "--changed", "--parallel", "1"]);
    assert_success(&output, "lint --changed");
    assert_eq!(workspace.read_selection_log(), vec!["app".to_string()]);
}

#[test]
fn branch_without_explicit_repos_uses_default_group() {
    let workspace = TestWorkspace::new();

    let output = workspace.run_harmonia(&["branch", "feature/default-group", "--create"]);
    assert_success(&output, "branch default group");

    assert_eq!(workspace.current_branch("core"), "feature/default-group");
    assert_eq!(workspace.current_branch("app"), "main");
    assert_eq!(workspace.current_branch("external-sdk"), "main");
    assert_eq!(workspace.current_branch("scratch"), "main");
}
