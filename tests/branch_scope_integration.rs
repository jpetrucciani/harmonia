use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = unique_temp_dir("branch-scope");
        fs::create_dir_all(root.join(".harmonia")).expect("create .harmonia");
        fs::create_dir_all(root.join("repos")).expect("create repos dir");

        fs::write(
            root.join(".harmonia").join("config.toml"),
            r#"[workspace]
name = "branch-scope-integration"
repos_dir = "repos"

[repos]
"core" = {}
"lib" = {}
"app" = {}
"#,
        )
        .expect("write workspace config");

        Self::write_repo(&root, "core", &[]);
        Self::write_repo(&root, "lib", &["core"]);
        Self::write_repo(&root, "app", &["lib"]);

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
fn branch_with_deps_includes_downstream_dependents() {
    let workspace = TestWorkspace::new();
    let output = workspace.run_harmonia(&[
        "branch",
        "feature/with-deps",
        "--create",
        "--repos",
        "lib",
        "--with-deps",
    ]);
    assert_success(&output, "branch --with-deps");

    assert_eq!(workspace.current_branch("lib"), "feature/with-deps");
    assert_eq!(workspace.current_branch("app"), "feature/with-deps");
    assert_eq!(workspace.current_branch("core"), "main");
}

#[test]
fn branch_with_all_deps_includes_full_dependency_tree() {
    let workspace = TestWorkspace::new();
    let output = workspace.run_harmonia(&[
        "branch",
        "feature/with-all",
        "--create",
        "--repos",
        "lib",
        "--with-all-deps",
    ]);
    assert_success(&output, "branch --with-all-deps");

    assert_eq!(workspace.current_branch("core"), "feature/with-all");
    assert_eq!(workspace.current_branch("lib"), "feature/with-all");
    assert_eq!(workspace.current_branch("app"), "feature/with-all");
}
