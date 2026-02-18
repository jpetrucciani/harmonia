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

fn unique_temp_dir(prefix: &str) -> PathBuf {
    static TEMP_DIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let pid = std::process::id();
    for _ in 0..32 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let seq = TEMP_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let candidate = std::env::temp_dir().join(format!("harmonia-{prefix}-{pid}-{nanos}-{seq}"));
        match fs::create_dir(&candidate) {
            Ok(()) => return candidate,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => panic!("failed to create temp dir {}: {}", candidate.display(), err),
        }
    }

    panic!("failed to create unique temp dir for {prefix}");
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
