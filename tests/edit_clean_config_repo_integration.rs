use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
    config_path: PathBuf,
    repo_path: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("edit-clean-config-repo");
        let config_path = root.join(".harmonia").join("config.toml");
        let repo_path = root.join("repos").join("service");

        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(&repo_path).expect("create repo directory");

        fs::write(
            &config_path,
            r#"[workspace]
name = "edit-clean-config-repo-integration"
repos_dir = "repos"

[repos]
"service" = {}

[groups]
core = ["service"]
"#,
        )
        .expect("write workspace config");

        init_git_repo(&repo_path);

        Self {
            root,
            config_path,
            repo_path,
        }
    }

    fn run_harmonia(&self, args: &[&str]) -> std::process::Output {
        Command::new(harmonia_bin())
            .arg("--workspace")
            .arg(&self.root)
            .args(args)
            .output()
            .expect("run harmonia")
    }

    fn mark_repo_changed(&self) {
        fs::write(self.repo_path.join("CHANGED.md"), "changed\n").expect("write changed marker");
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
fn config_command_get_set_show_and_edit() {
    let workspace = TestWorkspace::new();

    let set_output = workspace.run_harmonia(&["config", "set", "defaults.default_branch", "trunk"]);
    assert_success(&set_output, "config set");

    let get_output = workspace.run_harmonia(&["config", "get", "defaults.default_branch"]);
    assert_success(&get_output, "config get");
    let get_stdout = String::from_utf8_lossy(&get_output.stdout).to_string();
    assert_eq!(get_stdout.trim(), "trunk");

    let show_output = workspace.run_harmonia(&["config", "show"]);
    assert_success(&show_output, "config show");
    let show_stdout = String::from_utf8_lossy(&show_output.stdout).to_string();
    assert!(show_stdout.contains("[defaults]"), "stdout:\n{show_stdout}");
    assert!(
        show_stdout.contains("default_branch = \"trunk\""),
        "stdout:\n{show_stdout}"
    );

    let edit_output = workspace.run_harmonia(&["config", "edit", "--editor", "true"]);
    assert_success(&edit_output, "config edit");
}

#[test]
fn repo_command_add_list_show_remove() {
    let workspace = TestWorkspace::new();

    let add_output = workspace.run_harmonia(&[
        "repo",
        "add",
        "api",
        "--url",
        "https://example.com/api.git",
        "--group",
        "core",
        "--external",
    ]);
    assert_success(&add_output, "repo add");

    let list_output = workspace.run_harmonia(&["repo", "list"]);
    assert_success(&list_output, "repo list");
    let list_stdout = String::from_utf8_lossy(&list_output.stdout).to_string();
    assert!(list_stdout.contains("service"), "stdout:\n{list_stdout}");
    assert!(list_stdout.contains("api"), "stdout:\n{list_stdout}");

    let show_output = workspace.run_harmonia(&["repo", "show", "api"]);
    assert_success(&show_output, "repo show");
    let show_stdout = String::from_utf8_lossy(&show_output.stdout).to_string();
    assert!(show_stdout.contains("repo: api"), "stdout:\n{show_stdout}");
    assert!(
        show_stdout.contains("url: https://example.com/api.git"),
        "stdout:\n{show_stdout}"
    );
    assert!(
        show_stdout.contains("external: true"),
        "stdout:\n{show_stdout}"
    );

    let remove_output = workspace.run_harmonia(&["repo", "remove", "api"]);
    assert_success(&remove_output, "repo remove");

    let list_after_remove = workspace.run_harmonia(&["repo", "list"]);
    assert_success(&list_after_remove, "repo list after remove");
    let list_after_remove_stdout = String::from_utf8_lossy(&list_after_remove.stdout).to_string();
    assert!(
        !list_after_remove_stdout.contains("api"),
        "stdout:\n{list_after_remove_stdout}"
    );

    let config_contents = fs::read_to_string(&workspace.config_path).expect("read config");
    assert!(
        !config_contents.contains("\"api\""),
        "config still contains removed repo:\n{config_contents}"
    );
}

#[test]
fn edit_command_uses_editor_with_selected_paths() {
    let workspace = TestWorkspace::new();
    workspace.mark_repo_changed();

    let explicit_output = workspace.run_harmonia(&["edit", "service", "--editor", "echo"]);
    assert_success(&explicit_output, "edit service");
    let explicit_stdout = String::from_utf8_lossy(&explicit_output.stdout).to_string();
    assert!(
        explicit_stdout.contains("repos/service"),
        "stdout:\n{explicit_stdout}"
    );

    let changed_output = workspace.run_harmonia(&["edit", "--all", "--editor", "echo"]);
    assert_success(&changed_output, "edit --all");
    let changed_stdout = String::from_utf8_lossy(&changed_output.stdout).to_string();
    assert!(
        changed_stdout.contains("repos/service"),
        "stdout:\n{changed_stdout}"
    );
}

#[test]
fn clean_command_dry_run_then_force() {
    let workspace = TestWorkspace::new();
    let temp_file = workspace.repo_path.join("TEMP_CLEAN_FILE.txt");
    fs::write(&temp_file, "remove me\n").expect("write untracked file");

    let dry_run_output = workspace.run_harmonia(&["clean", "--repos", "service"]);
    assert_success(&dry_run_output, "clean dry run");
    assert!(temp_file.exists(), "file should still exist after dry run");

    let force_output = workspace.run_harmonia(&["clean", "--repos", "service", "--force"]);
    assert_success(&force_output, "clean --force");
    assert!(
        !temp_file.exists(),
        "file should be removed by clean --force"
    );
}
