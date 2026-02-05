use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use crate::config::resolve::resolve_workspace_with_overrides;
use crate::core::repo::{Dependency, Repo, RepoId};
use crate::core::version::{
    bump_version, parse_bump_level, parse_bump_mode, parse_version_kind, BumpMode, Version,
    VersionKind,
};
use crate::core::workspace::Workspace;
use crate::ecosystem::{plugin_for, EcosystemId};
use crate::error::{HarmoniaError, Result};
use crate::git::ops::{
    branch_exists, checkout_branch, clone_repo, create_branch, current_branch, open_repo,
    repo_status, sync_repo, SyncOptions,
};
use crate::git::status::StatusSummary;
use crate::graph::constraint::{check_constraints, ConstraintReport, ViolationType};
use crate::graph::ops::{
    internal_dependencies_for, merge_order, package_map, resolve_internal_edges, topological_order,
    transitive_dependencies, transitive_dependents,
};
use crate::graph::viz;
use crate::util::{output, parallel};

#[derive(Parser, Debug)]
#[command(name = "harmonia")]
#[command(about = "Poly-repo orchestrator", long_about = None)]
pub struct Cli {
    #[arg(short, long)]
    pub workspace: Option<PathBuf>,
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    #[arg(short, long)]
    pub quiet: bool,
    #[arg(long)]
    pub no_color: bool,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Init(InitArgs),
    Clone(CloneArgs),
    Status(StatusArgs),
    Sync(SyncArgs),
    Exec(ExecArgs),
    Run(RunArgs),
    Each(EachArgs),
    Graph(GraphArgs),
    Branch(BranchArgs),
    Checkout(CheckoutArgs),
    Add(AddArgs),
    Commit(CommitArgs),
    Push(PushArgs),
    Diff(DiffArgs),
    Test,
    Lint,
    Version(VersionArgs),
    Deps(DepsArgs),
    Plan,
    Mr,
    Shell,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    pub source: Option<String>,
    #[arg(short = 'n', long)]
    pub name: Option<String>,
    #[arg(short = 'd', long)]
    pub directory: Option<PathBuf>,
    #[arg(long)]
    pub no_clone: bool,
    #[arg(long)]
    pub group: Option<String>,
}

#[derive(Args, Debug)]
pub struct CloneArgs {
    pub repos: Vec<String>,
    #[arg(short = 'g', long)]
    pub group: Option<String>,
    #[arg(short = 'a', long)]
    pub all: bool,
    #[arg(long)]
    pub depth: Option<String>,
    #[arg(long)]
    pub full: bool,
    #[arg(long)]
    pub protocol: Option<String>,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(short = 's', long)]
    pub short: bool,
    #[arg(short = 'l', long)]
    pub long: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub changed: bool,
    #[arg(long)]
    pub porcelain: bool,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    pub repos: Vec<String>,
    #[arg(short = 'r', long)]
    pub rebase: bool,
    #[arg(long = "ff-only")]
    pub ff_only: bool,
    #[arg(short = 'f', long = "fetch-only")]
    pub fetch_only: bool,
    #[arg(short = 'p', long)]
    pub prune: bool,
    #[arg(long)]
    pub parallel: Option<usize>,
}

#[derive(Args, Debug)]
pub struct ExecArgs {
    #[arg(long)]
    pub repos: Vec<String>,
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub changed: bool,
    #[arg(long)]
    pub parallel: Option<usize>,
    #[arg(long)]
    pub fail_fast: bool,
    #[arg(long)]
    pub ignore_errors: bool,
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    pub hook: String,
    #[arg(long)]
    pub repos: Vec<String>,
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub changed: bool,
    #[arg(long)]
    pub parallel: Option<usize>,
    #[arg(long)]
    pub fail_fast: bool,
}

#[derive(Args, Debug)]
pub struct EachArgs {
    #[arg(long)]
    pub repos: Vec<String>,
    #[arg(long)]
    pub parallel: Option<usize>,
    #[arg(long)]
    pub shell: bool,
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

#[derive(Args, Debug)]
pub struct GraphArgs {
    #[command(subcommand)]
    pub command: Option<GraphCommand>,
}

#[derive(Subcommand, Debug)]
pub enum GraphCommand {
    Show(GraphShowArgs),
    Deps(GraphDepsArgs),
    Dependents(GraphDependentsArgs),
    Order(GraphOrderArgs),
    Check(GraphCheckArgs),
}

#[derive(Args, Debug)]
pub struct GraphShowArgs {
    #[arg(long)]
    pub changed: bool,
    #[arg(long, default_value = "tree")]
    pub format: String,
    #[arg(long, default_value = "down")]
    pub direction: String,
}

#[derive(Args, Debug)]
pub struct GraphDepsArgs {
    pub repo: String,
    #[arg(short = 't', long)]
    pub transitive: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct GraphDependentsArgs {
    pub repo: String,
    #[arg(short = 't', long)]
    pub transitive: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct GraphOrderArgs {
    #[arg(long)]
    pub changed: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct GraphCheckArgs {
    #[arg(long)]
    pub fix: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct BranchArgs {
    pub name: String,
    #[arg(short = 'c', long)]
    pub create: bool,
    #[arg(short = 'C', long = "force-create")]
    pub force_create: bool,
    #[arg(long)]
    pub yes: bool,
    #[arg(long, value_delimiter = ',')]
    pub repos: Vec<String>,
    #[arg(long)]
    pub changed: bool,
    #[arg(long)]
    pub with_deps: bool,
    #[arg(long)]
    pub with_all_deps: bool,
    #[arg(short = 't', long)]
    pub track: Option<String>,
}

#[derive(Args, Debug)]
pub struct CheckoutArgs {
    pub branch: String,
    #[arg(long, value_delimiter = ',')]
    pub repos: Vec<String>,
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub graceful: bool,
    #[arg(long)]
    pub fallback: Option<String>,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    #[arg(long, value_delimiter = ',')]
    pub repos: Vec<String>,
    #[arg(short = 'A', long)]
    pub all: bool,
    #[arg(short = 'p', long)]
    pub patch: bool,
    #[arg(last = true)]
    pub pathspec: Vec<String>,
}

#[derive(Args, Debug)]
pub struct CommitArgs {
    #[arg(short = 'm', long)]
    pub message: Option<String>,
    #[arg(short = 'a', long)]
    pub all: bool,
    #[arg(long, value_delimiter = ',')]
    pub repos: Vec<String>,
    #[arg(long)]
    pub amend: bool,
    #[arg(long)]
    pub no_hooks: bool,
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub allow_empty: bool,
    #[arg(long = "trailer")]
    pub trailers: Vec<String>,
}

#[derive(Args, Debug)]
pub struct PushArgs {
    #[arg(long, value_delimiter = ',')]
    pub repos: Vec<String>,
    #[arg(short = 'f', long)]
    pub force: bool,
    #[arg(long = "force-with-lease")]
    pub force_with_lease: bool,
    #[arg(short = 'u', long = "set-upstream")]
    pub set_upstream: bool,
    #[arg(long)]
    pub no_hooks: bool,
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct DiffArgs {
    pub repos: Vec<String>,
    #[arg(long)]
    pub staged: bool,
    #[arg(long)]
    pub stat: bool,
    #[arg(long = "name-only")]
    pub name_only: bool,
    #[arg(long)]
    pub unified: Option<u32>,
    #[arg(long, default_value = "patch")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct VersionArgs {
    #[command(subcommand)]
    pub command: Option<VersionCommand>,
}

#[derive(Subcommand, Debug)]
pub enum VersionCommand {
    Show(VersionShowArgs),
    Check(VersionCheckArgs),
    Bump(VersionBumpArgs),
}

#[derive(Args, Debug)]
pub struct VersionShowArgs {
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub with_deps: bool,
}

#[derive(Args, Debug)]
pub struct VersionCheckArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct VersionBumpArgs {
    pub level: Option<String>,
    #[arg(long, value_delimiter = ',')]
    pub repos: Vec<String>,
    #[arg(long)]
    pub changed: bool,
    #[arg(long)]
    pub mode: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub cascade: bool,
    #[arg(long)]
    pub no_commit: bool,
    #[arg(long)]
    pub pre: Option<String>,
}

#[derive(Args, Debug)]
pub struct DepsArgs {
    #[command(subcommand)]
    pub command: Option<DepsCommand>,
}

#[derive(Subcommand, Debug)]
pub enum DepsCommand {
    Show(DepsShowArgs),
    Check(DepsCheckArgs),
    Update(DepsUpdateArgs),
}

#[derive(Args, Debug)]
pub struct DepsShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DepsCheckArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DepsUpdateArgs {
    pub packages: Vec<String>,
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run() {
    let cli = Cli::parse();
    if let Err(err) = dispatch(cli) {
        output::error(&err.to_string());
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init(args) => handle_init(args),
        Commands::Clone(args) => handle_clone(args, cli.workspace, cli.config),
        Commands::Status(args) => handle_status(args, cli.workspace, cli.config),
        Commands::Sync(args) => handle_sync(args, cli.workspace, cli.config),
        Commands::Exec(args) => handle_exec(args, cli.workspace, cli.config),
        Commands::Run(args) => handle_run(args, cli.workspace, cli.config),
        Commands::Each(args) => handle_each(args, cli.workspace, cli.config),
        Commands::Branch(args) => handle_branch(args, cli.workspace, cli.config),
        Commands::Checkout(args) => handle_checkout(args, cli.workspace, cli.config),
        Commands::Graph(args) => handle_graph(args, cli.workspace, cli.config),
        Commands::Add(args) => handle_add(args, cli.workspace, cli.config),
        Commands::Commit(args) => handle_commit(args, cli.workspace, cli.config),
        Commands::Push(args) => handle_push(args, cli.workspace, cli.config),
        Commands::Diff(args) => handle_diff(args, cli.workspace, cli.config),
        Commands::Version(args) => handle_version(args, cli.workspace, cli.config),
        Commands::Deps(args) => handle_deps(args, cli.workspace, cli.config),
        _ => Err(HarmoniaError::Other(anyhow::anyhow!(
            "command not implemented yet"
        ))),
    }
}

fn handle_init(args: InitArgs) -> Result<()> {
    let target_dir = determine_init_directory(&args)?;
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir)?;
    }

    if let Some(source) = args.source.as_ref() {
        if target_dir.read_dir()?.next().is_some() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(
                "target directory is not empty"
            )));
        }
        if Path::new(source).is_dir() {
            copy_workspace_from_path(Path::new(source), &target_dir)?;
        } else {
            output::git_op(&format!("clone {} {}", source, target_dir.display()));
            clone_repo(source, &target_dir, None)?;
        }
    } else {
        ensure_workspace_layout(&target_dir, args.name.as_deref())?;
    }

    if !args.no_clone {
        let clone_args = CloneArgs {
            repos: Vec::new(),
            group: args.group,
            all: false,
            depth: None,
            full: false,
            protocol: None,
        };
        handle_clone(clone_args, Some(target_dir.clone()), None)?;
    }

    Ok(())
}

fn determine_init_directory(args: &InitArgs) -> Result<PathBuf> {
    if let Some(dir) = args.directory.as_ref() {
        return Ok(dir.clone());
    }

    if let Some(source) = args.source.as_ref() {
        let path = Path::new(source);
        if path.is_dir() {
            if let Some(name) = path.file_name() {
                return Ok(PathBuf::from(name));
            }
        }

        if let Some(name) = derive_repo_name(source) {
            return Ok(PathBuf::from(name));
        }
    }

    env::current_dir().map_err(HarmoniaError::from)
}

fn derive_repo_name(source: &str) -> Option<String> {
    let trimmed = source.trim_end_matches('/').trim_end_matches(".git");
    let name = trimmed
        .split('/')
        .next_back()
        .unwrap_or(trimmed)
        .split(':')
        .next_back()
        .unwrap_or(trimmed);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn ensure_workspace_layout(root: &Path, name: Option<&str>) -> Result<()> {
    let harmonia_dir = root.join(".harmonia");
    let repos_dir = root.join("repos");

    if !harmonia_dir.exists() {
        fs::create_dir_all(&harmonia_dir)?;
    }
    if !repos_dir.exists() {
        fs::create_dir_all(&repos_dir)?;
    }

    let config_path = harmonia_dir.join("config.toml");
    if !config_path.exists() {
        let workspace_name = name
            .map(|value| value.to_string())
            .or_else(|| {
                root.file_name()
                    .and_then(OsStr::to_str)
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "workspace".to_string());
        let config = format!(
            "[workspace]\nname = \"{}\"\nrepos_dir = \"repos\"\n\n[repos]\n",
            workspace_name
        );
        fs::write(config_path, config)?;
    }

    ensure_gitignore(root)?;
    Ok(())
}

fn copy_workspace_from_path(source: &Path, target: &Path) -> Result<()> {
    let source_harmonia = source.join(".harmonia");
    if !source_harmonia.is_dir() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "source workspace missing .harmonia/"
        )));
    }

    ensure_workspace_layout(target, None)?;
    copy_dir_all(&source_harmonia, &target.join(".harmonia"))?;
    Ok(())
}

fn copy_dir_all(source: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        fs::create_dir_all(target)?;
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_all(&entry_path, &target_path)?;
        } else {
            fs::copy(&entry_path, &target_path)?;
        }
    }
    Ok(())
}

fn ensure_gitignore(root: &Path) -> Result<()> {
    let gitignore_path = root.join(".gitignore");
    let entry = "repos/";

    if !gitignore_path.exists() {
        fs::write(&gitignore_path, format!("{}\n", entry))?;
        return Ok(());
    }

    let contents = fs::read_to_string(&gitignore_path)?;
    if !contents.lines().any(|line| line.trim() == entry) {
        let mut new_contents = contents;
        new_contents.push_str(&format!("{}\n", entry));
        fs::write(&gitignore_path, new_contents)?;
    }

    Ok(())
}

fn handle_clone(
    args: CloneArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let repos = select_repos(
        &workspace,
        &args.repos,
        args.group.as_deref(),
        args.all,
        true,
    )?;
    let depth = parse_depth(args.depth.as_deref(), args.full)?;
    let jobs = resolve_parallel(None);

    let results = parallel::run_in_parallel(repos, jobs, |repo| {
        if repo.remote_url.is_empty() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} missing url",
                repo.id.as_str()
            ))));
        }
        if let Some(parent) = repo.path.parent() {
            crate::git::ops::ensure_repo_dir(parent)?;
        }
        output::git_op(&format!(
            "clone {} {}",
            repo.remote_url,
            repo.path.display()
        ));
        clone_repo(&repo.remote_url, &repo.path, depth)
    });

    for result in results {
        result?;
    }

    Ok(())
}

fn parse_depth(depth: Option<&str>, full: bool) -> Result<Option<u32>> {
    if full {
        return Ok(None);
    }
    let depth = match depth {
        Some(value) => value,
        None => return Ok(None),
    };
    if depth == "full" {
        return Ok(None);
    }
    let parsed: u32 = depth
        .parse()
        .map_err(|_| HarmoniaError::Other(anyhow::anyhow!("depth must be an integer or 'full'")))?;
    Ok(Some(parsed))
}

fn handle_status(
    args: StatusArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let repos = select_repos(&workspace, &[], None, true, false)?;

    let mut rows = Vec::new();
    for repo in repos {
        if !repo.path.is_dir() {
            continue;
        }
        let open = open_repo(&repo.path)?;
        let branch = current_branch(&open.repo)?;
        let status = repo_status(&open.repo)?;
        if args.changed && status.is_clean() {
            continue;
        }
        rows.push(StatusRow {
            repo: repo.id.as_str().to_string(),
            branch,
            status,
        });
    }

    if args.json {
        print_status_json(&rows)?;
        return Ok(());
    }

    print_status_table(&workspace, &rows, args.short, args.long)?;
    Ok(())
}

fn handle_sync(
    args: SyncArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let repos = select_repos(&workspace, &args.repos, None, args.repos.is_empty(), false)?;
    let jobs = resolve_parallel(args.parallel);

    let results = parallel::run_in_parallel(repos, jobs, |repo| {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        let open = open_repo(&repo.path)?;
        output::git_op(&format!("fetch (repo {})", repo.id.as_str()));
        let outcome = sync_repo(
            &open.repo,
            SyncOptions {
                fetch_only: args.fetch_only,
                ff_only: args.ff_only,
                rebase: args.rebase,
                prune: args.prune,
            },
        )?;
        if !args.fetch_only && outcome.fast_forwarded {
            output::git_op(&format!("fast-forward (repo {})", repo.id.as_str()));
        }
        if outcome.pruned > 0 {
            output::info(&format!(
                "pruned {} stale refs in {}",
                outcome.pruned,
                repo.id.as_str()
            ));
        }
        Ok(())
    });

    for result in results {
        result?;
    }

    Ok(())
}

fn handle_exec(
    args: ExecArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let default_changed = args.repos.is_empty() && !args.all;
    let repos = select_repos(
        &workspace,
        &args.repos,
        None,
        args.all || default_changed,
        false,
    )?;
    let jobs = resolve_parallel(args.parallel);

    let results = parallel::run_in_parallel(repos, jobs, |repo| {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        let status = open_repo(&repo.path)
            .and_then(|open| repo_status(&open.repo))
            .unwrap_or_default();
        if (args.changed || default_changed) && status.is_clean() {
            return Ok(());
        }
        run_command_in_repo(&repo.path, &args.command)
    });

    for result in results {
        match result {
            Ok(()) => {}
            Err(err) => {
                if args.fail_fast {
                    return Err(err);
                }
                if !args.ignore_errors {
                    return Err(err);
                }
            }
        }
    }

    Ok(())
}

fn handle_run(
    args: RunArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let repos = select_repos(&workspace, &args.repos, None, args.all, false)?;
    let jobs = resolve_parallel(args.parallel);

    let hook_name = args.hook;
    let workspace_hook = workspace
        .config
        .hooks
        .as_ref()
        .and_then(|hooks| hooks.custom.as_ref())
        .and_then(|custom| custom.get(&hook_name))
        .cloned();
    let workspace_disabled = repos.iter().any(|repo| {
        repo.config
            .as_ref()
            .and_then(|cfg| cfg.hooks.as_ref())
            .and_then(|hooks| hooks.disable_workspace_hooks.as_ref())
            .map(|disabled| disabled.iter().any(|name| name == &hook_name))
            .unwrap_or(false)
    });
    if let Some(command) = workspace_hook {
        if !workspace_disabled {
            run_command_in_repo(&workspace.root, &split_command(&command))?;
        }
    }
    let results = parallel::run_in_parallel(repos, jobs, |repo| {
        let hook = repo
            .config
            .as_ref()
            .and_then(|config| config.hooks.as_ref())
            .and_then(|hooks| hooks.custom.as_ref())
            .and_then(|custom| custom.get(&hook_name))
            .cloned();

        if let Some(command) = hook {
            run_command_in_repo(&repo.path, &split_command(&command))
        } else {
            Ok(())
        }
    });

    for result in results {
        result?;
    }

    Ok(())
}

fn handle_each(
    args: EachArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let all = args.repos.is_empty();
    let repos = select_repos(&workspace, &args.repos, None, all, false)?;
    let jobs = resolve_parallel(args.parallel);

    let results = parallel::run_in_parallel(repos, jobs, |repo| {
        if args.shell {
            run_shell_command_in_repo(&repo.path, &args.command)
        } else {
            run_command_in_repo(&repo.path, &args.command)
        }
    });

    for result in results {
        result?;
    }

    Ok(())
}

fn handle_graph(
    args: GraphArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let command = args.command.unwrap_or(GraphCommand::Show(GraphShowArgs {
        changed: false,
        format: "tree".to_string(),
        direction: "down".to_string(),
    }));

    match command {
        GraphCommand::Show(show) => handle_graph_show(show, &workspace),
        GraphCommand::Deps(deps) => handle_graph_deps(deps, &workspace),
        GraphCommand::Dependents(dependents) => handle_graph_dependents(dependents, &workspace),
        GraphCommand::Order(order) => handle_graph_order(order, &workspace),
        GraphCommand::Check(check) => handle_graph_check(check, &workspace),
    }
}

#[derive(Clone, Copy, Debug)]
enum GraphDirection {
    Down,
    Up,
    Both,
}

fn parse_graph_direction(input: &str) -> Result<GraphDirection> {
    match input.to_ascii_lowercase().as_str() {
        "down" => Ok(GraphDirection::Down),
        "up" => Ok(GraphDirection::Up),
        "both" => Ok(GraphDirection::Both),
        _ => Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "unknown graph direction '{}'",
            input
        )))),
    }
}

fn handle_graph_show(args: GraphShowArgs, workspace: &Workspace) -> Result<()> {
    let direction = parse_graph_direction(&args.direction)?;
    let mut scope: HashSet<RepoId> = workspace
        .repos
        .values()
        .filter(|repo| !repo.ignored)
        .map(|repo| repo.id.clone())
        .collect();

    if args.changed {
        let changed = changed_repos(workspace)?;
        let mut expanded = changed.clone();
        for repo in &changed {
            if matches!(direction, GraphDirection::Down | GraphDirection::Both) {
                for dep in transitive_dependencies(&workspace.graph, &workspace.repos, repo) {
                    expanded.insert(dep);
                }
            }
            if matches!(direction, GraphDirection::Up | GraphDirection::Both) {
                for dep in transitive_dependents(&workspace.graph, &workspace.repos, repo) {
                    expanded.insert(dep);
                }
            }
        }
        scope = expanded;
    }

    let edges = build_directional_edges(&workspace.graph, &workspace.repos, direction, &scope);
    let roots = graph_roots(&edges, &scope);
    let versions = collect_versions(workspace)?;
    let mut labels = HashMap::new();
    for repo in &scope {
        let label = if let Some(version) = versions.get(repo) {
            format!("{} ({})", repo.as_str(), version.raw)
        } else {
            repo.as_str().to_string()
        };
        labels.insert(repo.clone(), label);
    }

    match args.format.to_ascii_lowercase().as_str() {
        "tree" => {
            print!("{}", viz::render_tree(&roots, &edges, &labels));
            Ok(())
        }
        "flat" => {
            print!("{}", viz::render_flat(&roots, &edges, &labels));
            Ok(())
        }
        "dot" => {
            print!("{}", viz::render_dot(&edges, &labels));
            Ok(())
        }
        "json" => {
            let json = graph_to_json(&edges, &labels);
            println!(
                "{}",
                serde_json::to_string_pretty(&json)
                    .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
            );
            Ok(())
        }
        other => Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "unknown graph format '{}'",
            other
        )))),
    }
}

fn handle_graph_deps(args: GraphDepsArgs, workspace: &Workspace) -> Result<()> {
    let repo_id = RepoId::new(args.repo.clone());
    if !workspace.repos.contains_key(&repo_id) {
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "unknown repo {}",
            args.repo
        ))));
    }
    let map = package_map(&workspace.repos);
    let mut deps: Vec<String> = if args.transitive {
        transitive_dependencies(&workspace.graph, &workspace.repos, &repo_id)
            .into_iter()
            .map(|id| id.as_str().to_string())
            .collect()
    } else {
        internal_dependencies_for(&workspace.graph, &repo_id)
            .into_iter()
            .map(|dep| match map.get(&dep.name) {
                Some(target) => target.as_str().to_string(),
                None => format!("{} (missing)", dep.name),
            })
            .collect()
    };
    deps.sort();
    deps.dedup();

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&deps)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
        return Ok(());
    }

    for dep in deps {
        println!("{}", dep);
    }
    Ok(())
}

fn handle_graph_dependents(args: GraphDependentsArgs, workspace: &Workspace) -> Result<()> {
    let repo_id = RepoId::new(args.repo.clone());
    let repo = workspace.repos.get(&repo_id).ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(format!("unknown repo {}", args.repo)))
    })?;
    let package_name = repo
        .package_name
        .clone()
        .unwrap_or_else(|| repo.id.as_str().to_string());
    let mut dependents: Vec<String> = if args.transitive {
        transitive_dependents(&workspace.graph, &workspace.repos, &repo_id)
            .into_iter()
            .map(|id| id.as_str().to_string())
            .collect()
    } else {
        direct_dependents(&workspace.graph, &workspace.repos, &repo_id)
            .into_iter()
            .map(|id| id.as_str().to_string())
            .collect()
    };
    dependents.sort();
    dependents.dedup();

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&dependents)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
        return Ok(());
    }

    println!("dependents of {}:", package_name);
    for dep in dependents {
        println!("{}", dep);
    }
    Ok(())
}

fn handle_graph_order(args: GraphOrderArgs, workspace: &Workspace) -> Result<()> {
    let order = if args.changed {
        let changed = changed_repos(workspace)?;
        if changed.is_empty() {
            Vec::new()
        } else {
            merge_order(
                &workspace.graph,
                &workspace.repos,
                &changed.into_iter().collect::<Vec<_>>(),
            )
            .map_err(HarmoniaError::Other)?
        }
    } else {
        topological_order(&workspace.graph, &workspace.repos).map_err(HarmoniaError::Other)?
    };

    let output: Vec<String> = order
        .into_iter()
        .map(|id| id.as_str().to_string())
        .collect();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&output)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
    } else {
        for repo in output {
            println!("{}", repo);
        }
    }
    Ok(())
}

fn handle_graph_check(args: GraphCheckArgs, workspace: &Workspace) -> Result<()> {
    let versions = collect_versions(workspace)?;
    let report = check_constraints(&workspace.graph, &workspace.repos, &versions);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&GraphCheckJson::from(report))
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
        return Ok(());
    }

    print_constraint_report(&report, args.fix);
    Ok(())
}

fn handle_branch(
    args: BranchArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    if args.track.is_some() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "tracking branches are not implemented yet"
        )));
    }
    if args.with_deps || args.with_all_deps {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "dependency-aware selection is not implemented yet"
        )));
    }
    if args.force_create && !args.yes {
        let confirm = output::confirm(
            &format!("Force-create branch '{}' in all selected repos?", args.name),
            false,
        )
        .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        if !confirm {
            return Ok(());
        }
    }

    let workspace = load_workspace(workspace_root, config_path)?;
    let repos = select_repos(&workspace, &args.repos, None, false, false)?;

    for repo in repos {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        let open = open_repo(&repo.path)?;
        if args.changed {
            let status = repo_status(&open.repo)?;
            if status.is_clean() {
                continue;
            }
        }
        if args.create || args.force_create {
            create_branch(&open.repo, &args.name, args.force_create)?;
        } else if !branch_exists(&open.repo, &args.name)? {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "branch {} does not exist in {}",
                args.name,
                repo.id.as_str()
            ))));
        }
        checkout_branch(&open.repo, &args.name)?;
    }

    Ok(())
}

fn handle_checkout(
    args: CheckoutArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let repos = select_repos(&workspace, &args.repos, None, args.all, false)?;

    for repo in repos {
        if !repo.path.is_dir() {
            if args.graceful {
                continue;
            }
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        let open = open_repo(&repo.path)?;
        let mut target = args.branch.clone();
        if !branch_exists(&open.repo, &target)? {
            if let Some(fallback) = args.fallback.as_ref() {
                if branch_exists(&open.repo, fallback)? {
                    target = fallback.clone();
                } else if args.graceful {
                    continue;
                } else {
                    return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                        "branch {} does not exist in {}",
                        fallback,
                        repo.id.as_str()
                    ))));
                }
            } else if args.graceful {
                continue;
            } else {
                return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                    "branch {} does not exist in {}",
                    target,
                    repo.id.as_str()
                ))));
            }
        }
        checkout_branch(&open.repo, &target)?;
    }

    Ok(())
}

fn handle_add(
    args: AddArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let default_all = args.repos.is_empty();
    let repos = select_repos(&workspace, &args.repos, None, default_all, false)?;

    for repo in repos {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        if args.repos.is_empty() {
            let open = open_repo(&repo.path)?;
            let status = repo_status(&open.repo)?;
            if status.is_clean() {
                continue;
            }
        }

        let mut cmd = vec!["git".to_string(), "add".to_string()];
        if args.patch {
            cmd.push("-p".to_string());
        } else if args.all || args.pathspec.is_empty() {
            cmd.push("-A".to_string());
        }
        if !args.pathspec.is_empty() {
            cmd.push("--".to_string());
            cmd.extend(args.pathspec.iter().cloned());
        }
        output::git_op(&cmd[1..].join(" "));
        run_command_in_repo(&repo.path, &cmd)?;
    }

    Ok(())
}

fn handle_commit(
    args: CommitArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    if args.amend && !args.yes {
        let confirm = output::confirm("Amend commits in selected repos?", false)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        if !confirm {
            return Ok(());
        }
    }

    let workspace = load_workspace(workspace_root, config_path)?;
    let repos = select_repos(&workspace, &args.repos, None, false, false)?;
    let mut commit_repos = Vec::new();

    for repo in repos {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        if args.all {
            let cmd = vec!["git".to_string(), "add".to_string(), "-A".to_string()];
            output::git_op(&cmd[1..].join(" "));
            run_command_in_repo(&repo.path, &cmd)?;
        }
        let open = open_repo(&repo.path)?;
        let status = repo_status(&open.repo)?;
        if status.is_clean() && !args.allow_empty {
            continue;
        }
        commit_repos.push(repo);
    }

    if commit_repos.is_empty() {
        output::info("nothing to commit");
        return Ok(());
    }

    run_hook_for_repos(&workspace, &commit_repos, "pre_commit", args.no_hooks)?;

    for repo in commit_repos {
        let mut cmd = vec!["git".to_string(), "commit".to_string()];
        if let Some(message) = args.message.as_ref() {
            cmd.push("-m".to_string());
            cmd.push(message.clone());
        }
        if args.amend {
            cmd.push("--amend".to_string());
        }
        if args.allow_empty {
            cmd.push("--allow-empty".to_string());
        }
        for trailer in &args.trailers {
            cmd.push("--trailer".to_string());
            cmd.push(trailer.clone());
        }
        output::git_op(&cmd[1..].join(" "));
        run_command_in_repo(&repo.path, &cmd)?;
    }

    Ok(())
}

fn handle_push(
    args: PushArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    if args.force && args.force_with_lease {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "--force and --force-with-lease cannot be used together"
        )));
    }

    if (args.force || args.force_with_lease) && !args.yes {
        let confirm = output::confirm("Force push selected repos?", false)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        if !confirm {
            return Ok(());
        }
    }

    let workspace = load_workspace(workspace_root, config_path)?;
    let repos = select_repos(&workspace, &args.repos, None, false, false)?;

    run_hook_for_repos(&workspace, &repos, "pre_push", args.no_hooks)?;

    for repo in repos {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        let mut cmd = vec!["git".to_string(), "push".to_string()];
        if args.dry_run {
            cmd.push("--dry-run".to_string());
        }
        if args.force_with_lease {
            cmd.push("--force-with-lease".to_string());
        } else if args.force {
            cmd.push("--force".to_string());
        }
        if args.set_upstream {
            cmd.push("-u".to_string());
        }
        output::git_op(&cmd[1..].join(" "));
        run_command_in_repo(&repo.path, &cmd)?;
    }

    Ok(())
}

fn handle_diff(
    args: DiffArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let default_changed = args.repos.is_empty();
    let mut repos = select_repos(&workspace, &args.repos, None, default_changed, false)?;

    if default_changed {
        repos = filter_changed_repos(repos)?;
    }

    if args.format.eq_ignore_ascii_case("json") {
        let mut entries = Vec::new();
        for repo in repos {
            let output = git_diff_output(&repo.path, args.staged)?;
            let files: Vec<String> = output
                .lines()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .map(|line| line.to_string())
                .collect();
            entries.push(DiffJsonEntry {
                repo: repo.id.as_str().to_string(),
                files,
            });
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&entries)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
        return Ok(());
    }

    let multi = repos.len() > 1;
    for repo in repos {
        if multi {
            println!("== {} ==", repo.id.as_str());
        }
        let cmd = build_diff_command(&args);
        output::git_op(&cmd[1..].join(" "));
        run_command_in_repo(&repo.path, &cmd)?;
    }

    Ok(())
}

fn handle_version(
    args: VersionArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let command = args
        .command
        .unwrap_or(VersionCommand::Show(VersionShowArgs {
            json: false,
            with_deps: false,
        }));

    match command {
        VersionCommand::Show(show) => handle_version_show(show, &workspace),
        VersionCommand::Check(check) => handle_version_check(check, &workspace),
        VersionCommand::Bump(bump) => handle_version_bump(bump, &workspace),
    }
}

fn handle_deps(
    args: DepsArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let command = args
        .command
        .unwrap_or(DepsCommand::Show(DepsShowArgs { json: false }));

    match command {
        DepsCommand::Show(show) => handle_deps_show(show, &workspace),
        DepsCommand::Check(check) => handle_deps_check(check, &workspace),
        DepsCommand::Update(update) => handle_deps_update(update, &workspace),
    }
}

fn load_workspace(
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<Workspace> {
    let cwd = env::current_dir()?;
    let resolved = resolve_workspace_with_overrides(cwd, workspace_root, config_path)?;
    Workspace::load_from(resolved.root, resolved.config_path).map_err(HarmoniaError::from)
}

fn select_repos(
    workspace: &Workspace,
    repos: &[String],
    group: Option<&str>,
    all: bool,
    include_external: bool,
) -> Result<Vec<crate::core::repo::Repo>> {
    if !repos.is_empty() {
        return repos
            .iter()
            .map(|name| {
                workspace
                    .repos
                    .get(&crate::core::repo::RepoId::new(name.clone()))
                    .cloned()
                    .ok_or_else(|| {
                        HarmoniaError::Other(anyhow::anyhow!(format!("unknown repo {}", name)))
                    })
            })
            .collect();
    }

    if let Some(group_name) = group {
        if let Some(groups) = workspace.config.groups.as_ref() {
            if let Some(group_repos) = groups.groups.get(group_name) {
                let mut selected = Vec::new();
                for name in group_repos {
                    if let Some(repo) = workspace
                        .repos
                        .get(&crate::core::repo::RepoId::new(name.clone()))
                    {
                        if should_include_repo(repo, include_external) {
                            selected.push(repo.clone());
                        }
                    }
                }
                return Ok(selected);
            }
        }
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "unknown group {}",
            group_name
        ))));
    }

    if all {
        return Ok(workspace
            .repos
            .values()
            .filter(|repo| should_include_repo(repo, include_external))
            .cloned()
            .collect());
    }

    if let Some(groups) = workspace.config.groups.as_ref() {
        if let Some(default_group) = groups.default.as_ref() {
            return select_repos(workspace, &[], Some(default_group), false, include_external);
        }
    }

    Ok(workspace
        .repos
        .values()
        .filter(|repo| should_include_repo(repo, include_external))
        .cloned()
        .collect())
}

fn should_include_repo(repo: &crate::core::repo::Repo, include_external: bool) -> bool {
    if repo.ignored {
        return false;
    }
    if repo.external && !include_external {
        return false;
    }
    true
}

fn resolve_parallel(override_value: Option<usize>) -> Option<usize> {
    if let Some(value) = override_value {
        return Some(value);
    }
    if let Ok(value) = env::var("HARMONIA_PARALLEL") {
        if let Ok(parsed) = value.parse() {
            return Some(parsed);
        }
    }
    std::thread::available_parallelism().ok().map(|n| n.get())
}

fn run_command_in_repo(repo_path: &Path, command: &[String]) -> Result<()> {
    if command.is_empty() {
        return Err(HarmoniaError::Other(anyhow::anyhow!("missing command")));
    }
    let mut cmd = std::process::Command::new(&command[0]);
    if command.len() > 1 {
        cmd.args(&command[1..]);
    }
    let status = cmd
        .current_dir(repo_path)
        .status()
        .with_context(|| format!("failed to run {:?}", command))?;
    if status.success() {
        Ok(())
    } else {
        Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "command {:?} failed",
            command
        ))))
    }
}

fn run_command_output_in_repo(repo_path: &Path, command: &[String]) -> Result<String> {
    if command.is_empty() {
        return Err(HarmoniaError::Other(anyhow::anyhow!("missing command")));
    }
    let mut cmd = std::process::Command::new(&command[0]);
    if command.len() > 1 {
        cmd.args(&command[1..]);
    }
    let output = cmd
        .current_dir(repo_path)
        .output()
        .with_context(|| format!("failed to run {:?}", command))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "command {:?} failed",
            command
        ))))
    }
}

fn run_shell_command_in_repo(repo_path: &Path, command: &[String]) -> Result<()> {
    let joined = command.join(" ");
    if joined.is_empty() {
        return Err(HarmoniaError::Other(anyhow::anyhow!("missing command")));
    }

    let mut cmd = if cfg!(windows) {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(&joined);
        cmd
    } else {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(&joined);
        cmd
    };
    let status = cmd
        .current_dir(repo_path)
        .status()
        .with_context(|| format!("failed to run shell command {}", joined))?;
    if status.success() {
        Ok(())
    } else {
        Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "shell command '{}' failed",
            joined
        ))))
    }
}

fn split_command(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(|part| part.to_string())
        .collect()
}

#[derive(Serialize)]
struct GraphJson {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

#[derive(Serialize)]
struct GraphNode {
    id: String,
    label: String,
}

#[derive(Serialize)]
struct GraphEdge {
    from: String,
    to: String,
}

#[derive(Serialize)]
struct GraphCheckJson {
    cycles: Vec<Vec<String>>,
    missing: Vec<GraphMissingJson>,
    violations: Vec<GraphViolationJson>,
}

#[derive(Serialize)]
struct GraphMissingJson {
    from: String,
    dependency: String,
    constraint: String,
}

#[derive(Serialize)]
struct GraphViolationJson {
    from: String,
    to: String,
    constraint: String,
    actual: String,
    kind: String,
}

#[derive(Serialize)]
struct DiffJsonEntry {
    repo: String,
    files: Vec<String>,
}

#[derive(Serialize)]
struct VersionEntryJson {
    repo: String,
    version: Option<String>,
    dependencies: Option<Vec<VersionDepJson>>,
}

#[derive(Serialize)]
struct VersionDepJson {
    name: String,
    constraint: String,
    actual: Option<String>,
}

#[derive(Serialize)]
struct DepsEntryJson {
    repo: String,
    dependencies: Vec<DepsDepJson>,
}

#[derive(Serialize)]
struct DepsDepJson {
    name: String,
    constraint: String,
    actual: Option<String>,
}

#[derive(Clone)]
struct DependencyUpdate {
    repo: RepoId,
    dependency: String,
    constraint: String,
}

fn graph_to_json(
    edges: &HashMap<RepoId, Vec<RepoId>>,
    labels: &HashMap<RepoId, String>,
) -> GraphJson {
    let mut nodes: Vec<GraphNode> = labels
        .iter()
        .map(|(id, label)| GraphNode {
            id: id.as_str().to_string(),
            label: label.clone(),
        })
        .collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut edge_list = Vec::new();
    for (from, deps) in edges {
        for dep in deps {
            edge_list.push(GraphEdge {
                from: from.as_str().to_string(),
                to: dep.as_str().to_string(),
            });
        }
    }
    edge_list.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));

    GraphJson {
        nodes,
        edges: edge_list,
    }
}

impl From<ConstraintReport> for GraphCheckJson {
    fn from(report: ConstraintReport) -> Self {
        let cycles = report
            .cycles
            .into_iter()
            .map(|cycle| {
                cycle
                    .into_iter()
                    .map(|id| id.as_str().to_string())
                    .collect()
            })
            .collect();
        let missing = report
            .missing
            .into_iter()
            .map(|item| GraphMissingJson {
                from: item.from.as_str().to_string(),
                dependency: item.dependency.name,
                constraint: item.dependency.constraint.raw,
            })
            .collect();
        let violations = report
            .violations
            .into_iter()
            .map(|violation| GraphViolationJson {
                from: violation.from_repo.as_str().to_string(),
                to: violation.to_repo.as_str().to_string(),
                constraint: violation.constraint.raw,
                actual: violation.actual_version.raw,
                kind: violation_type_label(violation.violation_type),
            })
            .collect();
        Self {
            cycles,
            missing,
            violations,
        }
    }
}

fn violation_type_label(violation: ViolationType) -> String {
    match violation {
        ViolationType::Unsatisfied => "unsatisfied".to_string(),
        ViolationType::ExactPin => "exact-pin".to_string(),
        ViolationType::UpperBound => "upper-bound".to_string(),
        ViolationType::Circular => "circular".to_string(),
    }
}

fn print_constraint_report(report: &ConstraintReport, show_fixes: bool) {
    if report.cycles.is_empty() && report.missing.is_empty() && report.violations.is_empty() {
        output::info("no constraint issues found");
        return;
    }

    if !report.cycles.is_empty() {
        println!("cycles:");
        for cycle in &report.cycles {
            let line = cycle
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(" -> ");
            println!("  {}", line);
        }
    }

    if !report.missing.is_empty() {
        println!("missing internal dependencies:");
        for missing in &report.missing {
            println!(
                "  {} -> {} ({})",
                missing.from.as_str(),
                missing.dependency.name,
                missing.dependency.constraint.raw
            );
        }
    }

    if !report.violations.is_empty() {
        println!("constraint violations:");
        for violation in &report.violations {
            println!(
                "  {} -> {} {} (actual {}) [{}]",
                violation.from_repo.as_str(),
                violation.to_repo.as_str(),
                violation.constraint.raw,
                violation.actual_version.raw,
                violation_type_label(violation.violation_type.clone())
            );
            if show_fixes {
                if let Some(suggestion) = constraint_fix_suggestion(violation) {
                    println!("    suggestion: {}", suggestion);
                }
            }
        }
    }
}

fn constraint_fix_suggestion(
    violation: &crate::graph::constraint::ConstraintViolation,
) -> Option<String> {
    match violation.violation_type {
        ViolationType::Unsatisfied => Some(format!(
            "update constraint to include {}",
            violation.actual_version.raw
        )),
        ViolationType::ExactPin => Some("relax exact pin to a range".to_string()),
        ViolationType::UpperBound => Some("consider widening upper bound".to_string()),
        ViolationType::Circular => None,
    }
}

fn build_directional_edges(
    graph: &crate::graph::DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
    direction: GraphDirection,
    scope: &HashSet<RepoId>,
) -> HashMap<RepoId, Vec<RepoId>> {
    let resolved = resolve_internal_edges(graph, repos);
    let mut edges: HashMap<RepoId, HashSet<RepoId>> = HashMap::new();
    for node in scope {
        edges.insert(node.clone(), HashSet::new());
    }

    for (from, deps) in resolved.edges {
        for dep in deps {
            if !scope.contains(&from) || !scope.contains(&dep) {
                continue;
            }
            if matches!(direction, GraphDirection::Down | GraphDirection::Both) {
                if let Some(entry) = edges.get_mut(&from) {
                    entry.insert(dep.clone());
                }
            }
            if matches!(direction, GraphDirection::Up | GraphDirection::Both) {
                if let Some(entry) = edges.get_mut(&dep) {
                    entry.insert(from.clone());
                }
            }
        }
    }

    let mut out = HashMap::new();
    for (node, deps) in edges {
        let mut list: Vec<RepoId> = deps.into_iter().collect();
        list.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out.insert(node, list);
    }
    out
}

fn graph_roots(edges: &HashMap<RepoId, Vec<RepoId>>, scope: &HashSet<RepoId>) -> Vec<RepoId> {
    let mut indegree: HashMap<RepoId, usize> = HashMap::new();
    for node in scope {
        indegree.entry(node.clone()).or_insert(0);
    }
    for deps in edges.values() {
        for dep in deps {
            if let Some(entry) = indegree.get_mut(dep) {
                *entry += 1;
            }
        }
    }
    let mut roots: Vec<RepoId> = indegree
        .iter()
        .filter_map(|(node, &count)| if count == 0 { Some(node.clone()) } else { None })
        .collect();
    if roots.is_empty() {
        roots = scope.iter().cloned().collect();
    }
    roots.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    roots
}

fn direct_dependents(
    graph: &crate::graph::DependencyGraph,
    repos: &HashMap<RepoId, Repo>,
    repo: &RepoId,
) -> Vec<RepoId> {
    let resolved = resolve_internal_edges(graph, repos);
    let mut dependents = Vec::new();
    for (from, deps) in resolved.edges {
        if deps.iter().any(|dep| dep == repo) {
            dependents.push(from);
        }
    }
    dependents.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    dependents
}

fn build_diff_command(args: &DiffArgs) -> Vec<String> {
    let mut cmd = vec!["git".to_string(), "diff".to_string()];
    if args.staged {
        cmd.push("--staged".to_string());
    }
    let format = args.format.to_ascii_lowercase();
    if args.stat || format == "stat" {
        cmd.push("--stat".to_string());
    }
    if args.name_only || format == "name-only" {
        cmd.push("--name-only".to_string());
    }
    if let Some(lines) = args.unified {
        cmd.push(format!("--unified={lines}"));
    }
    cmd
}

fn git_diff_output(repo_path: &Path, staged: bool) -> Result<String> {
    let mut cmd = vec![
        "git".to_string(),
        "diff".to_string(),
        "--name-only".to_string(),
    ];
    if staged {
        cmd.push("--staged".to_string());
    }
    output::git_op(&cmd[1..].join(" "));
    run_command_output_in_repo(repo_path, &cmd)
}

fn run_hook_for_repos(
    workspace: &Workspace,
    repos: &[Repo],
    hook_name: &str,
    skip: bool,
) -> Result<()> {
    if skip {
        return Ok(());
    }

    let workspace_hook = workspace
        .config
        .hooks
        .as_ref()
        .and_then(|hooks| match hook_name {
            "pre_commit" => hooks.pre_commit.as_ref(),
            "pre_push" => hooks.pre_push.as_ref(),
            _ => None,
        });
    let should_run_workspace = repos
        .iter()
        .any(|repo| !repo_disables_hook(repo, hook_name));
    if let Some(command) = workspace_hook {
        if should_run_workspace {
            run_command_in_repo(&workspace.root, &split_command(command))?;
        }
    }

    for repo in repos {
        let hook = repo
            .config
            .as_ref()
            .and_then(|config| config.hooks.as_ref())
            .and_then(|hooks| match hook_name {
                "pre_commit" => hooks.pre_commit.as_ref(),
                "pre_push" => hooks.pre_push.as_ref(),
                _ => None,
            });
        if let Some(command) = hook {
            run_command_in_repo(&repo.path, &split_command(command))?;
        }
    }

    Ok(())
}

fn repo_disables_hook(repo: &Repo, hook_name: &str) -> bool {
    repo.config
        .as_ref()
        .and_then(|config| config.hooks.as_ref())
        .and_then(|hooks| hooks.disable_workspace_hooks.as_ref())
        .map(|disabled| disabled.iter().any(|name| name == hook_name))
        .unwrap_or(false)
}

fn changed_repos(workspace: &Workspace) -> Result<HashSet<RepoId>> {
    let mut changed = HashSet::new();
    for repo in workspace.repos.values() {
        if repo.ignored || !repo.path.is_dir() {
            continue;
        }
        let open = open_repo(&repo.path)?;
        let status = repo_status(&open.repo)?;
        if !status.is_clean() {
            changed.insert(repo.id.clone());
        }
    }
    Ok(changed)
}

fn filter_changed_repos(repos: Vec<Repo>) -> Result<Vec<Repo>> {
    let mut out = Vec::new();
    for repo in repos {
        if !repo.path.is_dir() {
            continue;
        }
        let open = open_repo(&repo.path)?;
        let status = repo_status(&open.repo)?;
        if !status.is_clean() {
            out.push(repo);
        }
    }
    Ok(out)
}

fn handle_version_show(args: VersionShowArgs, workspace: &Workspace) -> Result<()> {
    let versions = collect_versions(workspace)?;
    let package_map = package_map(&workspace.repos);

    let mut repos: Vec<&Repo> = workspace
        .repos
        .values()
        .filter(|repo| !repo.ignored)
        .collect();
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    let mut entries = Vec::new();
    for repo in repos {
        let version = versions.get(&repo.id).map(|v| v.raw.clone());
        let dependencies = if args.with_deps {
            let mut deps = Vec::new();
            for dep in internal_dependencies_for(&workspace.graph, &repo.id) {
                let actual = package_map
                    .get(&dep.name)
                    .and_then(|id| versions.get(id))
                    .map(|v| v.raw.clone());
                deps.push(VersionDepJson {
                    name: dep.name,
                    constraint: dep.constraint.raw,
                    actual,
                });
            }
            deps.sort_by(|a, b| a.name.cmp(&b.name));
            Some(deps)
        } else {
            None
        };
        entries.push(VersionEntryJson {
            repo: repo.id.as_str().to_string(),
            version,
            dependencies,
        });
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&entries)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
        return Ok(());
    }

    for entry in entries {
        match entry.version {
            Some(version) => println!("{}: {}", entry.repo, version),
            None => println!("{}: (no version)", entry.repo),
        }
        if let Some(deps) = entry.dependencies {
            for dep in deps {
                if let Some(actual) = dep.actual {
                    println!("  {} {} (actual {})", dep.name, dep.constraint, actual);
                } else {
                    println!("  {} {}", dep.name, dep.constraint);
                }
            }
        }
    }

    Ok(())
}

fn handle_version_check(args: VersionCheckArgs, workspace: &Workspace) -> Result<()> {
    let versions = collect_versions(workspace)?;
    let mut report = check_constraints(&workspace.graph, &workspace.repos, &versions);
    report
        .violations
        .retain(|violation| matches!(violation.violation_type, ViolationType::Unsatisfied));

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&GraphCheckJson::from(report))
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
        return Ok(());
    }

    print_constraint_report(&report, false);
    Ok(())
}

fn handle_version_bump(args: VersionBumpArgs, workspace: &Workspace) -> Result<()> {
    let override_mode = match args.mode.as_deref() {
        Some(mode) => Some(parse_bump_mode(mode).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!("unknown bump mode '{}'", mode)))
        })?),
        None => None,
    };
    let level = match args.level.as_deref() {
        Some(level) => Some(parse_bump_level(level).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!("unknown bump level '{}'", level)))
        })?),
        None => None,
    };

    let default_changed = args.repos.is_empty();
    let mut repos = select_repos(workspace, &args.repos, None, false, false)?;
    if args.changed || default_changed {
        repos = filter_changed_repos(repos)?;
    }
    if repos.is_empty() {
        output::info("no repos selected for version bump");
        return Ok(());
    }

    let calver_format = workspace
        .config
        .versioning
        .as_ref()
        .and_then(|config| config.calver_format.as_deref());
    let cascade = args.cascade
        || workspace
            .config
            .versioning
            .as_ref()
            .and_then(|config| config.cascade_bumps)
            .unwrap_or(false);

    let mut bump_plan: HashMap<RepoId, Version> = HashMap::new();
    for repo in &repos {
        let current = read_repo_version(repo, workspace)?.ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "no version found for {}",
                repo.id.as_str()
            )))
        })?;
        let mode = resolve_bump_mode(repo, workspace, override_mode)?;
        if args.pre.is_some() && mode != BumpMode::Semver {
            return Err(HarmoniaError::Other(anyhow::anyhow!(
                "prerelease tags are only supported with semver"
            )));
        }
        let new_version =
            bump_version(&current, mode, level, calver_format, args.pre.as_deref())
                .map_err(|err| HarmoniaError::Other(anyhow::anyhow!(format!("{}", err))))?;
        bump_plan.insert(repo.id.clone(), new_version);
    }

    if cascade {
        let mut dependents = HashSet::new();
        for repo in &repos {
            for dep in transitive_dependents(&workspace.graph, &workspace.repos, &repo.id) {
                dependents.insert(dep);
            }
        }
        for dep_id in dependents {
            if bump_plan.contains_key(&dep_id) {
                continue;
            }
            let dep_repo = match workspace.repos.get(&dep_id) {
                Some(repo) => repo,
                None => continue,
            };
            if dep_repo.external || dep_repo.ignored {
                continue;
            }
            let current = read_repo_version(dep_repo, workspace)?.ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!(format!(
                    "no version found for {}",
                    dep_repo.id.as_str()
                )))
            })?;
            let mode = resolve_bump_mode(dep_repo, workspace, override_mode)?;
            if args.pre.is_some() && mode != BumpMode::Semver {
                return Err(HarmoniaError::Other(anyhow::anyhow!(
                    "prerelease tags are only supported with semver"
                )));
            }
            let new_version =
                bump_version(&current, mode, level, calver_format, args.pre.as_deref())
                    .map_err(|err| HarmoniaError::Other(anyhow::anyhow!(format!("{}", err))))?;
            bump_plan.insert(dep_repo.id.clone(), new_version);
        }
    }

    let dep_updates = if cascade {
        build_dependency_updates(workspace, &bump_plan)?
    } else {
        Vec::new()
    };

    if args.dry_run {
        println!("version bump plan:");
        let mut planned: Vec<_> = bump_plan.iter().collect();
        planned.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
        for (repo_id, version) in planned {
            println!("  {} -> {}", repo_id.as_str(), version.raw);
        }
        if !dep_updates.is_empty() {
            println!("dependency updates:");
            for update in &dep_updates {
                println!(
                    "  {}: {} -> {}",
                    update.repo.as_str(),
                    update.dependency,
                    update.constraint
                );
            }
        }
        return Ok(());
    }

    for (repo_id, version) in &bump_plan {
        let repo = workspace.repos.get(repo_id).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {}",
                repo_id.as_str()
            )))
        })?;
        update_repo_version(repo, version, args.dry_run)?;
    }

    for update in dep_updates {
        let repo = workspace.repos.get(&update.repo).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {}",
                update.repo.as_str()
            )))
        })?;
        update_dependency_in_repo(repo, &update.dependency, &update.constraint, args.dry_run)?;
    }

    Ok(())
}

fn handle_deps_show(args: DepsShowArgs, workspace: &Workspace) -> Result<()> {
    let versions = collect_versions(workspace)?;
    let package_map = package_map(&workspace.repos);
    let mut repos: Vec<&Repo> = workspace
        .repos
        .values()
        .filter(|repo| !repo.ignored)
        .collect();
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    let mut entries = Vec::new();
    for repo in repos {
        let mut deps = Vec::new();
        for dep in internal_dependencies_for(&workspace.graph, &repo.id) {
            let actual = package_map
                .get(&dep.name)
                .and_then(|id| versions.get(id))
                .map(|v| v.raw.clone());
            deps.push(DepsDepJson {
                name: dep.name,
                constraint: dep.constraint.raw,
                actual,
            });
        }
        deps.sort_by(|a, b| a.name.cmp(&b.name));
        entries.push(DepsEntryJson {
            repo: repo.id.as_str().to_string(),
            dependencies: deps,
        });
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&entries)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
        return Ok(());
    }

    for entry in entries {
        println!("{}:", entry.repo);
        for dep in entry.dependencies {
            if let Some(actual) = dep.actual {
                println!("  {} {} (actual {})", dep.name, dep.constraint, actual);
            } else {
                println!("  {} {}", dep.name, dep.constraint);
            }
        }
    }
    Ok(())
}

fn handle_deps_check(args: DepsCheckArgs, workspace: &Workspace) -> Result<()> {
    let versions = collect_versions(workspace)?;
    let mut report = check_constraints(&workspace.graph, &workspace.repos, &versions);
    report
        .violations
        .retain(|violation| matches!(violation.violation_type, ViolationType::Unsatisfied));

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&GraphCheckJson::from(report))
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
        return Ok(());
    }

    print_constraint_report(&report, false);
    Ok(())
}

fn handle_deps_update(args: DepsUpdateArgs, workspace: &Workspace) -> Result<()> {
    let versions = collect_versions(workspace)?;
    let map = package_map(&workspace.repos);
    let mut target_names = HashSet::new();
    if !args.packages.is_empty() {
        for package in &args.packages {
            if let Some(repo) = workspace.repos.get(&RepoId::new(package.clone())) {
                let name = repo
                    .package_name
                    .clone()
                    .unwrap_or_else(|| repo.id.as_str().to_string());
                target_names.insert(name);
                continue;
            }
            if map.contains_key(package) {
                target_names.insert(package.clone());
                continue;
            }
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown package {}",
                package
            ))));
        }
    }

    let mut updates = Vec::new();
    let mut repos: Vec<&Repo> = workspace
        .repos
        .values()
        .filter(|repo| !repo.ignored && !repo.external)
        .collect();
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    for repo in repos {
        for dep in internal_dependencies_for(&workspace.graph, &repo.id) {
            if !target_names.is_empty() && !target_names.contains(&dep.name) {
                continue;
            }
            let target = match map.get(&dep.name) {
                Some(repo_id) => repo_id,
                None => continue,
            };
            let version = match versions.get(target) {
                Some(version) => version,
                None => continue,
            };
            let constraint = update_constraint_for_repo(repo, &dep, version);
            updates.push(DependencyUpdate {
                repo: repo.id.clone(),
                dependency: dep.name,
                constraint,
            });
        }
    }

    if args.dry_run {
        println!("dependency update plan:");
        for update in &updates {
            println!(
                "  {}: {} -> {}",
                update.repo.as_str(),
                update.dependency,
                update.constraint
            );
        }
        return Ok(());
    }

    for update in updates {
        let repo = workspace.repos.get(&update.repo).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {}",
                update.repo.as_str()
            )))
        })?;
        update_dependency_in_repo(repo, &update.dependency, &update.constraint, args.dry_run)?;
    }

    Ok(())
}

fn collect_versions(workspace: &Workspace) -> Result<HashMap<RepoId, Version>> {
    let mut versions = HashMap::new();
    for repo in workspace.repos.values() {
        if repo.ignored {
            continue;
        }
        if let Some(version) = read_repo_version(repo, workspace)? {
            versions.insert(repo.id.clone(), version);
        }
    }
    Ok(versions)
}

fn read_repo_version(repo: &Repo, workspace: &Workspace) -> Result<Option<Version>> {
    let file = match version_file_for_repo(repo) {
        Some(path) => path,
        None => return Ok(None),
    };
    if !file.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&file)?;
    let strategy = resolve_version_kind(repo, workspace)?;
    let version_cfg = repo
        .config
        .as_ref()
        .and_then(|config| config.versioning.as_ref());

    let raw = if let Some(pattern) = version_cfg.and_then(|cfg| cfg.pattern.as_ref()) {
        read_version_with_pattern(pattern, &content)?
    } else if let Some(path) = version_cfg.and_then(|cfg| cfg.path.as_ref()) {
        read_version_with_path(&file, &content, path)?
    } else if let Some(ecosystem) = repo.ecosystem.as_ref() {
        let plugin = plugin_for(ecosystem);
        plugin
            .parse_version(&file, &content)?
            .map(|version| version.raw)
    } else {
        None
    };

    match raw {
        Some(raw) => {
            let version = Version::new(raw, strategy.clone());
            if strategy == VersionKind::Semver && version.semver.is_none() {
                return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                    "invalid semver for {}",
                    repo.id.as_str()
                ))));
            }
            Ok(Some(version))
        }
        None => Ok(None),
    }
}

fn resolve_version_kind(repo: &Repo, workspace: &Workspace) -> Result<VersionKind> {
    let strategy = repo
        .config
        .as_ref()
        .and_then(|config| config.versioning.as_ref())
        .and_then(|config| config.strategy.as_ref())
        .or_else(|| {
            workspace
                .config
                .versioning
                .as_ref()
                .and_then(|config| config.strategy.as_ref())
        });

    if let Some(strategy) = strategy {
        return parse_version_kind(strategy).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown versioning strategy '{}'",
                strategy
            )))
        });
    }

    Ok(VersionKind::Semver)
}

fn resolve_bump_mode(
    repo: &Repo,
    workspace: &Workspace,
    override_mode: Option<BumpMode>,
) -> Result<BumpMode> {
    if let Some(mode) = override_mode {
        return Ok(mode);
    }
    let mode = repo
        .config
        .as_ref()
        .and_then(|config| config.versioning.as_ref())
        .and_then(|config| config.bump_mode.as_ref())
        .or_else(|| {
            workspace
                .config
                .versioning
                .as_ref()
                .and_then(|config| config.bump_mode.as_ref())
        });
    if let Some(mode) = mode {
        return parse_bump_mode(mode).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!("unknown bump mode '{}'", mode)))
        });
    }
    Ok(BumpMode::Semver)
}

fn version_file_for_repo(repo: &Repo) -> Option<PathBuf> {
    let configured = repo
        .config
        .as_ref()
        .and_then(|config| config.versioning.as_ref())
        .and_then(|config| config.file.as_ref())
        .map(|file| repo.path.join(file));
    if configured.is_some() {
        return configured;
    }
    let ecosystem = repo.ecosystem.as_ref()?;
    let plugin = plugin_for(ecosystem);
    for pattern in plugin.file_patterns() {
        let candidate = repo.path.join(pattern);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn deps_file_for_repo(repo: &Repo) -> Option<PathBuf> {
    let configured = repo
        .config
        .as_ref()
        .and_then(|config| config.dependencies.as_ref())
        .and_then(|config| config.file.as_ref())
        .map(|file| repo.path.join(file));
    if configured.is_some() {
        return configured;
    }
    let ecosystem = repo.ecosystem.as_ref()?;
    let plugin = plugin_for(ecosystem);
    for pattern in plugin.file_patterns() {
        let candidate = repo.path.join(pattern);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn read_version_with_pattern(pattern: &str, content: &str) -> Result<Option<String>> {
    let regex =
        regex::Regex::new(pattern).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    let captures = match regex.captures(content) {
        Some(captures) => captures,
        None => return Ok(None),
    };
    let capture = captures.get(1).ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(
            "version regex must include a capture group"
        ))
    })?;
    Ok(Some(capture.as_str().to_string()))
}

fn read_version_with_path(path: &Path, content: &str, key_path: &str) -> Result<Option<String>> {
    let segments: Vec<&str> = key_path.split('.').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Ok(None);
    }
    let extension = path.extension().and_then(OsStr::to_str).unwrap_or("");

    match extension {
        "toml" => {
            let value: toml::Value = toml::from_str(content)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
            let value = toml_value_at_path(&value, &segments);
            Ok(value.and_then(value_to_string_toml))
        }
        "json" => {
            let value: serde_json::Value = serde_json::from_str(content)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
            let value = json_value_at_path(&value, &segments);
            Ok(value.and_then(value_to_string_json))
        }
        "yaml" | "yml" => {
            let value: serde_yaml::Value = serde_yaml::from_str(content)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
            let value = yaml_value_at_path(&value, &segments);
            Ok(value.and_then(value_to_string_yaml))
        }
        _ => Ok(None),
    }
}

fn toml_value_at_path<'a>(value: &'a toml::Value, path: &[&str]) -> Option<&'a toml::Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn json_value_at_path<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn yaml_value_at_path<'a>(
    value: &'a serde_yaml::Value,
    path: &[&str],
) -> Option<&'a serde_yaml::Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn value_to_string_toml(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(value) => Some(value.clone()),
        toml::Value::Integer(value) => Some(value.to_string()),
        toml::Value::Float(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_to_string_json(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_to_string_yaml(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(value) => Some(value.clone()),
        serde_yaml::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn update_repo_version(repo: &Repo, new_version: &Version, dry_run: bool) -> Result<()> {
    let file = version_file_for_repo(repo).ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "no version file configured for {}",
            repo.id.as_str()
        )))
    })?;
    let content = fs::read_to_string(&file)?;
    let version_cfg = repo
        .config
        .as_ref()
        .and_then(|config| config.versioning.as_ref());

    let updated = if let Some(pattern) = version_cfg.and_then(|cfg| cfg.pattern.as_ref()) {
        update_version_with_pattern(pattern, &content, new_version)?
    } else if let Some(path) = version_cfg.and_then(|cfg| cfg.path.as_ref()) {
        update_version_with_path(&file, &content, path, new_version)?
    } else if let Some(ecosystem) = repo.ecosystem.as_ref() {
        let plugin = plugin_for(ecosystem);
        plugin.update_version(&file, &content, new_version)?
    } else {
        content.clone()
    };

    if dry_run {
        output::info(&format!(
            "would update {} in {}",
            repo.id.as_str(),
            file.display()
        ));
        return Ok(());
    }

    if updated != content {
        fs::write(&file, updated)?;
    }

    Ok(())
}

fn update_version_with_pattern(
    pattern: &str,
    content: &str,
    new_version: &Version,
) -> Result<String> {
    let regex =
        regex::Regex::new(pattern).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    let captures = regex
        .captures(content)
        .ok_or_else(|| HarmoniaError::Other(anyhow::anyhow!("version pattern did not match")))?;
    let capture = captures.get(1).ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(
            "version regex must include a capture group"
        ))
    })?;
    let mut updated = String::new();
    updated.push_str(&content[..capture.start()]);
    updated.push_str(&new_version.raw);
    updated.push_str(&content[capture.end()..]);
    Ok(updated)
}

fn update_version_with_path(
    path: &Path,
    content: &str,
    key_path: &str,
    new_version: &Version,
) -> Result<String> {
    let segments: Vec<&str> = key_path.split('.').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Ok(content.to_string());
    }
    let extension = path.extension().and_then(OsStr::to_str).unwrap_or("");
    match extension {
        "toml" => {
            let mut value: toml::Value = toml::from_str(content)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
            set_toml_value(&mut value, &segments, new_version.raw.clone());
            toml::to_string(&value).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
        }
        "json" => {
            let mut value: serde_json::Value = serde_json::from_str(content)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
            set_json_value(&mut value, &segments, new_version.raw.clone());
            serde_json::to_string_pretty(&value)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
        }
        "yaml" | "yml" => {
            let mut value: serde_yaml::Value = serde_yaml::from_str(content)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
            set_yaml_value(&mut value, &segments, new_version.raw.clone());
            serde_yaml::to_string(&value)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
        }
        _ => Ok(content.to_string()),
    }
}

fn set_toml_value(value: &mut toml::Value, path: &[&str], new_value: String) {
    if path.is_empty() {
        return;
    }
    let mut current = value;
    for segment in &path[..path.len() - 1] {
        if let Some(table) = current.as_table_mut() {
            current = table
                .entry(segment.to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
        } else {
            return;
        }
    }
    if let Some(table) = current.as_table_mut() {
        table.insert(
            path[path.len() - 1].to_string(),
            toml::Value::String(new_value),
        );
    }
}

fn set_json_value(value: &mut serde_json::Value, path: &[&str], new_value: String) {
    if path.is_empty() {
        return;
    }
    let mut current = value;
    for segment in &path[..path.len() - 1] {
        if let Some(object) = current.as_object_mut() {
            current = object
                .entry(segment.to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        } else {
            return;
        }
    }
    if let Some(object) = current.as_object_mut() {
        object.insert(
            path[path.len() - 1].to_string(),
            serde_json::Value::String(new_value),
        );
    }
}

fn set_yaml_value(value: &mut serde_yaml::Value, path: &[&str], new_value: String) {
    if path.is_empty() {
        return;
    }
    let mut current = value;
    for segment in &path[..path.len() - 1] {
        if let Some(mapping) = current.as_mapping_mut() {
            let key = serde_yaml::Value::String(segment.to_string());
            current = mapping
                .entry(key)
                .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        } else {
            return;
        }
    }
    if let Some(mapping) = current.as_mapping_mut() {
        mapping.insert(
            serde_yaml::Value::String(path[path.len() - 1].to_string()),
            serde_yaml::Value::String(new_value),
        );
    }
}

fn build_dependency_updates(
    workspace: &Workspace,
    bumped: &HashMap<RepoId, Version>,
) -> Result<Vec<DependencyUpdate>> {
    let map = package_map(&workspace.repos);
    let mut updates: HashMap<(RepoId, String), String> = HashMap::new();

    for (from, deps) in &workspace.graph.edges {
        let repo = match workspace.repos.get(from) {
            Some(repo) => repo,
            None => continue,
        };
        if repo.ignored || repo.external {
            continue;
        }
        for dep in deps {
            if !dep.is_internal {
                continue;
            }
            let target = match map.get(&dep.name) {
                Some(target) => target,
                None => continue,
            };
            let version = match bumped.get(target) {
                Some(version) => version,
                None => continue,
            };
            let constraint = update_constraint_for_repo(repo, dep, version);
            updates.insert((from.clone(), dep.name.clone()), constraint);
        }
    }

    Ok(updates
        .into_iter()
        .map(|((repo, dependency), constraint)| DependencyUpdate {
            repo,
            dependency,
            constraint,
        })
        .collect())
}

fn update_constraint_for_repo(repo: &Repo, dep: &Dependency, version: &Version) -> String {
    let raw = dep.constraint.raw.trim();
    let default_prefix = match repo.ecosystem.as_ref() {
        Some(EcosystemId::Python) => "==",
        _ => "",
    };
    let prefix = detect_constraint_prefix(raw).unwrap_or(default_prefix);
    format!("{prefix}{}", version.raw)
}

fn detect_constraint_prefix(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.contains(',') {
        return None;
    }
    ["^", "~", "==", "="]
        .into_iter()
        .find(|prefix| trimmed.starts_with(prefix))
}

fn update_dependency_in_repo(
    repo: &Repo,
    dependency: &str,
    constraint: &str,
    dry_run: bool,
) -> Result<()> {
    let file = deps_file_for_repo(repo).ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "no dependency file configured for {}",
            repo.id.as_str()
        )))
    })?;
    let content = fs::read_to_string(&file)?;
    let ecosystem = repo.ecosystem.as_ref().ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "no ecosystem configured for {}",
            repo.id.as_str()
        )))
    })?;
    let plugin = plugin_for(ecosystem);
    let updated = plugin.update_dependency(&file, &content, dependency, constraint)?;

    if dry_run {
        output::info(&format!(
            "would update dependency {} in {}",
            dependency,
            repo.id.as_str()
        ));
        return Ok(());
    }

    if updated != content {
        fs::write(&file, updated)?;
    }
    Ok(())
}

#[derive(Debug)]
struct StatusRow {
    repo: String,
    branch: String,
    status: StatusSummary,
}

fn print_status_table(
    workspace: &Workspace,
    rows: &[StatusRow],
    _short: bool,
    _long: bool,
) -> Result<()> {
    let workspace_name = if workspace.config.workspace.name.is_empty() {
        workspace
            .root
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("workspace")
            .to_string()
    } else {
        workspace.config.workspace.name.clone()
    };
    println!("Workspace: {} ({} repos)", workspace_name, rows.len());
    println!();
    println!("Repo          Branch              Status");
    println!("----------------------------------------");
    for row in rows {
        let status = if row.status.is_clean() {
            "clean".to_string()
        } else {
            format!(
                "{} staged, {} modified, {} untracked",
                row.status.staged.len(),
                row.status.modified.len(),
                row.status.untracked.len()
            )
        };
        println!("{:<12} {:<18} {}", row.repo, row.branch, status);
    }
    Ok(())
}

fn print_status_json(rows: &[StatusRow]) -> Result<()> {
    let json = serde_json::to_string_pretty(
        &rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "repo": row.repo,
                    "branch": row.branch,
                    "staged": row.status.staged.len(),
                    "modified": row.status.modified.len(),
                    "untracked": row.status.untracked.len(),
                    "conflicts": row.status.conflicts.len(),
                })
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    println!("{}", json);
    Ok(())
}
