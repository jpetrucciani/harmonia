use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("workspace-config-precedence");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::write(
            root.join(".harmonia").join("config.toml"),
            r#"[workspace]
name = "primary-config"
repos_dir = "repos"

[repos]
"#,
        )
        .expect("write primary config");
        fs::write(
            root.join(".harmonia").join("override.toml"),
            r#"[workspace]
name = "override-config"
repos_dir = "repos"

[repos]
"#,
        )
        .expect("write override config");
        Self { root }
    }

    fn run_harmonia(&self, args: &[&str]) -> std::process::Output {
        Command::new(harmonia_bin())
            .arg("--workspace")
            .arg(&self.root)
            .arg("--config")
            .arg(".harmonia/override.toml")
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

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("harmonia-{prefix}-{pid}-{nanos}"))
}

#[test]
fn workspace_and_config_flags_use_explicit_config_path() {
    let workspace = TestWorkspace::new();
    let output = workspace.run_harmonia(&["config", "get", "workspace.name"]);

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(stdout.trim(), "override-config");
}
