use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
    remote_bare: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("git-ops");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(root.join("repos")).expect("create repos dir");

        let source = root.join("origin-source");
        fs::create_dir_all(&source).expect("create source dir");
        fs::write(source.join("README.md"), "hello\n").expect("write README");
        init_git_repo(&source, "initial");
        run_git(&source, &["branch", "-M", "main"]);

        let remote_bare = root.join("service.git");
        run_git(
            &root,
            &[
                "clone",
                "--bare",
                source.to_str().expect("source utf-8 path"),
                remote_bare.to_str().expect("remote utf-8 path"),
            ],
        );

        let remote_url = format!("file://{}", remote_bare.display());
        fs::write(
            root.join(".harmonia").join("config.toml"),
            format!(
                "[workspace]\nname = \"git-ops\"\nrepos_dir = \"repos\"\n\n[repos]\n\"service\" = {{ url = \"{remote_url}\" }}\n"
            ),
        )
        .expect("write workspace config");

        Self { root, remote_bare }
    }

    fn run_harmonia(&self, args: &[&str]) -> std::process::Output {
        Command::new(harmonia_bin())
            .arg("--workspace")
            .arg(&self.root)
            .args(args)
            .output()
            .expect("run harmonia")
    }

    fn cloned_repo_path(&self) -> PathBuf {
        self.root.join("repos").join("service")
    }

    fn configure_clone_identity(&self) {
        let repo = self.cloned_repo_path();
        run_git(&repo, &["config", "user.name", "Harmonia Test"]);
        run_git(
            &repo,
            &["config", "user.email", "harmonia-test@example.com"],
        );
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

fn init_git_repo(repo_path: &Path, message: &str) {
    run_git(repo_path, &["init", "--quiet"]);
    run_git(repo_path, &["config", "user.name", "Harmonia Test"]);
    run_git(
        repo_path,
        &["config", "user.email", "harmonia-test@example.com"],
    );
    run_git(repo_path, &["add", "-A"]);
    run_git(repo_path, &["commit", "--quiet", "-m", message]);
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
fn clone_diff_add_commit_push_flow() {
    let workspace = TestWorkspace::new();

    let clone_output = workspace.run_harmonia(&["clone", "service"]);
    assert_success(&clone_output, "clone");
    workspace.configure_clone_identity();

    fs::write(
        workspace.cloned_repo_path().join("README.md"),
        "hello\nupdated\n",
    )
    .expect("write README update");

    let diff_output = workspace.run_harmonia(&["diff", "service", "--format", "json"]);
    assert_success(&diff_output, "diff --format json");
    let diff_json: serde_json::Value =
        serde_json::from_slice(&diff_output.stdout).expect("parse diff json");
    let files = diff_json
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("files"))
        .and_then(|value| value.as_array())
        .expect("files array");
    assert!(
        files
            .iter()
            .filter_map(|value| value.as_str())
            .any(|path| path == "README.md"),
        "diff json: {}",
        String::from_utf8_lossy(&diff_output.stdout)
    );

    let add_output = workspace.run_harmonia(&["add", "--repos", "service", "--all"]);
    assert_success(&add_output, "add");

    let commit_output = workspace.run_harmonia(&[
        "commit",
        "--repos",
        "service",
        "--message",
        "feat: update readme",
    ]);
    assert_success(&commit_output, "commit");

    let push_output = workspace.run_harmonia(&["push", "--repos", "service"]);
    assert_success(&push_output, "push");

    let verify = workspace.root.join("verify-clone");
    run_git(
        &workspace.root,
        &[
            "clone",
            "--quiet",
            workspace.remote_bare.to_str().expect("remote path"),
            verify.to_str().expect("verify path"),
        ],
    );
    let readme = fs::read_to_string(verify.join("README.md")).expect("read verify README");
    assert!(readme.contains("updated"));
}

#[test]
fn branch_checkout_and_sync_flow() {
    let workspace = TestWorkspace::new();

    let clone_output = workspace.run_harmonia(&["clone", "service"]);
    assert_success(&clone_output, "clone");

    let branch_output =
        workspace.run_harmonia(&["branch", "feature/sync", "--create", "--repos", "service"]);
    assert_success(&branch_output, "branch --create");

    let current_feature = Command::new("git")
        .current_dir(workspace.cloned_repo_path())
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("read feature branch");
    assert_eq!(
        String::from_utf8_lossy(&current_feature.stdout).trim(),
        "feature/sync"
    );

    let checkout_output = workspace.run_harmonia(&["checkout", "main", "--repos", "service"]);
    assert_success(&checkout_output, "checkout main");

    let upstream_clone = workspace.root.join("upstream-clone");
    run_git(
        &workspace.root,
        &[
            "clone",
            "--quiet",
            workspace.remote_bare.to_str().expect("remote path"),
            upstream_clone.to_str().expect("upstream clone path"),
        ],
    );
    run_git(&upstream_clone, &["config", "user.name", "Harmonia Test"]);
    run_git(
        &upstream_clone,
        &["config", "user.email", "harmonia-test@example.com"],
    );
    fs::write(upstream_clone.join("README.md"), "hello\nupstream\n")
        .expect("write upstream change");
    run_git(&upstream_clone, &["add", "README.md"]);
    run_git(
        &upstream_clone,
        &["commit", "--quiet", "-m", "upstream update"],
    );
    run_git(&upstream_clone, &["push", "origin", "main"]);

    let sync_output = workspace.run_harmonia(&["sync", "service"]);
    assert_success(&sync_output, "sync");

    let readme = fs::read_to_string(workspace.cloned_repo_path().join("README.md"))
        .expect("read synced README");
    assert!(readme.contains("upstream"));
}

#[test]
fn sync_reports_dirty_worktree_with_actionable_guidance() {
    let workspace = TestWorkspace::new();

    let clone_output = workspace.run_harmonia(&["clone", "service"]);
    assert_success(&clone_output, "clone");

    let upstream_clone = workspace.root.join("upstream-clone-dirty");
    run_git(
        &workspace.root,
        &[
            "clone",
            "--quiet",
            workspace.remote_bare.to_str().expect("remote path"),
            upstream_clone.to_str().expect("upstream clone path"),
        ],
    );
    run_git(&upstream_clone, &["config", "user.name", "Harmonia Test"]);
    run_git(
        &upstream_clone,
        &["config", "user.email", "harmonia-test@example.com"],
    );
    fs::write(upstream_clone.join("UPSTREAM.txt"), "upstream\n").expect("write upstream file");
    run_git(&upstream_clone, &["add", "UPSTREAM.txt"]);
    run_git(
        &upstream_clone,
        &["commit", "--quiet", "-m", "upstream update for dirty test"],
    );
    run_git(&upstream_clone, &["push", "origin", "main"]);

    fs::write(
        workspace.cloned_repo_path().join("README.md"),
        "hello\nlocal-change\n",
    )
    .expect("write local dirty change");

    let sync_output = workspace.run_harmonia(&["sync", "service"]);
    let stderr = String::from_utf8_lossy(&sync_output.stderr).to_string();
    assert!(
        !sync_output.status.success(),
        "sync unexpectedly succeeded\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("service: working tree has uncommitted changes"),
        "stderr should include repo-specific dirty-worktree message\n{stderr}"
    );
    assert!(
        stderr.contains("--autostash") && stderr.contains("--fetch-only"),
        "stderr should include actionable guidance\n{stderr}"
    );
}

#[test]
fn sync_autostash_updates_from_upstream_and_restores_local_changes() {
    let workspace = TestWorkspace::new();

    let clone_output = workspace.run_harmonia(&["clone", "service"]);
    assert_success(&clone_output, "clone");

    let upstream_clone = workspace.root.join("upstream-clone-autostash");
    run_git(
        &workspace.root,
        &[
            "clone",
            "--quiet",
            workspace.remote_bare.to_str().expect("remote path"),
            upstream_clone.to_str().expect("upstream clone path"),
        ],
    );
    run_git(&upstream_clone, &["config", "user.name", "Harmonia Test"]);
    run_git(
        &upstream_clone,
        &["config", "user.email", "harmonia-test@example.com"],
    );
    fs::write(upstream_clone.join("UPSTREAM.txt"), "upstream\n").expect("write upstream file");
    run_git(&upstream_clone, &["add", "UPSTREAM.txt"]);
    run_git(
        &upstream_clone,
        &[
            "commit",
            "--quiet",
            "-m",
            "upstream update for autostash test",
        ],
    );
    run_git(&upstream_clone, &["push", "origin", "main"]);

    fs::write(
        workspace.cloned_repo_path().join("README.md"),
        "hello\nlocal-change\n",
    )
    .expect("write local dirty change");
    fs::write(
        workspace.cloned_repo_path().join("LOCAL.txt"),
        "local-untracked\n",
    )
    .expect("write local untracked file");

    let sync_output = workspace.run_harmonia(&["sync", "service", "--autostash"]);
    assert_success(&sync_output, "sync --autostash");

    let stderr = String::from_utf8_lossy(&sync_output.stderr).to_string();
    assert!(
        stderr.contains("autostash reapplied local changes in service"),
        "stderr should report autostash use\n{stderr}"
    );

    let readme = fs::read_to_string(workspace.cloned_repo_path().join("README.md"))
        .expect("read local README");
    assert!(
        readme.contains("local-change"),
        "local README changes should be restored after autostash"
    );

    let upstream_file = fs::read_to_string(workspace.cloned_repo_path().join("UPSTREAM.txt"))
        .expect("read upstream file");
    assert!(
        upstream_file.contains("upstream"),
        "upstream update should be integrated during sync"
    );

    let local_file = fs::read_to_string(workspace.cloned_repo_path().join("LOCAL.txt"))
        .expect("read local untracked file");
    assert!(
        local_file.contains("local-untracked"),
        "local untracked file should be restored after autostash"
    );
}
