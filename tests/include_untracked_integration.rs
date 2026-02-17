use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
    repo_path: PathBuf,
}

impl TestWorkspace {
    fn new(include_untracked: bool) -> Self {
        let root = unique_temp_dir("include-untracked");
        let repo_path = root.join("repos").join("service");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(&repo_path).expect("create repo path");

        fs::write(
            root.join(".harmonia").join("config.toml"),
            format!(
                "[workspace]\nname = \"include-untracked-integration\"\nrepos_dir = \"repos\"\n\n[repos]\n\"service\" = {{}}\n\n[defaults]\ninclude_untracked = {}\n",
                include_untracked
            ),
        )
        .expect("write workspace config");

        init_git_repo(&repo_path);

        Self { root, repo_path }
    }

    fn mark_untracked_file(&self) {
        fs::write(self.repo_path.join("UNTRACKED.txt"), "untracked\n").expect("write untracked");
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
    fs::write(repo_path.join("README.md"), "# service\n").expect("write README");
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

fn assert_success(output: &std::process::Output, context: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn include_untracked_false_excludes_untracked_from_status_and_diff() {
    let workspace = TestWorkspace::new(false);
    workspace.mark_untracked_file();

    let status_output = workspace.run_harmonia(&["status", "--json"]);
    assert_success(&status_output, "status --json");
    let status_json: serde_json::Value =
        serde_json::from_slice(&status_output.stdout).expect("parse status json");
    let first = status_json
        .as_array()
        .and_then(|rows| rows.first())
        .expect("status json has first row");
    let untracked = first
        .get("untracked")
        .and_then(|value| value.as_u64())
        .expect("untracked count");
    assert_eq!(untracked, 0);

    let diff_output = workspace.run_harmonia(&["diff", "service", "--format", "json"]);
    assert_success(&diff_output, "diff --format json");
    let diff_json: serde_json::Value =
        serde_json::from_slice(&diff_output.stdout).expect("parse diff json");
    let files = diff_json
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("files"))
        .and_then(|value| value.as_array())
        .expect("diff files");
    assert!(
        files
            .iter()
            .all(|value| value.as_str() != Some("UNTRACKED.txt")),
        "untracked file should be excluded: {files:?}"
    );
}

#[test]
fn include_untracked_true_includes_untracked_in_diff_json() {
    let workspace = TestWorkspace::new(true);
    workspace.mark_untracked_file();

    let diff_output = workspace.run_harmonia(&["diff", "service", "--format", "json"]);
    assert_success(&diff_output, "diff --format json");
    let diff_json: serde_json::Value =
        serde_json::from_slice(&diff_output.stdout).expect("parse diff json");
    let files = diff_json
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("files"))
        .and_then(|value| value.as_array())
        .expect("diff files");
    assert!(
        files
            .iter()
            .any(|value| value.as_str() == Some("UNTRACKED.txt")),
        "untracked file should be present: {files:?}"
    );
}
