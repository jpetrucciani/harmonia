use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("hooks");
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

        let remote_url = file_url(&remote_bare);
        fs::write(
            root.join(".harmonia").join("config.toml"),
            format!(
                "[workspace]\nname = \"hooks\"\nrepos_dir = \"repos\"\n\n[repos]\n\"service\" = {{ url = \"{remote_url}\" }}\n\n[hooks]\npre_commit = \"touch workspace-pre-commit.flag\"\npre_push = \"touch workspace-pre-push.flag\"\n"
            ),
        )
        .expect("write workspace config");

        let _ = remote_bare;
        Self { root }
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

    fn write_repo_hooks(&self, disable_workspace: bool) {
        let disable = if disable_workspace {
            "disable_workspace_hooks = [\"pre_commit\", \"pre_push\"]\n"
        } else {
            ""
        };
        let contents = format!(
            "[package]\nname = \"service\"\necosystem = \"rust\"\n\n[hooks]\n{disable}pre_commit = \"touch ../../repo-pre-commit.flag\"\npre_push = \"touch ../../repo-pre-push.flag\"\n"
        );
        fs::write(self.cloned_repo_path().join(".harmonia.toml"), contents)
            .expect("write repo hooks");
    }

    fn append_change(&self) {
        let path = self.cloned_repo_path().join("README.md");
        let mut current = fs::read_to_string(&path).expect("read README");
        current.push_str("change\n");
        fs::write(path, current).expect("write README");
    }

    fn flag_exists(&self, flag_name: &str) -> bool {
        self.root.join(flag_name).exists()
    }

    fn clear_flags(&self) {
        let _ = fs::remove_file(self.root.join("workspace-pre-commit.flag"));
        let _ = fs::remove_file(self.root.join("workspace-pre-push.flag"));
        let _ = fs::remove_file(self.root.join("repo-pre-commit.flag"));
        let _ = fs::remove_file(self.root.join("repo-pre-push.flag"));
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

fn file_url(path: &Path) -> String {
    let mut normalized = path.to_string_lossy().replace('\\', "/");
    if !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }
    format!("file://{normalized}")
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
fn workspace_and_repo_hooks_run_for_commit_and_push() {
    let workspace = TestWorkspace::new();

    let clone_output = workspace.run_harmonia(&["clone", "service"]);
    assert_success(&clone_output, "clone");
    workspace.configure_clone_identity();
    workspace.write_repo_hooks(false);

    workspace.append_change();
    let commit_output = workspace.run_harmonia(&[
        "commit",
        "--repos",
        "service",
        "--message",
        "feat: hook test",
        "--all",
    ]);
    assert_success(&commit_output, "commit");

    let push_output = workspace.run_harmonia(&["push", "--repos", "service"]);
    assert_success(&push_output, "push");

    assert!(workspace.flag_exists("workspace-pre-commit.flag"));
    assert!(workspace.flag_exists("repo-pre-commit.flag"));
    assert!(workspace.flag_exists("workspace-pre-push.flag"));
    assert!(workspace.flag_exists("repo-pre-push.flag"));
}

#[test]
fn disable_workspace_hooks_skips_workspace_pre_hooks() {
    let workspace = TestWorkspace::new();

    let clone_output = workspace.run_harmonia(&["clone", "service"]);
    assert_success(&clone_output, "clone");
    workspace.configure_clone_identity();
    workspace.write_repo_hooks(true);
    workspace.clear_flags();

    workspace.append_change();
    let commit_output = workspace.run_harmonia(&[
        "commit",
        "--repos",
        "service",
        "--message",
        "feat: disable workspace hooks",
        "--all",
    ]);
    assert_success(&commit_output, "commit");

    let push_output = workspace.run_harmonia(&["push", "--repos", "service"]);
    assert_success(&push_output, "push");

    assert!(!workspace.flag_exists("workspace-pre-commit.flag"));
    assert!(!workspace.flag_exists("workspace-pre-push.flag"));
    assert!(workspace.flag_exists("repo-pre-commit.flag"));
    assert!(workspace.flag_exists("repo-pre-push.flag"));
}
