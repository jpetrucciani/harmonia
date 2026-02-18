use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("plan-mr-shell");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(root.join("repos")).expect("create repos dir");

        fs::write(
            root.join(".harmonia").join("config.toml"),
            r#"[workspace]
name = "plan-mr-shell-integration"
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

    fn checkout_branch(&self, repo: &str, branch: &str) {
        let repo_path = self.root.join("repos").join(repo);
        run_git(&repo_path, &["checkout", "-b", branch]);
    }

    fn write_changeset(&self, id: &str, branch: &str) {
        let dir = self.root.join(".harmonia").join("changesets");
        fs::create_dir_all(&dir).expect("create changesets dir");
        let content = format!(
            "id = \"{id}\"\ntitle = \"feat: auth\"\ndescription = \"changeset driven\"\nbranch = \"{branch}\"\n\n[[repos]]\nrepo = \"core\"\nsummary = \"shared auth helpers\"\n\n[[repos]]\nrepo = \"app\"\nsummary = \"integrate auth\"\n"
        );
        fs::write(dir.join(format!("{id}.toml")), content).expect("write changeset");
    }

    fn enable_changesets(&self) {
        let config_path = self.root.join(".harmonia").join("config.toml");
        let mut config = fs::read_to_string(&config_path).expect("read workspace config");
        config.push_str("\n[changesets]\nenabled = true\ndir = \"changesets\"\n");
        fs::write(config_path, config).expect("write workspace config");
    }

    fn run_harmonia(&self, args: &[&str]) -> std::process::Output {
        Command::new(harmonia_bin())
            .arg("--workspace")
            .arg(&self.root)
            .args(args)
            .output()
            .expect("run harmonia")
    }

    fn current_branch(&self, repo: &str) -> String {
        let repo_path = self.root.join("repos").join(repo);
        run_git_output(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"])
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

fn run_git_output(repo_path: &Path, args: &[&str]) -> String {
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
    stdout.trim().to_string()
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
fn plan_command_dispatches_and_reports_merge_order() {
    let workspace = TestWorkspace::new();
    workspace.mark_repo_changed("app");

    let output = workspace.run_harmonia(&["plan"]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "plan command failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("Changeset Analysis"), "stdout:\n{stdout}");
    assert!(stdout.contains("Changed repositories"), "stdout:\n{stdout}");
    assert!(stdout.contains("Merge order"), "stdout:\n{stdout}");
    assert!(stdout.contains("app"), "stdout:\n{stdout}");
}

#[test]
fn mr_command_dispatches_and_prints_preview() {
    let workspace = TestWorkspace::new();
    workspace.mark_repo_changed("app");

    let output = workspace.run_harmonia(&["mr"]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "mr command failed\nstdout:\n{stdout}"
    );
    assert!(stdout.contains("MR Preview"), "stdout:\n{stdout}");
}

#[test]
fn plan_command_supports_json_include_and_exclude() {
    let workspace = TestWorkspace::new();
    workspace.mark_repo_changed("app");

    let output =
        workspace.run_harmonia(&["plan", "--json", "--include", "core", "--exclude", "app"]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "plan --json failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse plan json");
    let repos = json
        .get("changed_repos")
        .and_then(|value| value.as_array())
        .expect("changed_repos array");
    let repo_names: Vec<&str> = repos
        .iter()
        .filter_map(|row| row.get("repo"))
        .filter_map(|value| value.as_str())
        .collect();
    assert!(repo_names.contains(&"core"), "json:\n{stdout}");
    assert!(!repo_names.contains(&"app"), "json:\n{stdout}");
}

#[test]
fn plan_uses_active_changeset_file_for_scope_and_metadata() {
    let workspace = TestWorkspace::new();
    workspace.enable_changesets();
    workspace.write_changeset("cs-auth", "feature/auth");
    workspace.checkout_branch("app", "feature/auth");
    workspace.mark_repo_changed("app");

    let output = workspace.run_harmonia(&["plan", "--json"]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "plan --json failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse plan json");
    let changeset_value = json.get("changeset").expect("changeset field");
    let changeset = changeset_value
        .as_object()
        .unwrap_or_else(|| panic!("changeset object\njson:\n{stdout}"));
    assert_eq!(
        changeset
            .get("id")
            .and_then(|value| value.as_str())
            .expect("changeset id"),
        "cs-auth"
    );

    let repos = json
        .get("changed_repos")
        .and_then(|value| value.as_array())
        .expect("changed_repos array");
    let repo_names: Vec<&str> = repos
        .iter()
        .filter_map(|row| row.get("repo"))
        .filter_map(|value| value.as_str())
        .collect();
    assert!(repo_names.contains(&"core"), "json:\n{stdout}");
    assert!(repo_names.contains(&"app"), "json:\n{stdout}");
}

#[test]
fn mr_subcommands_parse_for_parity() {
    let workspace = TestWorkspace::new();
    workspace.mark_repo_changed("app");

    let create_output = workspace.run_harmonia(&[
        "mr",
        "create",
        "--title",
        "feat: test",
        "--dry-run",
        "--labels",
        "a,b",
        "--reviewers",
        "alice,bob",
    ]);
    let create_stdout = String::from_utf8_lossy(&create_output.stdout).to_string();
    let create_stderr = String::from_utf8_lossy(&create_output.stderr).to_string();
    assert!(
        create_output.status.success(),
        "mr create failed\nstdout:\n{create_stdout}\nstderr:\n{create_stderr}"
    );
    assert!(
        create_stdout.contains("MR Create Plan"),
        "stdout:\n{create_stdout}"
    );
    assert!(create_stderr.is_empty(), "stderr:\n{create_stderr}");

    let status_output =
        workspace.run_harmonia(&["mr", "status", "--json", "--wait", "--timeout", "5"]);
    let status_stdout = String::from_utf8_lossy(&status_output.stdout).to_string();
    let status_stderr = String::from_utf8_lossy(&status_output.stderr).to_string();
    assert!(
        status_output.status.success(),
        "mr status failed\nstdout:\n{status_stdout}\nstderr:\n{status_stderr}"
    );
    let status_json: serde_json::Value =
        serde_json::from_slice(&status_output.stdout).expect("parse mr status json");
    assert_eq!(
        status_json
            .get("timeout_minutes")
            .and_then(|value| value.as_u64()),
        Some(5)
    );

    let update_output = workspace.run_harmonia(&[
        "mr",
        "update",
        "--description",
        "desc",
        "--labels",
        "foo,bar",
    ]);
    let update_stderr = String::from_utf8_lossy(&update_output.stderr).to_string();
    assert!(update_output.status.success(), "stderr:\n{update_stderr}");
    assert!(
        update_stderr.contains("no tracked MRs found"),
        "stderr:\n{update_stderr}"
    );

    let merge_output = workspace.run_harmonia(&[
        "mr",
        "merge",
        "--dry-run",
        "--no-wait",
        "--squash",
        "--delete-branch",
        "--yes",
    ]);
    let merge_stderr = String::from_utf8_lossy(&merge_output.stderr).to_string();
    assert!(merge_output.status.success(), "stderr:\n{merge_stderr}");
    assert!(
        merge_stderr.contains("no tracked MRs found"),
        "stderr:\n{merge_stderr}"
    );

    let close_output = workspace.run_harmonia(&["mr", "close", "--yes"]);
    let close_stderr = String::from_utf8_lossy(&close_output.stderr).to_string();
    assert!(close_output.status.success(), "stderr:\n{close_stderr}");
    assert!(
        close_stderr.contains("no tracked MRs found"),
        "stderr:\n{close_stderr}"
    );
}

#[test]
fn mr_create_auto_branch_creates_feature_branch_before_forge_call() {
    let workspace = TestWorkspace::new();
    let app_repo = workspace.root.join("repos").join("app");
    run_git(&app_repo, &["checkout", "-B", "main"]);
    workspace.mark_repo_changed("app");
    assert_eq!(workspace.current_branch("app"), "main");

    let output = workspace.run_harmonia(&[
        "mr",
        "create",
        "--auto-branch",
        "--branch-name",
        "feature/test-auto-branch",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        !output.status.success(),
        "mr create unexpectedly succeeded\nstderr:\n{stderr}"
    );
    assert_eq!(
        workspace.current_branch("app"),
        "feature/test-auto-branch",
        "stderr:\n{stderr}"
    );
}

#[test]
fn shell_command_dispatches_and_prints_exports_when_non_interactive() {
    let workspace = TestWorkspace::new();

    let output = workspace.run_harmonia(&["shell"]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "shell command failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("export HARMONIA_WORKSPACE="),
        "stdout:\n{stdout}"
    );
    assert!(stdout.contains("export PYTHONPATH="), "stdout:\n{stdout}");
}

#[test]
fn shell_command_scopes_environment_to_selected_repos() {
    let workspace = TestWorkspace::new();
    let core_src = workspace.root.join("repos").join("core").join("src");
    let app_src = workspace.root.join("repos").join("app").join("src");

    let output = workspace.run_harmonia(&["shell", "--repos", "core"]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "shell command failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains(&core_src.to_string_lossy().to_string()),
        "stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains(&app_src.to_string_lossy().to_string()),
        "stdout:\n{stdout}"
    );
}

#[test]
fn shell_command_runs_command_with_workspace_environment() {
    let workspace = TestWorkspace::new();
    let command = if cfg!(windows) {
        "echo %HARMONIA_WORKSPACE%"
    } else {
        "printf %s \"$HARMONIA_WORKSPACE\""
    };

    let output = workspace.run_harmonia(&["shell", "--repos", "core", "--command", command]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "shell command failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout.trim_end_matches(['\r', '\n']),
        workspace.root.to_string_lossy()
    );
    assert!(
        !stdout.contains("export HARMONIA_WORKSPACE="),
        "stdout:\n{stdout}"
    );
}

#[test]
fn completion_command_emits_script_for_requested_shell() {
    let workspace = TestWorkspace::new();

    let output = workspace.run_harmonia(&["completion", "bash"]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "completion command failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("_harmonia"), "stdout:\n{stdout}");
    assert!(stdout.contains("complete"), "stdout:\n{stdout}");
}
