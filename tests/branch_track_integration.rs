use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
    repo_path: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("branch-track");
        let harmonia_dir = root.join(".harmonia");
        let repo_path = root.join("repos").join("service");

        fs::create_dir_all(&harmonia_dir).expect("create .harmonia");
        fs::create_dir_all(&repo_path).expect("create repo path");

        fs::write(
            harmonia_dir.join("config.toml"),
            r#"[workspace]
name = "branch-track-integration"
repos_dir = "repos"

[repos]
"service" = {}
"#,
        )
        .expect("write workspace config");

        init_repo(&repo_path);

        Self { root, repo_path }
    }

    fn run_branch_create_with_track(&self, branch: &str, track: &str) {
        let output = Command::new(harmonia_bin())
            .arg("--workspace")
            .arg(&self.root)
            .arg("branch")
            .arg(branch)
            .arg("--create")
            .arg("--repos")
            .arg("service")
            .arg("--track")
            .arg(track)
            .output()
            .expect("run harmonia branch");

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "branch command failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }

    fn git_stdout(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(&self.repo_path)
            .args(args)
            .output()
            .expect("run git command");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "git command failed: git {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            args.join(" ")
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

fn init_repo(repo_path: &Path) {
    fs::write(repo_path.join("README.md"), "# service\n").expect("write README");
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

#[test]
fn branch_track_sets_upstream_branch() {
    let workspace = TestWorkspace::new();
    workspace.run_branch_create_with_track("feature/tracked", "main");
    assert_eq!(
        workspace.git_stdout(&["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/tracked"
    );
    assert_eq!(
        workspace.git_stdout(&[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}"
        ]),
        "main"
    );
}
