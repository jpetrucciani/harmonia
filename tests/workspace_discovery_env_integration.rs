use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(name: &str, alt_name: Option<&str>) -> Self {
        let root = unique_temp_dir(name);
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::write(
            root.join(".harmonia").join("config.toml"),
            workspace_config(name),
        )
        .expect("write config.toml");

        if let Some(alt_name) = alt_name {
            fs::write(
                root.join(".harmonia").join("alt.toml"),
                workspace_config(alt_name),
            )
            .expect("write alt.toml");
        }

        Self { root }
    }

    fn new_flat(name: &str) -> Self {
        let root = unique_temp_dir(name);
        fs::create_dir_all(&root).expect("create workspace root");
        fs::write(root.join(".harmonia.toml"), workspace_config(name))
            .expect("write .harmonia.toml");
        Self { root }
    }
}

impl Drop for Workspace {
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

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("harmonia-{prefix}-{pid}-{nanos}"))
}

fn workspace_config(name: &str) -> String {
    format!(
        "[workspace]\nname = \"{}\"\nrepos_dir = \"repos\"\n\n[repos]\n",
        name
    )
}

fn run_config_get_workspace_name(
    current_dir: &PathBuf,
    args: &[&str],
    envs: &[(&str, &str)],
) -> std::process::Output {
    let mut cmd = Command::new(harmonia_bin());
    cmd.current_dir(current_dir)
        .args(args)
        .arg("config")
        .arg("get")
        .arg("workspace.name");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("run harmonia config get")
}

fn assert_name(output: &std::process::Output, expected: &str, context: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(stdout.trim(), expected, "{context} stdout mismatch");
}

#[test]
fn discovers_workspace_from_current_directory() {
    let workspace = Workspace::new("discover-cwd", None);
    let output = run_config_get_workspace_name(&workspace.root, &[], &[]);
    assert_name(&output, "discover-cwd", "discover from cwd");
}

#[test]
fn discovers_workspace_from_current_directory_with_flat_config() {
    let workspace = Workspace::new_flat("discover-cwd-flat");
    let output = run_config_get_workspace_name(&workspace.root, &[], &[]);
    assert_name(
        &output,
        "discover-cwd-flat",
        "discover from cwd flat config",
    );
}

#[test]
fn uses_harmonia_workspace_env_override() {
    let cwd_workspace = Workspace::new("cwd-workspace", None);
    let env_workspace = Workspace::new("env-workspace", None);
    let env_workspace_value = env_workspace.root.to_string_lossy().to_string();

    let output = run_config_get_workspace_name(
        &cwd_workspace.root,
        &[],
        &[("HARMONIA_WORKSPACE", env_workspace_value.as_str())],
    );
    assert_name(&output, "env-workspace", "HARMONIA_WORKSPACE override");
}

#[test]
fn uses_harmonia_workspace_env_override_with_flat_config() {
    let cwd_workspace = Workspace::new("cwd-workspace", None);
    let env_workspace = Workspace::new_flat("env-workspace-flat");
    let env_workspace_value = env_workspace.root.to_string_lossy().to_string();

    let output = run_config_get_workspace_name(
        &cwd_workspace.root,
        &[],
        &[("HARMONIA_WORKSPACE", env_workspace_value.as_str())],
    );
    assert_name(
        &output,
        "env-workspace-flat",
        "HARMONIA_WORKSPACE override flat config",
    );
}

#[test]
fn uses_harmonia_config_env_override() {
    let cwd_workspace = Workspace::new("cwd-workspace", None);
    let env_workspace = Workspace::new("env-workspace", Some("env-alt-config"));
    let config_value = env_workspace
        .root
        .join(".harmonia")
        .join("alt.toml")
        .to_string_lossy()
        .to_string();

    let output = run_config_get_workspace_name(
        &cwd_workspace.root,
        &[],
        &[("HARMONIA_CONFIG", config_value.as_str())],
    );
    assert_name(&output, "env-alt-config", "HARMONIA_CONFIG override");
}

#[test]
fn uses_harmonia_config_env_override_with_flat_config_path() {
    let cwd_workspace = Workspace::new("cwd-workspace", None);
    let env_workspace = Workspace::new_flat("env-flat-config");
    let config_value = env_workspace
        .root
        .join(".harmonia.toml")
        .to_string_lossy()
        .to_string();

    let output = run_config_get_workspace_name(
        &cwd_workspace.root,
        &[],
        &[("HARMONIA_CONFIG", config_value.as_str())],
    );
    assert_name(
        &output,
        "env-flat-config",
        "HARMONIA_CONFIG override flat config path",
    );
}

#[test]
fn cli_workspace_overrides_workspace_env() {
    let cli_workspace = Workspace::new("cli-workspace", None);
    let env_workspace = Workspace::new("env-workspace", None);
    let cli_workspace_value = cli_workspace.root.to_string_lossy().to_string();
    let env_workspace_value = env_workspace.root.to_string_lossy().to_string();

    let output = run_config_get_workspace_name(
        &cli_workspace.root,
        &["--workspace", cli_workspace_value.as_str()],
        &[("HARMONIA_WORKSPACE", env_workspace_value.as_str())],
    );
    assert_name(&output, "cli-workspace", "CLI --workspace precedence");
}

#[test]
fn ignores_repo_level_harmonia_toml_when_discovering_flat_workspace() {
    let workspace = Workspace::new_flat("flat-root");
    let repo_path = workspace.root.join("repos").join("service");
    fs::create_dir_all(&repo_path).expect("create repo dir");
    fs::write(
        repo_path.join(".harmonia.toml"),
        "[package]\nname = \"service\"\n",
    )
    .expect("write repo config");

    let output = run_config_get_workspace_name(&repo_path, &[], &[]);
    assert_name(
        &output,
        "flat-root",
        "discover workspace should ignore repo-level .harmonia.toml",
    );
}
