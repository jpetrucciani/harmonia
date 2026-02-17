use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::Context;
use clap::{Args, CommandFactory, Parser, Subcommand};
use serde::{Deserialize, Serialize};

use crate::config::resolve::resolve_workspace_with_overrides;
use crate::core::changeset::{load_changeset_files, select_active_changeset, ChangesetFile};
use crate::core::repo::{Dependency, Repo, RepoId};
use crate::core::version::{
    bump_version, parse_bump_level, parse_bump_mode, parse_version_kind, BumpMode, Version,
    VersionKind,
};
use crate::core::workspace::Workspace;
use crate::ecosystem::{plugin_for, EcosystemId};
use crate::error::{HarmoniaError, Result};
use crate::forge::traits::{CreateIssueParams, CreateMrParams, MergeMrParams, UpdateMrParams};
use crate::forge::{client_from_forge_config, CiState, MrState};
use crate::git::ops::{
    branch_exists, checkout_branch, clone_repo, create_and_checkout_branch, create_branch,
    current_branch, open_repo, repo_status, set_branch_upstream, sync_repo, SyncOptions,
};
use crate::git::status::StatusSummary;
use crate::graph::constraint::{check_constraints, ConstraintReport, ViolationType};
use crate::graph::ops::{
    internal_dependencies_for, merge_order, package_map, resolve_internal_edges, topological_order,
    transitive_dependencies, transitive_dependents,
};
use crate::graph::viz;
use crate::util::template::render_template_file;
use crate::util::{output, parallel};

#[derive(Parser, Debug)]
#[command(name = "harmonia")]
#[command(about = "Poly-repo orchestrator", long_about = None)]
pub struct Cli {
    #[arg(
        short,
        long,
        help = "Path to workspace root. Overrides auto-discovery."
    )]
    pub workspace: Option<PathBuf>,
    #[arg(
        short,
        long,
        help = "Path to workspace config file. Overrides default config lookup."
    )]
    pub config: Option<PathBuf>,
    #[arg(
        short,
        long,
        action = clap::ArgAction::Count,
        help = "Increase log verbosity (-v, -vv, ...)."
    )]
    pub verbose: u8,
    #[arg(short, long, help = "Suppress non-error output.")]
    pub quiet: bool,
    #[arg(long, help = "Disable colored output.")]
    pub no_color: bool,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(
        about = "Initialize a workspace, optionally seeding it from a source repository or workspace."
    )]
    Init(InitArgs),
    #[command(
        about = "Clone selected repos from workspace config into the local repos directory."
    )]
    Clone(CloneArgs),
    #[command(about = "Show per-repo branch, ahead/behind, and working-tree status.")]
    Status(StatusArgs),
    #[command(
        about = "Fetch and integrate upstream changes across selected repos.",
        long_about = "Fetch and integrate upstream changes across selected repos.\n\nDefault behavior:\n  - fetch from the repository's configured upstream remote\n  - if the current branch can fast-forward, advance it\n  - if histories diverged, create a merge commit with --no-edit\n  - if already up to date, leave the branch unchanged\n\nSafety behavior:\n  - by default, branch updates require a clean working tree\n  - use --autostash to stash local changes before updating and re-apply them after\n  - use --fetch-only to only fetch remote updates without changing local branches"
    )]
    Sync(SyncArgs),
    #[command(about = "Switch all repos back to main/master and fast-forward from upstream.")]
    Refresh(RefreshArgs),
    #[command(about = "Create MRs, stage, commit, and push changed repos in one command.")]
    Submit(SubmitArgs),
    #[command(about = "Run an arbitrary command in each selected repository.")]
    Exec(ExecArgs),
    #[command(about = "Run a configured hook across selected repositories.")]
    Run(RunArgs),
    #[command(about = "Run a command in each selected repository, optionally through a shell.")]
    Each(EachArgs),
    #[command(about = "Inspect dependency relationships between repositories.")]
    Graph(GraphArgs),
    #[command(
        about = "Create or switch branches across repositories with optional dependency expansion."
    )]
    Branch(BranchArgs),
    #[command(about = "Checkout a branch across selected repositories.")]
    Checkout(CheckoutArgs),
    #[command(about = "Stage changes across repositories via git add.")]
    Add(AddArgs),
    #[command(about = "Create commits across selected repositories.")]
    Commit(CommitArgs),
    #[command(about = "Push selected repositories, with optional force and upstream settings.")]
    Push(PushArgs),
    #[command(about = "Show git diffs across selected repositories.")]
    Diff(DiffArgs),
    #[command(about = "Run ecosystem test commands across selected repositories.")]
    Test(TestArgs),
    #[command(about = "Run ecosystem lint commands across selected repositories.")]
    Lint(LintArgs),
    #[command(about = "Inspect, validate, and bump repository versions.")]
    Version(VersionArgs),
    #[command(about = "Inspect and update repository dependency declarations.")]
    Deps(DepsArgs),
    #[command(about = "Open workspace or repository paths in your editor.")]
    Edit(EditArgs),
    #[command(about = "Clean untracked files and directories with git clean.")]
    Clean(CleanArgs),
    #[command(about = "Show and edit workspace configuration values.")]
    Config(ConfigArgs),
    #[command(about = "List, add, remove, and inspect repositories in workspace config.")]
    Repo(RepoArgs),
    #[command(about = "Build a cross-repo execution and merge plan from current changes.")]
    Plan(PlanArgs),
    #[command(about = "Create, inspect, update, merge, and close merge requests.")]
    Mr(MrArgs),
    #[command(about = "Generate shell completion scripts.")]
    Completion(CompletionArgs),
    #[command(
        about = "Open a workspace-aware shell or run one shell command with workspace context."
    )]
    Shell(ShellArgs),
}

#[derive(Args, Debug)]
pub struct InitArgs {
    #[arg(help = "Optional source URL/path to seed the workspace from.")]
    pub source: Option<String>,
    #[arg(short = 'n', long, help = "Workspace name to write into config.")]
    pub name: Option<String>,
    #[arg(short = 'd', long, help = "Target directory for the workspace.")]
    pub directory: Option<PathBuf>,
    #[arg(long, help = "Create workspace layout without cloning repos.")]
    pub no_clone: bool,
    #[arg(long, help = "Initial repo group to clone after init.")]
    pub group: Option<String>,
}

#[derive(Args, Debug)]
pub struct CloneArgs {
    #[arg(help = "Specific repositories to clone.")]
    pub repos: Vec<String>,
    #[arg(
        short = 'g',
        long,
        help = "Clone repositories from this configured group."
    )]
    pub group: Option<String>,
    #[arg(
        short = 'a',
        long,
        help = "Clone all repositories in workspace config."
    )]
    pub all: bool,
    #[arg(long, help = "Shallow clone depth or 'full' to disable shallow clone.")]
    pub depth: Option<String>,
    #[arg(long, help = "Disable shallow clone and fetch full history.")]
    pub full: bool,
    #[arg(long, help = "Preferred clone protocol: ssh or https.")]
    pub protocol: Option<String>,
    #[arg(long, help = "Fail when repo path already exists instead of skipping.")]
    pub strict: bool,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(
        short = 's',
        long,
        help = "Print compact one-line status per repository."
    )]
    pub short: bool,
    #[arg(short = 'l', long, help = "Print full git status per repository.")]
    pub long: bool,
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
    #[arg(long, help = "Show only repositories with local changes.")]
    pub changed: bool,
    #[arg(long, help = "Emit tab-delimited porcelain-style output.")]
    pub porcelain: bool,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    #[arg(help = "Specific repositories to sync.")]
    pub repos: Vec<String>,
    #[arg(
        short = 'r',
        long,
        help = "Rebase local branch onto upstream after fetch."
    )]
    pub rebase: bool,
    #[arg(
        long = "ff-only",
        help = "Only fast-forward, fail if merge/rebase is required."
    )]
    pub ff_only: bool,
    #[arg(
        short = 'f',
        long = "fetch-only",
        help = "Fetch remotes without integrating changes."
    )]
    pub fetch_only: bool,
    #[arg(
        long,
        help = "Temporarily stash local changes before update and re-apply them after."
    )]
    pub autostash: bool,
    #[arg(
        short = 'p',
        long,
        help = "Prune stale remote-tracking branches while fetching."
    )]
    pub prune: bool,
    #[arg(long, help = "Number of repositories to sync in parallel.")]
    pub parallel: Option<usize>,
}

#[derive(Args, Debug, Default)]
pub struct RefreshArgs;

#[derive(Args, Debug, Default)]
pub struct SubmitArgs {
    #[arg(
        short = 'm',
        long,
        help = "Commit message for submit flow. Defaults to 'updates'."
    )]
    pub message: Option<String>,
    #[arg(
        long,
        help = "Disable auto-branching before MR creation in submit flow."
    )]
    pub no_auto_branch: bool,
    #[arg(long, help = "Branch name to use for auto-branching in submit flow.")]
    pub branch_name: Option<String>,
}

#[derive(Args, Debug)]
pub struct ExecArgs {
    #[arg(long, help = "Comma-separated repositories to target.")]
    pub repos: Vec<String>,
    #[arg(long, help = "Run on all configured repositories.")]
    pub all: bool,
    #[arg(long, help = "Run only on repositories with local changes.")]
    pub changed: bool,
    #[arg(long, help = "Number of repositories to run in parallel.")]
    pub parallel: Option<usize>,
    #[arg(long, help = "Stop after first command failure.")]
    pub fail_fast: bool,
    #[arg(long, help = "Continue even when commands fail.")]
    pub ignore_errors: bool,
    #[arg(
        last = true,
        required = true,
        help = "Command to execute in each repository."
    )]
    pub command: Vec<String>,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[arg(help = "Hook name to execute (workspace and repo hook precedence applies).")]
    pub hook: String,
    #[arg(long, help = "Comma-separated repositories to target.")]
    pub repos: Vec<String>,
    #[arg(long, help = "Run on all configured repositories.")]
    pub all: bool,
    #[arg(long, help = "Run only on repositories with local changes.")]
    pub changed: bool,
    #[arg(long, help = "Number of repositories to run in parallel.")]
    pub parallel: Option<usize>,
    #[arg(long, help = "Stop after first hook failure.")]
    pub fail_fast: bool,
}

#[derive(Args, Debug)]
pub struct EachArgs {
    #[arg(long, help = "Comma-separated repositories to target.")]
    pub repos: Vec<String>,
    #[arg(long, help = "Number of repositories to run in parallel.")]
    pub parallel: Option<usize>,
    #[arg(long, help = "Run command through shell (sh -c / cmd /C).")]
    pub shell: bool,
    #[arg(
        last = true,
        required = true,
        help = "Command to run in each repository."
    )]
    pub command: Vec<String>,
}

#[derive(Args, Debug)]
pub struct GraphArgs {
    #[command(subcommand)]
    pub command: Option<GraphCommand>,
}

#[derive(Subcommand, Debug)]
pub enum GraphCommand {
    #[command(about = "Render the repository dependency graph.")]
    Show(GraphShowArgs),
    #[command(about = "List direct or transitive dependencies for a repository.")]
    Deps(GraphDepsArgs),
    #[command(about = "List direct or transitive dependents for a repository.")]
    Dependents(GraphDependentsArgs),
    #[command(about = "Compute a dependency-safe execution order.")]
    Order(GraphOrderArgs),
    #[command(about = "Validate dependency constraints and optionally auto-fix known issues.")]
    Check(GraphCheckArgs),
}

#[derive(Args, Debug)]
pub struct GraphShowArgs {
    #[arg(long, help = "Limit graph output to repositories with local changes.")]
    pub changed: bool,
    #[arg(
        long,
        default_value = "tree",
        help = "Output format: tree, mermaid, dot, or json."
    )]
    pub format: String,
    #[arg(
        long,
        default_value = "down",
        help = "Traversal direction: down (dependencies) or up (dependents)."
    )]
    pub direction: String,
}

#[derive(Args, Debug)]
pub struct GraphDepsArgs {
    #[arg(help = "Repository whose dependencies should be shown.")]
    pub repo: String,
    #[arg(short = 't', long, help = "Include transitive dependencies.")]
    pub transitive: bool,
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct GraphDependentsArgs {
    #[arg(help = "Repository whose dependents should be shown.")]
    pub repo: String,
    #[arg(short = 't', long, help = "Include transitive dependents.")]
    pub transitive: bool,
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct GraphOrderArgs {
    #[arg(long, help = "Limit order to repositories with local changes.")]
    pub changed: bool,
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct GraphCheckArgs {
    #[arg(long, help = "Apply safe, automatic fixes for detected violations.")]
    pub fix: bool,
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct BranchArgs {
    #[arg(help = "Branch name to checkout/create in selected repositories.")]
    pub name: String,
    #[arg(short = 'c', long, help = "Create branch if missing before checkout.")]
    pub create: bool,
    #[arg(
        short = 'C',
        long = "force-create",
        help = "Force-create/reset branch before checkout."
    )]
    pub force_create: bool,
    #[arg(
        long,
        help = "Skip confirmation prompts for destructive branch actions."
    )]
    pub yes: bool,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to target."
    )]
    pub repos: Vec<String>,
    #[arg(long, help = "Limit target set to repositories with local changes.")]
    pub changed: bool,
    #[arg(
        long,
        help = "Include transitive dependencies of selected repositories."
    )]
    pub with_deps: bool,
    #[arg(
        long,
        help = "Include full dependency closure for selected repositories."
    )]
    pub with_all_deps: bool,
    #[arg(
        short = 't',
        long,
        help = "Set upstream tracking target after checkout."
    )]
    pub track: Option<String>,
}

#[derive(Args, Debug)]
pub struct CheckoutArgs {
    #[arg(help = "Branch name to checkout.")]
    pub branch: String,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to target."
    )]
    pub repos: Vec<String>,
    #[arg(long, help = "Target all configured repositories.")]
    pub all: bool,
    #[arg(long, help = "Skip repositories where checkout cannot be completed.")]
    pub graceful: bool,
    #[arg(
        long,
        help = "Fallback branch to try if the requested branch does not exist."
    )]
    pub fallback: Option<String>,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to target."
    )]
    pub repos: Vec<String>,
    #[arg(
        short = 'A',
        long,
        help = "Stage all changes (equivalent to git add -A)."
    )]
    pub all: bool,
    #[arg(short = 'p', long, help = "Interactively select hunks to stage.")]
    pub patch: bool,
    #[arg(last = true, help = "Optional pathspecs passed through to git add.")]
    pub pathspec: Vec<String>,
}

#[derive(Args, Debug)]
pub struct CommitArgs {
    #[arg(short = 'm', long, help = "Commit message to use.")]
    pub message: Option<String>,
    #[arg(short = 'a', long, help = "Stage tracked changes before committing.")]
    pub all: bool,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to target."
    )]
    pub repos: Vec<String>,
    #[arg(long, help = "Amend the previous commit.")]
    pub amend: bool,
    #[arg(long, help = "Skip workspace/repo pre-commit hooks.")]
    pub no_hooks: bool,
    #[arg(long, help = "Skip confirmation prompts.")]
    pub yes: bool,
    #[arg(long, help = "Allow creating empty commits.")]
    pub allow_empty: bool,
    #[arg(
        long = "trailer",
        help = "Add one or more commit trailers (key=value or raw)."
    )]
    pub trailers: Vec<String>,
}

#[derive(Args, Debug)]
pub struct PushArgs {
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to target."
    )]
    pub repos: Vec<String>,
    #[arg(
        short = 'f',
        long,
        help = "Force push (unsafe, may rewrite remote history)."
    )]
    pub force: bool,
    #[arg(
        long = "force-with-lease",
        help = "Force push with lease protection against remote divergence."
    )]
    pub force_with_lease: bool,
    #[arg(
        short = 'u',
        long = "set-upstream",
        help = "Set upstream tracking on push."
    )]
    pub set_upstream: bool,
    #[arg(long, help = "Skip workspace/repo pre-push hooks.")]
    pub no_hooks: bool,
    #[arg(long, help = "Skip confirmation prompts for force pushes.")]
    pub yes: bool,
    #[arg(long, help = "Show what would be pushed without pushing.")]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct DiffArgs {
    #[arg(help = "Specific repositories to diff. Defaults to changed repos.")]
    pub repos: Vec<String>,
    #[arg(long, help = "Diff staged changes instead of working tree changes.")]
    pub staged: bool,
    #[arg(long, help = "Show summary statistics instead of full patch.")]
    pub stat: bool,
    #[arg(long = "name-only", help = "Show only changed file names.")]
    pub name_only: bool,
    #[arg(long, help = "Number of context lines to include in patch output.")]
    pub unified: Option<u32>,
    #[arg(
        long,
        default_value = "patch",
        help = "Output format: patch, name-only, or json."
    )]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct TestArgs {
    #[arg(help = "Specific repositories to test.")]
    pub repos: Vec<String>,
    #[arg(long, help = "Run tests for all configured repositories.")]
    pub all: bool,
    #[arg(long, help = "Run tests only for repositories with local changes.")]
    pub changed: bool,
    #[arg(
        long = "graph-order",
        help = "Run repositories in dependency-safe graph order."
    )]
    pub graph_order: bool,
    #[arg(long, help = "Number of repositories to run in parallel.")]
    pub parallel: Option<usize>,
    #[arg(long, help = "Stop after first test failure.")]
    pub fail_fast: bool,
    #[arg(
        long,
        help = "Enable coverage mode when supported by ecosystem plugin."
    )]
    pub coverage: bool,
    #[arg(
        short = 'k',
        long = "filter",
        help = "Filter expression forwarded to ecosystem test command when supported."
    )]
    pub filter: Option<String>,
}

#[derive(Args, Debug)]
pub struct LintArgs {
    #[arg(help = "Specific repositories to lint.")]
    pub repos: Vec<String>,
    #[arg(long, help = "Run lint for all configured repositories.")]
    pub all: bool,
    #[arg(long, help = "Run lint only for repositories with local changes.")]
    pub changed: bool,
    #[arg(long, help = "Apply auto-fixes where supported by ecosystem plugin.")]
    pub fix: bool,
    #[arg(long, help = "Number of repositories to run in parallel.")]
    pub parallel: Option<usize>,
}

#[derive(Args, Debug)]
pub struct VersionArgs {
    #[command(subcommand)]
    pub command: Option<VersionCommand>,
}

#[derive(Subcommand, Debug)]
pub enum VersionCommand {
    #[command(about = "Show detected versions for repositories.")]
    Show(VersionShowArgs),
    #[command(about = "Check version constraints between repositories.")]
    Check(VersionCheckArgs),
    #[command(about = "Bump versions and update dependents using configured strategies.")]
    Bump(VersionBumpArgs),
}

#[derive(Args, Debug)]
pub struct VersionShowArgs {
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
    #[arg(long, help = "Include dependency context per repository.")]
    pub with_deps: bool,
}

#[derive(Args, Debug)]
pub struct VersionCheckArgs {
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct VersionBumpArgs {
    #[arg(help = "Bump level (patch, minor, major) when not derived from changesets.")]
    pub level: Option<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to target."
    )]
    pub repos: Vec<String>,
    #[arg(long, help = "Target only repositories with local changes.")]
    pub changed: bool,
    #[arg(
        long,
        help = "Bump mode override (for example independent or lockstep)."
    )]
    pub mode: Option<String>,
    #[arg(long, help = "Preview changes without writing files.")]
    pub dry_run: bool,
    #[arg(long, help = "Cascade bumps to downstream dependents.")]
    pub cascade: bool,
    #[arg(long, help = "Optional prerelease tag for bumped versions.")]
    pub pre: Option<String>,
}

#[derive(Args, Debug)]
pub struct DepsArgs {
    #[command(subcommand)]
    pub command: Option<DepsCommand>,
}

#[derive(Subcommand, Debug)]
pub enum DepsCommand {
    #[command(about = "Show resolved internal dependencies for repositories.")]
    Show(DepsShowArgs),
    #[command(about = "Validate dependency declarations against workspace rules.")]
    Check(DepsCheckArgs),
    #[command(about = "Update dependency files with new package version constraints.")]
    Update(DepsUpdateArgs),
}

#[derive(Args, Debug)]
pub struct DepsShowArgs {
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DepsCheckArgs {
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DepsUpdateArgs {
    #[arg(help = "Package names to update constraints for.")]
    pub packages: Vec<String>,
    #[arg(long, help = "Preview updates without writing files.")]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    #[arg(help = "Specific repositories to open.")]
    pub repos: Vec<String>,
    #[arg(
        long,
        help = "Editor command to use (defaults to $EDITOR then 'code')."
    )]
    pub editor: Option<String>,
    #[arg(long, help = "Open changed repositories instead of workspace root.")]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct CleanArgs {
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to clean."
    )]
    pub repos: Vec<String>,
    #[arg(
        short = 'f',
        long,
        help = "Required to perform clean; without this nothing is removed."
    )]
    pub force: bool,
    #[arg(short = 'd', long, help = "Also remove untracked directories.")]
    pub directories: bool,
    #[arg(short = 'x', long, help = "Also remove files ignored by .gitignore.")]
    pub ignored: bool,
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: Option<ConfigCommand>,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    #[command(about = "Print the effective workspace configuration TOML.")]
    Show,
    #[command(about = "Read a single config value by dotted key path.")]
    Get(ConfigGetArgs),
    #[command(about = "Set a config value by dotted key path.")]
    Set(ConfigSetArgs),
    #[command(about = "Open the workspace config file in an editor.")]
    Edit(ConfigEditArgs),
}

#[derive(Args, Debug)]
pub struct ConfigGetArgs {
    #[arg(help = "Dotted config path, for example workspace.name or defaults.clone_protocol.")]
    pub key: String,
}

#[derive(Args, Debug)]
pub struct ConfigSetArgs {
    #[arg(help = "Dotted config path to write.")]
    pub key: String,
    #[arg(help = "New value. Parsed as TOML when possible, otherwise stored as string.")]
    pub value: String,
}

#[derive(Args, Debug)]
pub struct ConfigEditArgs {
    #[arg(
        long,
        help = "Editor command to use (defaults to $EDITOR then 'code')."
    )]
    pub editor: Option<String>,
}

#[derive(Args, Debug)]
pub struct RepoArgs {
    #[command(subcommand)]
    pub command: Option<RepoCommand>,
}

#[derive(Subcommand, Debug)]
pub enum RepoCommand {
    #[command(about = "List repositories defined in workspace config.")]
    List,
    #[command(about = "Add a repository entry to workspace config.")]
    Add(RepoAddArgs),
    #[command(about = "Remove a repository entry from workspace config.")]
    Remove(RepoRemoveArgs),
    #[command(about = "Show repository details from workspace config.")]
    Show(RepoShowArgs),
}

#[derive(Args, Debug)]
pub struct RepoAddArgs {
    #[arg(help = "Repository key in [repos].")]
    pub name: String,
    #[arg(long, help = "Explicit clone URL.")]
    pub url: Option<String>,
    #[arg(long = "default-branch", help = "Default branch for this repository.")]
    pub default_branch: Option<String>,
    #[arg(
        long = "package-name",
        help = "Internal package name used in dependency mapping."
    )]
    pub package_name: Option<String>,
    #[arg(long, help = "Mark repository as external to the workspace.")]
    pub external: bool,
    #[arg(long, help = "Mark repository as ignored by orchestration commands.")]
    pub ignored: bool,
    #[arg(long, help = "Optional group name to place this repository into.")]
    pub group: Option<String>,
}

#[derive(Args, Debug)]
pub struct RepoRemoveArgs {
    #[arg(help = "Repository key to remove from workspace config.")]
    pub name: String,
}

#[derive(Args, Debug)]
pub struct RepoShowArgs {
    #[arg(help = "Repository key to inspect.")]
    pub name: String,
}

#[derive(Args, Debug)]
pub struct PlanArgs {
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to force-include."
    )]
    pub include: Vec<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to exclude."
    )]
    pub exclude: Vec<String>,
}

#[derive(Args, Debug)]
pub struct MrArgs {
    #[command(subcommand)]
    pub command: Option<MrCommand>,
}

#[derive(Subcommand, Debug)]
pub enum MrCommand {
    #[command(about = "Create merge requests for selected repositories.")]
    Create(MrCreateArgs),
    #[command(about = "Show merge request and CI status, optionally waiting for required checks.")]
    Status(MrStatusArgs),
    #[command(about = "Update merge request metadata such as description and labels.")]
    Update(MrUpdateArgs),
    #[command(about = "Merge merge requests when policy and checks permit.")]
    Merge(MrMergeArgs),
    #[command(about = "Close open merge requests without merging.")]
    Close(MrCloseArgs),
}

#[derive(Args, Debug, Default)]
pub struct MrCreateArgs {
    #[arg(short = 't', long, help = "Merge request title override.")]
    pub title: Option<String>,
    #[arg(short = 'd', long, help = "Merge request description override.")]
    pub description: Option<String>,
    #[arg(long, help = "Create MRs as draft.")]
    pub draft: bool,
    #[arg(
        long = "no-link",
        help = "Skip linking related MRs/issues in descriptions."
    )]
    pub no_link: bool,
    #[arg(long = "no-issue", help = "Skip creating a tracking issue.")]
    pub no_issue: bool,
    #[arg(long, value_delimiter = ',', help = "Comma-separated labels to apply.")]
    pub labels: Vec<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated reviewer usernames."
    )]
    pub reviewers: Vec<String>,
    #[arg(
        long,
        help = "Automatically create and switch changed default-branch repos onto a feature branch before creating MRs."
    )]
    pub auto_branch: bool,
    #[arg(
        long,
        help = "Branch name used with --auto-branch. Defaults to active changeset branch, then a generated feature/harmonia-<timestamp> name."
    )]
    pub branch_name: Option<String>,
    #[arg(long, help = "Preview MR payloads without calling forge APIs.")]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct MrStatusArgs {
    #[arg(long, help = "Emit machine-readable JSON output.")]
    pub json: bool,
    #[arg(long, help = "Wait for required checks until success/failure/timeout.")]
    pub wait: bool,
    #[arg(
        long,
        default_value_t = 30,
        help = "Wait timeout in minutes when --wait is enabled."
    )]
    pub timeout: u64,
}

#[derive(Args, Debug, Default)]
pub struct MrUpdateArgs {
    #[arg(short = 'd', long, help = "New MR description.")]
    pub description: Option<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated labels to apply/replace."
    )]
    pub labels: Vec<String>,
}

#[derive(Args, Debug, Default)]
pub struct MrMergeArgs {
    #[arg(long, help = "Preview merge actions without calling forge APIs.")]
    pub dry_run: bool,
    #[arg(
        long = "no-wait",
        help = "Do not wait for required checks before merge."
    )]
    pub no_wait: bool,
    #[arg(long, help = "Request squash merge when forge supports it.")]
    pub squash: bool,
    #[arg(long = "delete-branch", help = "Delete source branches after merge.")]
    pub delete_branch: bool,
    #[arg(short = 'y', long, help = "Skip confirmation prompts.")]
    pub yes: bool,
}

#[derive(Args, Debug, Default)]
pub struct MrCloseArgs {
    #[arg(short = 'y', long, help = "Skip confirmation prompts.")]
    pub yes: bool,
}

#[derive(Args, Debug, Default)]
pub struct ShellArgs {
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated repositories to include in context."
    )]
    pub repos: Vec<String>,
    #[arg(
        long,
        help = "Run one command then exit instead of starting interactive shell."
    )]
    pub command: Option<String>,
}

#[derive(Args, Debug)]
pub struct CompletionArgs {
    #[arg(value_enum, help = "Target shell to generate completion script for.")]
    pub shell: clap_complete::Shell,
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
        Commands::Refresh(args) => handle_refresh(args, cli.workspace, cli.config),
        Commands::Submit(args) => handle_submit(args, cli.workspace, cli.config),
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
        Commands::Test(args) => handle_test(args, cli.workspace, cli.config),
        Commands::Lint(args) => handle_lint(args, cli.workspace, cli.config),
        Commands::Version(args) => handle_version(args, cli.workspace, cli.config),
        Commands::Deps(args) => handle_deps(args, cli.workspace, cli.config),
        Commands::Edit(args) => handle_edit(args, cli.workspace, cli.config),
        Commands::Clean(args) => handle_clean(args, cli.workspace, cli.config),
        Commands::Config(args) => handle_config(args, cli.workspace, cli.config),
        Commands::Repo(args) => handle_repo(args, cli.workspace, cli.config),
        Commands::Plan(args) => handle_plan(args, cli.workspace, cli.config),
        Commands::Mr(args) => handle_mr(args, cli.workspace, cli.config),
        Commands::Completion(args) => handle_completion(args),
        Commands::Shell(args) => handle_shell(args, cli.workspace, cli.config),
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
            strict: false,
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
    let default_depth = workspace
        .config
        .defaults
        .as_ref()
        .and_then(|defaults| defaults.clone_depth.as_deref());
    let depth = parse_depth(args.depth.as_deref(), args.full, default_depth)?;
    let protocol = resolve_clone_protocol(args.protocol.as_deref(), &workspace)?;
    let jobs = resolve_parallel(None);

    let results = parallel::run_in_parallel(repos, jobs, |repo| {
        if repo.remote_url.is_empty() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} missing url",
                repo.id.as_str()
            ))));
        }

        if !args.strict && repo.path.exists() {
            output::warn(&format!(
                "skipping {} because {} already exists",
                repo.id.as_str(),
                repo.path.display()
            ));
            return Ok(());
        }

        let clone_url = resolve_clone_url(&repo.remote_url, protocol.as_deref());
        if let Some(parent) = repo.path.parent() {
            crate::git::ops::ensure_repo_dir(parent)?;
        }
        output::git_op(&format!("clone {} {}", clone_url, repo.path.display()));
        clone_repo(&clone_url, &repo.path, depth)
    });

    for result in results {
        result?;
    }

    Ok(())
}

fn parse_depth(
    depth: Option<&str>,
    full: bool,
    default_depth: Option<&str>,
) -> Result<Option<u32>> {
    if full {
        return Ok(None);
    }
    let depth = match depth.or(default_depth) {
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

fn resolve_clone_protocol(input: Option<&str>, workspace: &Workspace) -> Result<Option<String>> {
    let protocol = input.or_else(|| {
        workspace
            .config
            .defaults
            .as_ref()
            .and_then(|defaults| defaults.clone_protocol.as_deref())
    });

    let Some(protocol) = protocol else {
        return Ok(None);
    };

    let normalized = protocol.to_ascii_lowercase();
    match normalized.as_str() {
        "ssh" | "https" => Ok(Some(normalized)),
        _ => Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "unsupported clone protocol '{}'; expected 'ssh' or 'https'",
            protocol
        )))),
    }
}

fn resolve_clone_url(remote_url: &str, protocol: Option<&str>) -> String {
    match protocol {
        Some("https") => to_https_url(remote_url).unwrap_or_else(|| remote_url.to_string()),
        Some("ssh") => to_ssh_url(remote_url).unwrap_or_else(|| remote_url.to_string()),
        _ => remote_url.to_string(),
    }
}

fn to_https_url(remote_url: &str) -> Option<String> {
    if remote_url.starts_with("http://") || remote_url.starts_with("https://") {
        return Some(remote_url.to_string());
    }
    if remote_url.starts_with("file://") {
        return None;
    }
    if let Some(rest) = remote_url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some(format!("https://{host}/{path}"));
    }
    if let Some(rest) = remote_url.strip_prefix("ssh://git@") {
        let (host, path) = rest.split_once('/')?;
        return Some(format!("https://{host}/{path}"));
    }
    None
}

fn to_ssh_url(remote_url: &str) -> Option<String> {
    if remote_url.starts_with("git@") || remote_url.starts_with("ssh://git@") {
        return Some(remote_url.to_string());
    }
    if remote_url.starts_with("file://") {
        return None;
    }
    let stripped = remote_url
        .strip_prefix("https://")
        .or_else(|| remote_url.strip_prefix("http://"))?;
    let (host, path) = stripped.split_once('/')?;
    Some(format!("git@{host}:{path}"))
}

fn handle_status(
    args: StatusArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let include_untracked = include_untracked_by_default(&workspace);
    let mut repos = select_repos(&workspace, &[], None, true, false)?;
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    let mut rows = Vec::new();
    for repo in repos {
        if !repo.path.is_dir() {
            continue;
        }
        let open = open_repo(&repo.path)?;
        let branch = current_branch(&open.repo)?;
        let mut status = repo_status(&open.repo)?;
        if !include_untracked {
            status.untracked.clear();
        }
        let (ahead, behind) = ahead_behind_for_repo(&repo.path);
        if args.changed && status.is_clean() {
            continue;
        }
        rows.push(StatusRow {
            repo: repo.id.as_str().to_string(),
            path: repo.path.clone(),
            branch,
            ahead,
            behind,
            status,
        });
    }

    if args.json {
        print_status_json(&rows)?;
        return Ok(());
    }
    if args.porcelain {
        print_status_porcelain(&rows);
        return Ok(());
    }
    if args.long {
        print_status_long(&rows, include_untracked)?;
        return Ok(());
    }

    print_status_table(&workspace, &rows, args.short)?;
    Ok(())
}

fn handle_sync(
    args: SyncArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let mut repos = select_repos(&workspace, &args.repos, None, args.repos.is_empty(), false)?;
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    let jobs = resolve_parallel(args.parallel);

    let results = parallel::run_in_parallel(repos, jobs, |repo| {
        let repo_name = repo.id.as_str().to_string();
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "{}: repository is not cloned",
                repo_name
            ))));
        }
        let open = open_repo(&repo.path)?;
        output::git_op(&format!("fetch (repo {})", repo_name));
        let outcome = sync_repo(
            &open.repo,
            SyncOptions {
                fetch_only: args.fetch_only,
                ff_only: args.ff_only,
                rebase: args.rebase,
                autostash: args.autostash,
                prune: args.prune,
            },
        )
        .map_err(|err| HarmoniaError::Other(anyhow::anyhow!(format!("{repo_name}: {err}"))))?;
        Ok((repo_name, outcome))
    });

    let mut failures = Vec::new();
    for result in results {
        match result {
            Ok((repo_name, outcome)) => {
                if args.fetch_only {
                    output::git_op(&format!("fetched (repo {})", repo_name));
                } else if outcome.fast_forwarded {
                    output::git_op(&format!("fast-forward (repo {})", repo_name));
                } else if outcome.rebased {
                    output::git_op(&format!("rebase (repo {})", repo_name));
                } else if outcome.merged {
                    output::git_op(&format!("merge (repo {})", repo_name));
                } else {
                    output::git_op(&format!("up-to-date (repo {})", repo_name));
                }
                if outcome.autostashed {
                    output::info(&format!(
                        "autostash reapplied local changes in {}",
                        repo_name
                    ));
                }
                if outcome.pruned > 0 {
                    output::info(&format!(
                        "pruned {} stale refs in {}",
                        outcome.pruned, repo_name
                    ));
                }
            }
            Err(err) => failures.push(err.to_string()),
        }
    }

    if !failures.is_empty() {
        for failure in &failures {
            output::error(failure);
        }
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "sync failed in {} repositories",
            failures.len()
        ))));
    }

    Ok(())
}

fn handle_refresh(
    _args: RefreshArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    output::info("refresh: checking out main/master across repositories");
    handle_checkout(
        CheckoutArgs {
            branch: "main".to_string(),
            repos: Vec::new(),
            all: true,
            graceful: true,
            fallback: Some("master".to_string()),
        },
        workspace_root.clone(),
        config_path.clone(),
    )?;

    output::info("refresh: syncing latest upstream changes");
    handle_sync(
        SyncArgs {
            repos: Vec::new(),
            rebase: false,
            ff_only: true,
            fetch_only: false,
            autostash: true,
            prune: false,
            parallel: None,
        },
        workspace_root,
        config_path,
    )
}

fn handle_submit(
    args: SubmitArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root.clone(), config_path.clone())?;
    let plan = build_plan_summary(&workspace, &[], &[])?;
    if plan.changed.is_empty() {
        output::info("no changed repositories detected; nothing to submit");
        return Ok(());
    }

    let target_repos: Vec<String> = ordered_plan_repos(&plan)
        .into_iter()
        .map(|repo| repo.as_str().to_string())
        .collect();
    let commit_message = args.message.unwrap_or_else(|| "updates".to_string());
    let mr_args = MrCreateArgs {
        auto_branch: !args.no_auto_branch,
        branch_name: args.branch_name,
        ..MrCreateArgs::default()
    };

    output::info("submit: creating merge requests");
    handle_mr_create(mr_args, &workspace)?;

    output::info("submit: staging changes");
    handle_add(
        AddArgs {
            repos: target_repos.clone(),
            all: false,
            patch: false,
            pathspec: Vec::new(),
        },
        workspace_root.clone(),
        config_path.clone(),
    )?;

    output::info("submit: committing changes");
    handle_commit(
        CommitArgs {
            message: Some(commit_message),
            all: false,
            repos: target_repos.clone(),
            amend: false,
            no_hooks: false,
            yes: false,
            allow_empty: false,
            trailers: Vec::new(),
        },
        workspace_root.clone(),
        config_path.clone(),
    )?;

    output::info("submit: pushing branches");
    handle_push(
        PushArgs {
            repos: target_repos,
            force: false,
            force_with_lease: false,
            set_upstream: true,
            no_hooks: false,
            yes: false,
            dry_run: false,
        },
        workspace_root,
        config_path,
    )
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
    let mut repos = select_repos(&workspace, &args.repos, None, false, false)?;
    if args.changed {
        repos = filter_changed_repos(repos)?;
    }
    if args.with_deps || args.with_all_deps {
        repos = expand_branch_scope(&workspace, repos, args.with_deps, args.with_all_deps);
    }
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    if repos.is_empty() {
        output::info("no repos selected for branch");
        return Ok(());
    }

    for repo in repos {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        let open = open_repo(&repo.path)?;
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
        if let Some(track) = args.track.as_ref() {
            output::git_op(&format!(
                "branch --set-upstream-to {} {} (repo {})",
                track,
                args.name,
                repo.id.as_str()
            ));
            set_branch_upstream(&open.repo, &args.name, track)?;
        }
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
        log_git_command_for_repo(repo.id.as_str(), &cmd);
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
            log_git_command_for_repo(repo.id.as_str(), &cmd);
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
        log_git_command_for_repo(repo.id.as_str(), &cmd);
        if args.message.is_none() {
            output::info(&format!(
                "opening commit message editor (repo {})",
                repo.id.as_str()
            ));
        }
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
        log_git_command_for_repo(repo.id.as_str(), &cmd);
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
    let include_untracked = include_untracked_by_default(&workspace);
    let default_changed = args.repos.is_empty();
    let mut repos = select_repos(&workspace, &args.repos, None, default_changed, false)?;

    if default_changed {
        repos = filter_changed_repos(repos)?;
    }

    if args.format.eq_ignore_ascii_case("json") {
        let mut entries = Vec::new();
        for repo in repos {
            let files =
                git_diff_files(&repo.path, repo.id.as_str(), args.staged, include_untracked)?;
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
    let format = args.format.to_ascii_lowercase();
    if args.name_only || format == "name-only" {
        for repo in repos {
            if multi {
                println!("== {} ==", repo.id.as_str());
            }
            let files =
                git_diff_files(&repo.path, repo.id.as_str(), args.staged, include_untracked)?;
            for file in files {
                println!("{}", file);
            }
        }
        return Ok(());
    }

    for repo in repos {
        if multi {
            println!("== {} ==", repo.id.as_str());
        }
        let cmd = build_diff_command(&args);
        log_git_command_for_repo(repo.id.as_str(), &cmd);
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

fn handle_edit(
    args: EditArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    if args.all && !args.repos.is_empty() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "--all cannot be combined with explicit repos"
        )));
    }

    let workspace = load_workspace(workspace_root, config_path)?;
    let targets: Vec<PathBuf> = if !args.repos.is_empty() {
        select_repos(&workspace, &args.repos, None, false, false)?
            .into_iter()
            .map(|repo| repo.path)
            .collect()
    } else if args.all {
        let repos = select_repos(&workspace, &[], None, true, false)?;
        filter_changed_repos(repos)?
            .into_iter()
            .map(|repo| repo.path)
            .collect()
    } else {
        vec![workspace.root.clone()]
    };

    if targets.is_empty() {
        output::info("no paths selected for edit");
        return Ok(());
    }

    let mut command = resolve_editor_command(args.editor.as_deref())?;
    for target in targets {
        command.push(target.to_string_lossy().to_string());
    }
    run_command_in_repo(&workspace.root, &command)
}

fn handle_clean(
    args: CleanArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let mut repos = if args.repos.is_empty() {
        select_repos(&workspace, &[], None, true, false)?
    } else {
        select_repos(&workspace, &args.repos, None, false, false)?
    };
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    if repos.is_empty() {
        output::info("no repos selected for clean");
        return Ok(());
    }
    if !args.force {
        output::info("clean is running in dry-run mode; pass --force to delete files");
    }

    let multi = repos.len() > 1;
    for repo in repos {
        if !repo.path.is_dir() {
            output::warn(&format!("repo {} not cloned; skipping", repo.id.as_str()));
            continue;
        }
        if multi {
            println!("== {} ==", repo.id.as_str());
        }
        let mut command = vec!["git".to_string(), "clean".to_string()];
        if args.force {
            command.push("-f".to_string());
        } else {
            command.push("-n".to_string());
        }
        if args.directories {
            command.push("-d".to_string());
        }
        if args.ignored {
            command.push("-x".to_string());
        }
        log_git_command_for_repo(repo.id.as_str(), &command);
        run_command_in_repo(&repo.path, &command)?;
    }

    Ok(())
}

fn handle_config(
    args: ConfigArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let (workspace_root, config_path) = resolve_workspace_paths(workspace_root, config_path)?;
    let command = args.command.unwrap_or(ConfigCommand::Show);

    match command {
        ConfigCommand::Show => handle_config_show(&config_path),
        ConfigCommand::Get(get) => handle_config_get(&config_path, get),
        ConfigCommand::Set(set) => handle_config_set(&config_path, set),
        ConfigCommand::Edit(edit) => handle_config_edit(&workspace_root, &config_path, edit),
    }
}

fn handle_config_show(config_path: &Path) -> Result<()> {
    let contents = fs::read_to_string(config_path)?;
    if contents.is_empty() {
        return Ok(());
    }
    print!("{}", contents);
    Ok(())
}

fn handle_config_get(config_path: &Path, args: ConfigGetArgs) -> Result<()> {
    let value = read_workspace_config_value(config_path)?;
    let found = workspace_config_get(&value, &args.key).ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "config key '{}' not found",
            args.key
        )))
    })?;
    println!("{}", format_config_value(found));
    Ok(())
}

fn handle_config_set(config_path: &Path, args: ConfigSetArgs) -> Result<()> {
    let mut value = read_workspace_config_value(config_path)?;
    let parsed = parse_config_value(&args.value)?;
    workspace_config_set(&mut value, &args.key, parsed)?;
    write_workspace_config_value(config_path, &value)?;
    output::info(&format!("updated {}", args.key));
    Ok(())
}

fn handle_config_edit(
    workspace_root: &Path,
    config_path: &Path,
    args: ConfigEditArgs,
) -> Result<()> {
    let mut command = resolve_editor_command(args.editor.as_deref())?;
    command.push(config_path.to_string_lossy().to_string());
    run_command_in_repo(workspace_root, &command)
}

fn handle_repo(
    args: RepoArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let (_, config_path) = resolve_workspace_paths(workspace_root, config_path)?;
    let command = args.command.unwrap_or(RepoCommand::List);

    match command {
        RepoCommand::List => handle_repo_list(&config_path),
        RepoCommand::Add(add) => handle_repo_add(&config_path, add),
        RepoCommand::Remove(remove) => handle_repo_remove(&config_path, remove),
        RepoCommand::Show(show) => handle_repo_show(&config_path, show),
    }
}

fn handle_repo_list(config_path: &Path) -> Result<()> {
    let value = read_workspace_config_value(config_path)?;
    let repos = workspace_repos_table(&value)?;
    if repos.is_empty() {
        output::info("no repos configured");
        return Ok(());
    }

    println!("Repo          URL                              External  Ignored");
    println!("----------------------------------------------------------------");
    let mut names: Vec<&String> = repos.keys().collect();
    names.sort();
    for name in names {
        let Some(entry) = repos.get(name) else {
            continue;
        };
        let url = entry
            .get("url")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let external = entry
            .get("external")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let ignored = entry
            .get("ignored")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        println!(
            "{:<13} {:<32} {:<8} {}",
            name.as_str(),
            url,
            external,
            ignored
        );
    }

    Ok(())
}

fn handle_repo_show(config_path: &Path, args: RepoShowArgs) -> Result<()> {
    let value = read_workspace_config_value(config_path)?;
    let repos = workspace_repos_table(&value)?;
    let entry = repos.get(&args.name).ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "repo '{}' not found in config",
            args.name
        )))
    })?;

    let url = entry.get("url").and_then(|value| value.as_str());
    let default_branch = entry.get("default_branch").and_then(|value| value.as_str());
    let package_name = entry.get("package_name").and_then(|value| value.as_str());
    let external = entry
        .get("external")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let ignored = entry
        .get("ignored")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    println!("repo: {}", args.name);
    println!("  url: {}", url.unwrap_or("(default)"));
    println!(
        "  default_branch: {}",
        default_branch.unwrap_or("(workspace default)")
    );
    println!("  package_name: {}", package_name.unwrap_or("(repo name)"));
    println!("  external: {}", external);
    println!("  ignored: {}", ignored);
    Ok(())
}

fn handle_repo_add(config_path: &Path, args: RepoAddArgs) -> Result<()> {
    let mut value = read_workspace_config_value(config_path)?;
    let root = value.as_table_mut().ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!("workspace config root must be a table"))
    })?;
    let repos = root
        .entry("repos".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| HarmoniaError::Other(anyhow::anyhow!("[repos] must be a table")))?;

    if repos.contains_key(&args.name) {
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "repo '{}' already exists",
            args.name
        ))));
    }

    let mut entry = toml::map::Map::new();
    if let Some(url) = args.url {
        entry.insert("url".to_string(), toml::Value::String(url));
    }
    if let Some(default_branch) = args.default_branch {
        entry.insert(
            "default_branch".to_string(),
            toml::Value::String(default_branch),
        );
    }
    if let Some(package_name) = args.package_name {
        entry.insert(
            "package_name".to_string(),
            toml::Value::String(package_name),
        );
    }
    if args.external {
        entry.insert("external".to_string(), toml::Value::Boolean(true));
    }
    if args.ignored {
        entry.insert("ignored".to_string(), toml::Value::Boolean(true));
    }

    repos.insert(args.name.clone(), toml::Value::Table(entry));

    if let Some(group) = args.group {
        let groups = root
            .entry("groups".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| HarmoniaError::Other(anyhow::anyhow!("[groups] must be a table")))?;
        let members = groups
            .entry(group.clone())
            .or_insert_with(|| toml::Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!(format!(
                    "[groups].{} must be an array",
                    group
                )))
            })?;
        if !members
            .iter()
            .any(|value| value.as_str() == Some(args.name.as_str()))
        {
            members.push(toml::Value::String(args.name.clone()));
        }
    }

    write_workspace_config_value(config_path, &value)?;
    output::info(&format!("added repo {}", args.name));
    Ok(())
}

fn handle_repo_remove(config_path: &Path, args: RepoRemoveArgs) -> Result<()> {
    let mut value = read_workspace_config_value(config_path)?;
    let root = value.as_table_mut().ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!("workspace config root must be a table"))
    })?;
    let repos = root
        .get_mut("repos")
        .and_then(|value| value.as_table_mut())
        .ok_or_else(|| HarmoniaError::Other(anyhow::anyhow!("[repos] must be a table")))?;

    if repos.remove(&args.name).is_none() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "repo '{}' not found in config",
            args.name
        ))));
    }

    if let Some(groups) = root
        .get_mut("groups")
        .and_then(|value| value.as_table_mut())
    {
        for (_, value) in groups.iter_mut() {
            if let Some(array) = value.as_array_mut() {
                array.retain(|item| item.as_str() != Some(args.name.as_str()));
            }
        }
    }

    write_workspace_config_value(config_path, &value)?;
    output::info(&format!("removed repo {}", args.name));
    Ok(())
}

fn handle_test(
    args: TestArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let default_changed = args.repos.is_empty() && !args.all && !args.changed;
    let changed_scope = args.changed || default_changed;
    let mut repos = select_repos(
        &workspace,
        &args.repos,
        None,
        args.all || changed_scope,
        false,
    )?;
    if changed_scope {
        repos = filter_changed_repos(repos)?;
    }
    if args.graph_order {
        repos = repos_in_graph_order(&workspace, repos)?;
    } else {
        repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    }
    if repos.is_empty() {
        output::info("no repos selected for test");
        return Ok(());
    }

    let mut commands = Vec::new();
    for repo in repos {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        let base = match resolve_quality_command(&workspace, &repo, QualityKind::Test) {
            Some(command) => command,
            None => {
                output::warn(&format!(
                    "no test command configured for {}; skipping",
                    repo.id.as_str()
                ));
                continue;
            }
        };
        let mut command = base;
        if args.coverage {
            let (updated, applied) = apply_test_coverage(&command, &repo);
            command = updated;
            if !applied {
                output::warn(&format!(
                    "coverage requested for {} but command has no automatic coverage flag; running as-is",
                    repo.id.as_str()
                ));
            }
        }
        if let Some(filter) = args.filter.as_deref() {
            command = apply_test_filter(&command, &repo, filter);
        }
        commands.push(QualityCommand { repo, command });
    }

    if commands.is_empty() {
        output::info("no repos selected for test");
        return Ok(());
    }

    if args.graph_order && args.parallel.unwrap_or(1) > 1 {
        output::warn("graph-order test execution is sequential; ignoring --parallel > 1");
    }

    let sequential = args.graph_order || args.fail_fast;
    if sequential {
        for command in commands {
            run_quality_command(QualityKind::Test, command)?;
        }
        return Ok(());
    }

    let jobs = resolve_parallel(args.parallel);
    let results = parallel::run_in_parallel(commands, jobs, |command| {
        run_quality_command(QualityKind::Test, command)
    });
    for result in results {
        result?;
    }

    Ok(())
}

fn handle_lint(
    args: LintArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let default_changed = args.repos.is_empty() && !args.all && !args.changed;
    let changed_scope = args.changed || default_changed;
    let mut repos = select_repos(
        &workspace,
        &args.repos,
        None,
        args.all || changed_scope,
        false,
    )?;
    if changed_scope {
        repos = filter_changed_repos(repos)?;
    }
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    if repos.is_empty() {
        output::info("no repos selected for lint");
        return Ok(());
    }

    let mut commands = Vec::new();
    for repo in repos {
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo.id.as_str()
            ))));
        }
        let base = match resolve_quality_command(&workspace, &repo, QualityKind::Lint) {
            Some(command) => command,
            None => {
                output::warn(&format!(
                    "no lint command configured for {}; skipping",
                    repo.id.as_str()
                ));
                continue;
            }
        };
        let mut command = base;
        if args.fix {
            let (updated, applied) = apply_lint_fix(&command, &repo);
            command = updated;
            if !applied {
                output::warn(&format!(
                    "fix requested for {} but command has no automatic --fix mapping; running as-is",
                    repo.id.as_str()
                ));
            }
        }
        commands.push(QualityCommand { repo, command });
    }

    if commands.is_empty() {
        output::info("no repos selected for lint");
        return Ok(());
    }

    let jobs = resolve_parallel(args.parallel);
    let results = parallel::run_in_parallel(commands, jobs, |command| {
        run_quality_command(QualityKind::Lint, command)
    });
    for result in results {
        result?;
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum QualityKind {
    Test,
    Lint,
}

impl QualityKind {
    fn as_str(self) -> &'static str {
        match self {
            QualityKind::Test => "test",
            QualityKind::Lint => "lint",
        }
    }
}

#[derive(Clone)]
struct QualityCommand {
    repo: Repo,
    command: String,
}

fn run_quality_command(kind: QualityKind, item: QualityCommand) -> Result<()> {
    output::info(&format!(
        "[{}] {}: {}",
        item.repo.id.as_str(),
        kind.as_str(),
        item.command
    ));
    run_shell_command_in_repo(&item.repo.path, &[item.command])
}

fn repos_in_graph_order(workspace: &Workspace, repos: Vec<Repo>) -> Result<Vec<Repo>> {
    let order =
        topological_order(&workspace.graph, &workspace.repos).map_err(HarmoniaError::Other)?;
    let order_index: HashMap<RepoId, usize> = order
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();
    let mut repos = repos;
    repos.sort_by(|a, b| {
        let a_index = order_index.get(&a.id).copied().unwrap_or(usize::MAX);
        let b_index = order_index.get(&b.id).copied().unwrap_or(usize::MAX);
        a_index
            .cmp(&b_index)
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });
    Ok(repos)
}

fn expand_branch_scope(
    workspace: &Workspace,
    repos: Vec<Repo>,
    include_dependents: bool,
    include_all_deps: bool,
) -> Vec<Repo> {
    let mut selected: HashSet<RepoId> = repos.iter().map(|repo| repo.id.clone()).collect();
    if include_dependents || include_all_deps {
        for repo in &repos {
            for dependent in transitive_dependents(&workspace.graph, &workspace.repos, &repo.id) {
                selected.insert(dependent);
            }
        }
    }
    if include_all_deps {
        for repo in &repos {
            for dependency in transitive_dependencies(&workspace.graph, &workspace.repos, &repo.id)
            {
                selected.insert(dependency);
            }
        }
    }

    let mut expanded: Vec<Repo> = selected
        .into_iter()
        .filter_map(|id| workspace.repos.get(&id).cloned())
        .filter(|repo| should_include_repo(repo, false))
        .collect();
    expanded.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    expanded
}

fn resolve_quality_command(
    workspace: &Workspace,
    repo: &Repo,
    kind: QualityKind,
) -> Option<String> {
    let key = kind.as_str();
    let repo_override = repo
        .config
        .as_ref()
        .and_then(|config| config.hooks.as_ref())
        .and_then(|hooks| hooks.custom.as_ref())
        .and_then(|custom| custom.get(key))
        .cloned();
    if repo_override.is_some() {
        return repo_override;
    }

    let workspace_override = workspace
        .config
        .hooks
        .as_ref()
        .and_then(|hooks| hooks.custom.as_ref())
        .and_then(|custom| custom.get(key))
        .cloned();
    if workspace_override.is_some() {
        return workspace_override;
    }

    let ecosystem = repo.ecosystem.as_ref()?;
    let plugin = plugin_for(ecosystem);
    match kind {
        QualityKind::Test => plugin.default_test_command().map(|cmd| cmd.to_string()),
        QualityKind::Lint => plugin.default_lint_command().map(|cmd| cmd.to_string()),
    }
}

fn apply_test_filter(command: &str, repo: &Repo, filter: &str) -> String {
    let quoted = shell_single_quote(filter);
    match repo.ecosystem.as_ref() {
        Some(EcosystemId::Python) => format!("{command} -k {quoted}"),
        Some(EcosystemId::Go) => format!("{command} -run {quoted}"),
        Some(EcosystemId::Node) => format!("{command} -- {quoted}"),
        _ => format!("{command} {quoted}"),
    }
}

fn apply_test_coverage(command: &str, repo: &Repo) -> (String, bool) {
    match repo.ecosystem.as_ref() {
        Some(EcosystemId::Python) if !command.contains("--cov") => {
            (format!("{command} --cov"), true)
        }
        Some(EcosystemId::Go) if !command.contains("-cover") => (format!("{command} -cover"), true),
        _ => (command.to_string(), false),
    }
}

fn apply_lint_fix(command: &str, repo: &Repo) -> (String, bool) {
    if command.contains("--fix") {
        return (command.to_string(), true);
    }

    match repo.ecosystem.as_ref() {
        Some(EcosystemId::Rust) if command.trim_start().starts_with("cargo clippy") => (
            format!("{command} --fix --allow-dirty --allow-staged"),
            true,
        ),
        Some(EcosystemId::Python) if command.trim_start().starts_with("ruff check") => {
            (format!("{command} --fix"), true)
        }
        Some(EcosystemId::Go) if command.contains("golangci-lint") => {
            (format!("{command} --fix"), true)
        }
        Some(EcosystemId::Node) => (format!("{command} -- --fix"), true),
        _ => (command.to_string(), false),
    }
}

fn shell_single_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

fn handle_plan(
    args: PlanArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let plan = build_plan_summary(&workspace, &args.include, &args.exclude)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan_to_json(&plan))
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
    } else {
        print_plan_summary(&plan);
    }
    Ok(())
}

fn handle_mr(
    args: MrArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let command = args.command.unwrap_or(MrCommand::Status(MrStatusArgs {
        json: false,
        wait: false,
        timeout: 30,
    }));

    match command {
        MrCommand::Create(args) => handle_mr_create(args, &workspace),
        MrCommand::Status(args) => handle_mr_status(args, &workspace),
        MrCommand::Update(args) => handle_mr_update(args, &workspace),
        MrCommand::Merge(args) => handle_mr_merge(args, &workspace),
        MrCommand::Close(args) => handle_mr_close(args, &workspace),
    }
}

fn handle_mr_create(args: MrCreateArgs, workspace: &Workspace) -> Result<()> {
    let mut plan = build_plan_summary(workspace, &[], &[])?;
    if plan.changed.is_empty() {
        output::info("no changed repositories detected; nothing to create");
        return Ok(());
    }

    let draft = args.draft
        || workspace
            .config
            .mr
            .as_ref()
            .and_then(|config| config.draft)
            .unwrap_or(false);
    let labels = merged_labels(workspace, &args.labels);
    let mut ordered = ordered_plan_repos(&plan);
    let link_behavior = effective_link_behavior(workspace, &args)?;
    let create_tracking_issue =
        should_create_tracking_issue(workspace, &args, ordered.len(), link_behavior);
    let title_override = args.title.clone();
    let shared_description = args.description.clone().or_else(|| {
        plan.changeset
            .as_ref()
            .map(|changeset| changeset.description.clone())
    });

    if args.dry_run {
        println!("MR Create Plan");
        println!("==============");
        println!("draft: {}", draft);
        println!("link related mrs: {}", link_behavior.related);
        println!("link in description: {}", link_behavior.description);
        println!("create tracking issue: {}", create_tracking_issue);
        println!("require tests: {}", mr_require_tests_enabled(workspace));
        if !labels.is_empty() {
            println!("labels: {}", labels.join(", "));
        }
        if !args.reviewers.is_empty() {
            println!("reviewers: {}", args.reviewers.join(", "));
        }
        println!("merge order:");
        for (index, repo_id) in ordered.iter().enumerate() {
            println!("  {}. {}", index + 1, repo_id.as_str());
        }
        return Ok(());
    }

    prepare_mr_create_branches(&args, workspace, &mut plan, &mut ordered)?;
    ensure_mr_branches_are_mergeable(workspace, &plan, &ordered)?;

    if mr_require_tests_enabled(workspace) {
        run_required_mr_tests(workspace, &ordered)?;
    }

    let forge = workspace_forge_client(workspace)?;
    let mut created = Vec::new();
    let mut state = load_mr_state(workspace)?;
    let base_title = title_override
        .or_else(|| {
            plan.changeset
                .as_ref()
                .map(|changeset| changeset.title.clone())
        })
        .unwrap_or_else(|| {
            let branch = plan
                .changed
                .first()
                .map(|repo| repo.branch.clone())
                .unwrap_or_else(|| "changeset".to_string());
            format!("changeset: {branch}")
        });

    for repo_id in ordered.clone() {
        let plan_repo = plan
            .changed
            .iter()
            .find(|repo| repo.id == repo_id)
            .ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!(format!(
                    "missing plan entry for {}",
                    repo_id.as_str()
                )))
            })?;
        let repo = workspace.repos.get(&repo_id).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {}",
                repo_id.as_str()
            )))
        })?;
        let forge_repo = forge_repo_for_repo(workspace, repo);
        let description = build_mr_description(
            workspace,
            &plan,
            repo,
            shared_description.as_deref().unwrap_or_default(),
        )?;
        let mr = forge.create_mr(
            &forge_repo,
            CreateMrParams {
                title: base_title.clone(),
                description,
                source_branch: plan_repo.branch.clone(),
                target_branch: repo.default_branch.clone(),
                draft,
                labels: labels.clone(),
                reviewers: args.reviewers.clone(),
            },
        )?;

        output::info(&format!(
            "created MR for {}: !{} {}",
            repo.id.as_str(),
            mr.iid,
            mr.url
        ));
        let entry = StoredMrEntry {
            repo: repo.id.as_str().to_string(),
            forge_repo: forge_repo.as_str().to_string(),
            branch: plan_repo.branch.clone(),
            mr_id: mr.iid.to_string(),
            iid: mr.iid,
            url: mr.url.clone(),
            source_branch: mr.source_branch.clone(),
            target_branch: mr.target_branch.clone(),
        };
        upsert_mr_state_entry(&mut state, entry.clone());
        created.push(entry);
    }

    if link_behavior.related && created.len() > 1 {
        let links: Vec<(RepoId, String)> = created
            .iter()
            .map(|entry| (RepoId::new(entry.forge_repo.clone()), entry.mr_id.clone()))
            .collect();
        forge.link_mrs(&links)?;
        output::info("linked merge requests in merge order");
    }

    if link_behavior.description && created.len() > 1 {
        for entry in &created {
            let repo = RepoId::new(entry.forge_repo.clone());
            let current = forge.get_mr(&repo, &entry.mr_id)?;
            let updated_description = with_related_mr_links(
                &current.description,
                &created,
                entry.repo.as_str(),
                plan.changeset
                    .as_ref()
                    .map(|changeset| changeset.id.as_str()),
            );
            forge.update_mr(
                &repo,
                &entry.mr_id,
                UpdateMrParams {
                    title: None,
                    description: Some(updated_description),
                    labels: None,
                    reviewers: None,
                },
            )?;
        }
        output::info("updated MR descriptions with related links");
    }

    if create_tracking_issue {
        if let Some(first) = created.first() {
            let issue_title = format!("Tracking: {}", base_title);
            let issue_description = build_tracking_issue_description(
                workspace,
                &plan,
                &created,
                shared_description.as_deref(),
            )?;
            let issue = forge.create_issue(CreateIssueParams {
                project: Some(RepoId::new(first.forge_repo.clone())),
                title: issue_title,
                description: issue_description,
                labels: labels.clone(),
            })?;
            output::info(&format!(
                "created tracking issue #{} {}",
                issue.iid, issue.url
            ));
        }
    }

    if mr_add_trailers_enabled(workspace) {
        let changeset_id = plan
            .changeset
            .as_ref()
            .map(|changeset| changeset.id.as_str())
            .unwrap_or("local-changeset");
        output::warn(&format!(
            "mr.add_trailers is enabled; add commit trailers manually (e.g. Changeset-ID: {}) before merge",
            changeset_id
        ));
    }

    save_mr_state(workspace, &state)?;
    run_post_mr_create_hook(workspace)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MrBranchConflict {
    repo: RepoId,
    source_branch: String,
    target_branch: String,
}

fn prepare_mr_create_branches(
    args: &MrCreateArgs,
    workspace: &Workspace,
    plan: &mut PlanSummary,
    ordered: &mut Vec<RepoId>,
) -> Result<()> {
    let conflicts = collect_mr_branch_conflicts(workspace, plan, ordered)?;
    if conflicts.is_empty() {
        return Ok(());
    }

    let branch_name = resolve_mr_auto_branch_name(args, plan)?;
    let mut should_auto_branch = args.auto_branch || args.branch_name.is_some();
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !should_auto_branch && interactive {
        let prompt = format!(
            "{} repositories are on their default branch for MR creation. create and checkout '{}' in those repositories now?",
            conflicts.len(),
            branch_name
        );
        should_auto_branch = output::confirm(&prompt, false)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    }

    if !should_auto_branch {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            format_mr_branch_conflict_error(&conflicts, Some(branch_name.as_str()))
        )));
    }

    auto_branch_conflicted_mr_repos(workspace, &conflicts, branch_name.as_str())?;
    output::info(&format!(
        "auto-branch switched {} repositories to {}",
        conflicts.len(),
        branch_name
    ));

    *plan = build_plan_summary(workspace, &[], &[])?;
    *ordered = ordered_plan_repos(plan);
    Ok(())
}

fn resolve_mr_auto_branch_name(args: &MrCreateArgs, plan: &PlanSummary) -> Result<String> {
    if let Some(branch_name) = args.branch_name.as_ref() {
        let trimmed = branch_name.trim();
        if trimmed.is_empty() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(
                "--branch-name cannot be empty"
            )));
        }
        return Ok(trimmed.to_string());
    }

    if let Some(changeset) = plan.changeset.as_ref() {
        let trimmed = changeset.branch.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    Ok(generated_mr_auto_branch_name())
}

fn generated_mr_auto_branch_name() -> String {
    let timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => 0,
    };
    format!("feature/harmonia-{timestamp}")
}

fn auto_branch_conflicted_mr_repos(
    workspace: &Workspace,
    conflicts: &[MrBranchConflict],
    branch_name: &str,
) -> Result<()> {
    let mut existing = Vec::new();
    for conflict in conflicts {
        let repo = workspace.repos.get(&conflict.repo).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {}",
                conflict.repo.as_str()
            )))
        })?;
        let open = open_repo(&repo.path)?;
        if branch_exists(&open.repo, branch_name)? {
            existing.push(repo.id.clone());
        }
    }

    if !existing.is_empty() {
        let names = existing
            .iter()
            .map(|repo| repo.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "cannot auto-create branch '{}' because it already exists in: {}. choose a different --branch-name or switch branches manually",
            branch_name, names
        ))));
    }

    for conflict in conflicts {
        let repo = workspace.repos.get(&conflict.repo).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {}",
                conflict.repo.as_str()
            )))
        })?;
        let open = open_repo(&repo.path)?;
        output::git_op(&format!(
            "checkout -b {} (repo {})",
            branch_name,
            repo.id.as_str()
        ));
        create_and_checkout_branch(&open.repo, branch_name).map_err(|err| {
            HarmoniaError::Other(anyhow::anyhow!(format!("{}: {}", repo.id.as_str(), err)))
        })?;
        output::info(&format!(
            "created and checked out '{}' in {}",
            branch_name,
            repo.id.as_str()
        ));
    }

    Ok(())
}

fn ensure_mr_branches_are_mergeable(
    workspace: &Workspace,
    plan: &PlanSummary,
    ordered: &[RepoId],
) -> Result<()> {
    let conflicts = collect_mr_branch_conflicts(workspace, plan, ordered)?;
    if conflicts.is_empty() {
        return Ok(());
    }

    Err(HarmoniaError::Other(anyhow::anyhow!(
        format_mr_branch_conflict_error(&conflicts, None)
    )))
}

fn collect_mr_branch_conflicts(
    workspace: &Workspace,
    plan: &PlanSummary,
    ordered: &[RepoId],
) -> Result<Vec<MrBranchConflict>> {
    let mut conflicts = Vec::new();
    for repo_id in ordered {
        let Some(plan_repo) = plan.changed.iter().find(|repo| &repo.id == repo_id) else {
            continue;
        };
        let repo = workspace.repos.get(repo_id).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {}",
                repo_id.as_str()
            )))
        })?;

        if plan_repo.branch == repo.default_branch {
            conflicts.push(MrBranchConflict {
                repo: repo.id.clone(),
                source_branch: plan_repo.branch.clone(),
                target_branch: repo.default_branch.clone(),
            });
        }
    }

    Ok(conflicts)
}

fn format_mr_branch_conflict_error(
    conflicts: &[MrBranchConflict],
    suggested_auto_branch: Option<&str>,
) -> String {
    let mut message =
        "cannot create merge requests where source and target branches are the same".to_string();
    for conflict in conflicts {
        message.push_str(&format!(
            "\n  - {} (source: {}, target: {})",
            conflict.repo.as_str(),
            conflict.source_branch,
            conflict.target_branch
        ));
    }
    message.push_str("\ncreate or switch to a feature branch before running `harmonia mr create`");
    message.push_str("\nhelper: re-run with `harmonia mr create --auto-branch` to create feature branches automatically for affected repos");
    if let Some(branch) = suggested_auto_branch {
        message.push_str(&format!(
            "\ndefault auto-branch name for this run: {}",
            branch
        ));
    }
    message.push_str("\nexample: harmonia branch <feature-name> --create --changed");
    message
}

fn handle_mr_status(args: MrStatusArgs, workspace: &Workspace) -> Result<()> {
    let store = load_mr_state(workspace)?;
    let tracked = tracked_mrs_for_current_branches(workspace, &store)?;
    if tracked.is_empty() {
        let plan = build_plan_summary(workspace, &[], &[])?;
        if args.json {
            let payload = serde_json::json!({
                "tracked_mrs": [],
                "changed_repos": plan.changed.iter().map(|repo| repo.id.as_str()).collect::<Vec<_>>(),
                "merge_order": plan.merge_order.iter().map(|repo| repo.as_str()).collect::<Vec<_>>(),
                "wait": args.wait,
                "timeout_minutes": args.timeout,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&payload)
                    .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
            );
        } else {
            println!("MR Preview");
            println!("==========");
            if plan.changed.is_empty() {
                println!("No changed repositories detected.");
            } else {
                println!("Repositories with local changes:");
                for repo in &plan.changed {
                    println!("  {}", repo.id.as_str());
                }
                println!();
                println!("Suggested merge order:");
                for (index, repo) in plan.merge_order.iter().enumerate() {
                    println!("  {}. {}", index + 1, repo.as_str());
                }
            }
            output::info("no tracked MRs found for current branches");
        }
        return Ok(());
    }

    let forge = match workspace_forge_client(workspace) {
        Ok(forge) => forge,
        Err(err) => {
            if args.wait {
                return Err(err);
            }
            if args.json {
                let payload = serde_json::json!({
                    "tracked_mrs": tracked.iter().map(|item| {
                        serde_json::json!({
                            "repo": item.repo.id.as_str(),
                            "branch": item.entry.branch.as_str(),
                            "mr_iid": item.entry.iid,
                            "url": item.entry.url.as_str(),
                            "state": "unknown",
                            "ci_state": serde_json::Value::Null,
                        })
                    }).collect::<Vec<_>>(),
                    "wait": false,
                    "timeout_minutes": args.timeout,
                    "note": "forge config missing; remote status unavailable",
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload)
                        .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
                );
            } else {
                output::warn("forge config missing; showing tracked MR metadata only");
                for item in tracked {
                    println!(
                        "{}: !{} ({})",
                        item.repo.id.as_str(),
                        item.entry.iid,
                        item.entry.url
                    );
                }
            }
            return Ok(());
        }
    };

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(args.timeout.saturating_mul(60)))
        .unwrap_or_else(Instant::now);
    let mut timed_out = false;
    let rows = loop {
        let rows = collect_mr_status_rows(forge.as_ref(), &tracked)?;
        let waiting = rows.iter().any(|row| {
            matches!(
                row.ci_state,
                Some(CiState::Pending) | Some(CiState::Running)
            ) || !row.missing_required_checks.is_empty()
        });
        let has_failed_required = rows
            .iter()
            .any(|row| !row.failed_required_checks.is_empty());
        if has_failed_required {
            break rows;
        }
        if !args.wait || !waiting {
            break rows;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break rows;
        }
        std::thread::sleep(Duration::from_secs(5));
    };

    if args.json {
        let payload = serde_json::json!({
            "tracked_mrs": rows.iter().map(mr_status_row_to_json).collect::<Vec<_>>(),
            "wait": args.wait,
            "timeout_minutes": args.timeout,
            "timed_out": timed_out,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload)
                .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?
        );
    } else {
        println!("Merge Request Status");
        println!("====================");
        for row in &rows {
            let ci_state = row
                .ci_state
                .as_ref()
                .map(ci_state_label)
                .unwrap_or("unknown");
            println!(
                "{}: !{} {} (state: {}, ci: {}, approvals: {})",
                row.repo.as_str(),
                row.iid,
                row.url,
                mr_state_label(&row.state),
                ci_state,
                row.approvals.join(", ")
            );
            if !row.missing_required_checks.is_empty() {
                println!(
                    "  waiting required checks: {}",
                    row.missing_required_checks.join(", ")
                );
            }
            if !row.failed_required_checks.is_empty() {
                println!(
                    "  failed required checks: {}",
                    row.failed_required_checks.join(", ")
                );
            }
        }
        if timed_out {
            output::warn("timed out while waiting for CI to settle");
        }
    }

    if args.wait
        && rows
            .iter()
            .any(|row| !row.failed_required_checks.is_empty())
    {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "one or more required checks failed"
        )));
    }

    if timed_out {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "timeout waiting for CI to complete"
        )));
    }

    Ok(())
}

fn handle_mr_update(args: MrUpdateArgs, workspace: &Workspace) -> Result<()> {
    if args.description.is_none() && args.labels.is_empty() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "mr update requires --description and/or --labels"
        )));
    }

    let store = load_mr_state(workspace)?;
    let tracked = tracked_mrs_for_current_branches(workspace, &store)?;
    if tracked.is_empty() {
        output::info("no tracked MRs found for current branches");
        return Ok(());
    }
    let forge = workspace_forge_client(workspace)?;

    for item in tracked {
        let params = UpdateMrParams {
            title: None,
            description: args.description.clone(),
            labels: if args.labels.is_empty() {
                None
            } else {
                Some(args.labels.clone())
            },
            reviewers: None,
        };
        let updated = forge.update_mr(&item.forge_repo, &item.entry.mr_id, params)?;
        output::info(&format!(
            "updated MR for {}: !{}",
            item.repo.id.as_str(),
            updated.iid
        ));
    }

    Ok(())
}

fn handle_mr_merge(args: MrMergeArgs, workspace: &Workspace) -> Result<()> {
    let store = load_mr_state(workspace)?;
    let tracked = tracked_mrs_for_current_branches(workspace, &store)?;
    if tracked.is_empty() {
        output::info("no tracked MRs found for current branches");
        return Ok(());
    }
    let forge = workspace_forge_client(workspace)?;

    let ordered = tracked_mrs_in_merge_order(workspace, tracked)?;
    if args.dry_run {
        println!("MR Merge Plan");
        println!("=============");
        for (index, item) in ordered.iter().enumerate() {
            println!(
                "  {}. {} (!{})",
                index + 1,
                item.repo.id.as_str(),
                item.entry.iid
            );
        }
        return Ok(());
    }

    if !output::confirm("merge tracked MRs in dependency order?", args.yes)
        .map_err(|err| HarmoniaError::Other(anyhow::anyhow!(err.to_string())))?
    {
        output::info("merge cancelled");
        return Ok(());
    }

    for item in ordered {
        let mr = forge.get_mr(&item.forge_repo, &item.entry.mr_id)?;
        if mr.state == MrState::Merged {
            output::info(&format!(
                "MR for {} is already merged; skipping",
                item.repo.id.as_str()
            ));
            continue;
        }
        if mr.state == MrState::Closed {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "MR for {} is closed and cannot be merged",
                item.repo.id.as_str()
            ))));
        }

        if !args.no_wait {
            wait_for_ci_success(forge.as_ref(), &item)?;
        }

        forge.merge_mr(
            &item.forge_repo,
            &item.entry.mr_id,
            MergeMrParams {
                squash: args.squash,
                delete_source_branch: args.delete_branch,
            },
        )?;
        output::info(&format!(
            "merged MR for {}: !{}",
            item.repo.id.as_str(),
            item.entry.iid
        ));
    }

    Ok(())
}

fn handle_mr_close(args: MrCloseArgs, workspace: &Workspace) -> Result<()> {
    let mut store = load_mr_state(workspace)?;
    let tracked = tracked_mrs_for_current_branches(workspace, &store)?;
    if tracked.is_empty() {
        output::info("no tracked MRs found for current branches");
        return Ok(());
    }
    let forge = workspace_forge_client(workspace)?;

    if !output::confirm("close tracked MRs for current branches?", args.yes)
        .map_err(|err| HarmoniaError::Other(anyhow::anyhow!(err.to_string())))?
    {
        output::info("close cancelled");
        return Ok(());
    }

    for item in &tracked {
        forge.close_mr(&item.forge_repo, &item.entry.mr_id)?;
        output::info(&format!(
            "closed MR for {}: !{}",
            item.repo.id.as_str(),
            item.entry.iid
        ));
    }

    for item in tracked {
        store
            .entries
            .retain(|entry| !(entry.repo == item.entry.repo && entry.branch == item.entry.branch));
    }
    save_mr_state(workspace, &store)?;

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMrEntry {
    repo: String,
    forge_repo: String,
    branch: String,
    mr_id: String,
    iid: u64,
    url: String,
    source_branch: String,
    target_branch: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MrStateStore {
    #[serde(default)]
    entries: Vec<StoredMrEntry>,
}

#[derive(Debug, Clone)]
struct TrackedMr {
    repo: Repo,
    forge_repo: RepoId,
    entry: StoredMrEntry,
}

#[derive(Debug, Clone)]
struct MrStatusRow {
    repo: RepoId,
    iid: u64,
    url: String,
    state: MrState,
    ci_state: Option<CiState>,
    approvals: Vec<String>,
    checks: Vec<(String, String)>,
    missing_required_checks: Vec<String>,
    failed_required_checks: Vec<String>,
}

fn run_post_mr_create_hook(workspace: &Workspace) -> Result<()> {
    let Some(command) = workspace
        .config
        .hooks
        .as_ref()
        .and_then(|hooks| hooks.post_mr_create.as_deref())
    else {
        return Ok(());
    };
    run_command_in_repo(&workspace.root, &split_command(command))
}

fn load_mr_state(workspace: &Workspace) -> Result<MrStateStore> {
    let path = mr_state_path(workspace);
    if !path.exists() {
        return Ok(MrStateStore::default());
    }
    let raw = fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(MrStateStore::default());
    }
    serde_json::from_str::<MrStateStore>(&raw).map_err(|err| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "failed to parse {}: {}",
            path.display(),
            err
        )))
    })
}

fn save_mr_state(workspace: &Workspace, state: &MrStateStore) -> Result<()> {
    let path = mr_state_path(workspace);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(state)
        .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    fs::write(path, contents)?;
    Ok(())
}

fn mr_state_path(workspace: &Workspace) -> PathBuf {
    workspace.root.join(".harmonia").join("mr-state.json")
}

fn upsert_mr_state_entry(state: &mut MrStateStore, entry: StoredMrEntry) {
    state
        .entries
        .retain(|existing| !(existing.repo == entry.repo && existing.branch == entry.branch));
    state.entries.push(entry);
}

fn tracked_mrs_for_current_branches(
    workspace: &Workspace,
    state: &MrStateStore,
) -> Result<Vec<TrackedMr>> {
    let mut repos: Vec<&Repo> = workspace
        .repos
        .values()
        .filter(|repo| !repo.ignored && !repo.external && repo.path.is_dir())
        .collect();
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    let mut by_repo_branch: HashMap<(String, String), StoredMrEntry> = HashMap::new();
    for entry in &state.entries {
        by_repo_branch.insert((entry.repo.clone(), entry.branch.clone()), entry.clone());
    }

    let mut tracked = Vec::new();
    for repo in repos {
        let open = open_repo(&repo.path)?;
        let branch = current_branch(&open.repo)?;
        let key = (repo.id.as_str().to_string(), branch);
        let Some(entry) = by_repo_branch.get(&key).cloned() else {
            continue;
        };
        tracked.push(TrackedMr {
            repo: repo.clone(),
            forge_repo: RepoId::new(entry.forge_repo.clone()),
            entry,
        });
    }
    Ok(tracked)
}

fn tracked_mrs_in_merge_order(
    workspace: &Workspace,
    tracked: Vec<TrackedMr>,
) -> Result<Vec<TrackedMr>> {
    if tracked.len() <= 1 {
        return Ok(tracked);
    }
    let targets: Vec<RepoId> = tracked.iter().map(|item| item.repo.id.clone()).collect();
    let order =
        merge_order(&workspace.graph, &workspace.repos, &targets).map_err(HarmoniaError::Other)?;
    let mut by_repo: HashMap<RepoId, TrackedMr> = tracked
        .into_iter()
        .map(|item| (item.repo.id.clone(), item))
        .collect();

    let mut ordered = Vec::new();
    for repo_id in order {
        if let Some(item) = by_repo.remove(&repo_id) {
            ordered.push(item);
        }
    }

    let mut remaining: Vec<TrackedMr> = by_repo.into_values().collect();
    remaining.sort_by(|a, b| a.repo.id.as_str().cmp(b.repo.id.as_str()));
    ordered.extend(remaining);
    Ok(ordered)
}

fn workspace_forge_client(workspace: &Workspace) -> Result<Box<dyn crate::forge::traits::Forge>> {
    let config = workspace.config.forge.as_ref().ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(
            "forge config is required (set [forge] in .harmonia/config.toml or .harmonia.toml)"
        ))
    })?;
    client_from_forge_config(config)
}

fn forge_repo_for_repo(workspace: &Workspace, repo: &Repo) -> RepoId {
    if let Some(path) = project_path_from_remote_url(&repo.remote_url) {
        return RepoId::new(path);
    }
    if let Some(group) = workspace
        .config
        .forge
        .as_ref()
        .and_then(|forge| forge.default_group.as_deref())
    {
        let group = group.trim().trim_matches('/');
        if !group.is_empty() {
            return RepoId::new(format!("{}/{}", group, repo.id.as_str()));
        }
    }
    repo.id.clone()
}

fn project_path_from_remote_url(remote_url: &str) -> Option<String> {
    let trimmed = remote_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git");
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("git@") {
        let (_, path) = rest.split_once(':')?;
        let path = path.trim_start_matches('/').trim();
        if path.is_empty() {
            return None;
        }
        return Some(path.to_string());
    }

    if let Some(rest) = trimmed.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map(|(_, value)| value).unwrap_or(rest);
        let (_, path) = rest.split_once('/')?;
        let path = path.trim_start_matches('/').trim();
        if path.is_empty() {
            return None;
        }
        return Some(path.to_string());
    }

    if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
    {
        let (_, path) = rest.split_once('/')?;
        let path = path.trim_start_matches('/').trim();
        if path.is_empty() {
            return None;
        }
        return Some(path.to_string());
    }

    if let Some((_, path)) = trimmed.split_once(':') {
        if path.contains('/') {
            let path = path.trim_start_matches('/').trim();
            if path.is_empty() {
                return None;
            }
            return Some(path.to_string());
        }
    }

    None
}

fn ordered_plan_repos(plan: &PlanSummary) -> Vec<RepoId> {
    let changed_ids: HashSet<RepoId> = plan.changed.iter().map(|repo| repo.id.clone()).collect();
    let mut ordered: Vec<RepoId> = plan
        .merge_order
        .iter()
        .filter(|repo| changed_ids.contains(*repo))
        .cloned()
        .collect();
    let seen: HashSet<RepoId> = ordered.iter().cloned().collect();
    let mut remaining: Vec<RepoId> = plan
        .changed
        .iter()
        .map(|repo| repo.id.clone())
        .filter(|repo| !seen.contains(repo))
        .collect();
    remaining.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    ordered.extend(remaining);
    ordered
}

fn merged_labels(workspace: &Workspace, cli_labels: &[String]) -> Vec<String> {
    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    for label in workspace
        .config
        .mr
        .as_ref()
        .and_then(|config| config.labels.as_ref())
        .into_iter()
        .flatten()
        .chain(cli_labels.iter())
    {
        if seen.insert(label.clone()) {
            labels.push(label.clone());
        }
    }
    labels
}

#[derive(Debug, Clone, Copy)]
struct LinkBehavior {
    related: bool,
    description: bool,
    issue: bool,
}

fn effective_link_behavior(workspace: &Workspace, args: &MrCreateArgs) -> Result<LinkBehavior> {
    let configured = workspace
        .config
        .mr
        .as_ref()
        .and_then(|config| config.link_strategy.as_deref())
        .unwrap_or("all")
        .to_ascii_lowercase();

    let mut behavior = match configured.as_str() {
        "related" => LinkBehavior {
            related: true,
            description: false,
            issue: false,
        },
        "description" => LinkBehavior {
            related: false,
            description: true,
            issue: false,
        },
        "issue" => LinkBehavior {
            related: false,
            description: false,
            issue: true,
        },
        "all" => LinkBehavior {
            related: true,
            description: true,
            issue: true,
        },
        other => {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "invalid mr.link_strategy '{}': expected related, description, issue, or all",
                other
            ))))
        }
    };

    if args.no_link {
        behavior.related = false;
        behavior.description = false;
    }
    if args.no_issue {
        behavior.issue = false;
    }
    Ok(behavior)
}

fn should_create_tracking_issue(
    workspace: &Workspace,
    args: &MrCreateArgs,
    mr_count: usize,
    behavior: LinkBehavior,
) -> bool {
    if !behavior.issue || args.no_issue {
        return false;
    }
    workspace
        .config
        .mr
        .as_ref()
        .and_then(|config| config.create_tracking_issue)
        .unwrap_or(mr_count > 1)
}

fn mr_require_tests_enabled(workspace: &Workspace) -> bool {
    workspace
        .config
        .mr
        .as_ref()
        .and_then(|config| config.require_tests)
        .unwrap_or(false)
}

fn mr_add_trailers_enabled(workspace: &Workspace) -> bool {
    workspace
        .config
        .mr
        .as_ref()
        .and_then(|config| config.add_trailers)
        .unwrap_or(false)
}

fn run_required_mr_tests(workspace: &Workspace, repos: &[RepoId]) -> Result<()> {
    if repos.is_empty() {
        return Ok(());
    }
    output::info("mr.require_tests=true, running tests for selected repos");
    for repo_id in repos {
        let repo = workspace.repos.get(repo_id).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {} in MR test scope",
                repo_id.as_str()
            )))
        })?;
        let command =
            resolve_quality_command(workspace, repo, QualityKind::Test).ok_or_else(|| {
                HarmoniaError::Other(anyhow::anyhow!(format!(
                    "mr.require_tests=true but no test command is configured for {}",
                    repo.id.as_str()
                )))
            })?;
        run_quality_command(
            QualityKind::Test,
            QualityCommand {
                repo: repo.clone(),
                command,
            },
        )?;
    }
    Ok(())
}

fn build_mr_description(
    workspace: &Workspace,
    plan: &PlanSummary,
    repo: &Repo,
    description_text: &str,
) -> Result<String> {
    let description = description_text.trim().to_string();
    let mrs = changeset_template_rows(workspace, plan, None);
    let context = serde_json::json!({
        "repo": repo.id.as_str(),
        "description": description,
        "title": plan.changeset.as_ref().map(|changeset| changeset.title.as_str()).unwrap_or(""),
        "changeset": {
            "id": plan.changeset.as_ref().map(|changeset| changeset.id.as_str()).unwrap_or(""),
            "branch": plan.changeset.as_ref().map(|changeset| changeset.branch.as_str()).unwrap_or(""),
            "repos": plan.changed.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            "merge_order": plan.merge_order.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            "mrs": mrs,
            "repo_summary": plan
                .changed
                .iter()
                .find(|item| item.id == repo.id)
                .and_then(|item| item.changeset_summary.as_deref())
                .unwrap_or(""),
        },
    });

    if let Some(path) = workspace
        .config
        .mr
        .as_ref()
        .and_then(|config| config.template.as_deref())
        .map(|path| resolve_template_path(workspace, path))
    {
        return render_template_file(&path, &context);
    }

    let mut body = String::new();
    if !description.is_empty() {
        body.push_str(&description);
        body.push_str("\n\n");
    }
    body.push_str("---\n\n");
    body.push_str("## Coordinated Changeset\n\n");
    body.push_str("Merge order:\n");
    for (index, repo_id) in plan.merge_order.iter().enumerate() {
        body.push_str(&format!("{}. {}\n", index + 1, repo_id.as_str()));
    }
    if let Some(summary) = plan
        .changed
        .iter()
        .find(|item| item.id == repo.id)
        .and_then(|item| item.changeset_summary.as_deref())
    {
        body.push_str("\nRepo summary:\n");
        body.push_str(summary);
        body.push('\n');
    }
    Ok(body)
}

fn build_tracking_issue_description(
    workspace: &Workspace,
    plan: &PlanSummary,
    created: &[StoredMrEntry],
    cli_description: Option<&str>,
) -> Result<String> {
    let description = cli_description.unwrap_or("").trim().to_string();
    let title = plan
        .changeset
        .as_ref()
        .map(|changeset| changeset.title.clone())
        .unwrap_or_default();
    let mrs = changeset_template_rows(workspace, plan, Some(created));
    let context = serde_json::json!({
        "title": title,
        "description": description,
        "now": format!("{:?}", std::time::SystemTime::now()),
        "changeset": {
            "id": plan.changeset.as_ref().map(|changeset| changeset.id.as_str()).unwrap_or(""),
            "branch": plan
                .changeset
                .as_ref()
                .map(|changeset| changeset.branch.as_str())
                .unwrap_or(""),
            "mrs": mrs,
        },
    });

    if let Some(path) = workspace
        .config
        .mr
        .as_ref()
        .and_then(|config| config.issue_template.as_deref())
        .map(|path| resolve_template_path(workspace, path))
    {
        return render_template_file(&path, &context);
    }

    let mut body = String::new();
    if !description.is_empty() {
        body.push_str(&description);
        body.push_str("\n\n");
    }
    body.push_str("This issue tracks coordinated merge requests:\n");
    for entry in created {
        body.push_str(&format!(
            "- {}: !{} ({})\n",
            entry.repo, entry.iid, entry.url
        ));
    }
    Ok(body)
}

fn with_related_mr_links(
    description: &str,
    created: &[StoredMrEntry],
    current_repo: &str,
    changeset_id: Option<&str>,
) -> String {
    let start_marker = "<!-- harmonia:related:start -->";
    let end_marker = "<!-- harmonia:related:end -->";
    let base = if let Some(start) = description.find(start_marker) {
        let suffix = &description[start..];
        if suffix.contains(end_marker) {
            let mut truncated = String::from(&description[..start]);
            truncated = truncated.trim_end().to_string();
            truncated
        } else {
            description.trim_end().to_string()
        }
    } else {
        description.trim_end().to_string()
    };

    let mut out = base;
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(start_marker);
    out.push('\n');
    out.push_str("## Related Merge Requests\n");
    if let Some(changeset_id) = changeset_id {
        out.push_str(&format!("Changeset: `{}`\n", changeset_id));
    }
    out.push('\n');
    for entry in created {
        let current = if entry.repo == current_repo {
            " (this MR)"
        } else {
            ""
        };
        out.push_str(&format!(
            "- {}: !{} ({}){}\n",
            entry.repo, entry.iid, entry.url, current
        ));
    }
    out.push_str(end_marker);
    out.push('\n');
    out
}

fn changeset_template_rows(
    workspace: &Workspace,
    plan: &PlanSummary,
    created: Option<&[StoredMrEntry]>,
) -> Vec<serde_json::Value> {
    let package_lookup = package_map(&workspace.repos);
    let selected: HashSet<RepoId> = plan.changed.iter().map(|item| item.id.clone()).collect();
    let merge_order_map: HashMap<RepoId, usize> = plan
        .merge_order
        .iter()
        .enumerate()
        .map(|(index, repo)| (repo.clone(), index + 1))
        .collect();

    let created_lookup: HashMap<String, &StoredMrEntry> = created
        .unwrap_or_default()
        .iter()
        .map(|entry| (entry.repo.clone(), entry))
        .collect();

    let mut rows = Vec::new();
    for item in &plan.changed {
        let dependencies = internal_dependencies_for(&workspace.graph, &item.id)
            .into_iter()
            .filter_map(|dep| package_lookup.get(&dep.name).cloned())
            .filter(|dep| selected.contains(dep))
            .map(|dep| dep.as_str().to_string())
            .collect::<Vec<_>>();
        let mut dependencies = dependencies;
        dependencies.sort();
        dependencies.dedup();

        let mut dependents = direct_dependents(&workspace.graph, &workspace.repos, &item.id)
            .into_iter()
            .filter(|dep| selected.contains(dep))
            .map(|dep| dep.as_str().to_string())
            .collect::<Vec<_>>();
        dependents.sort();
        dependents.dedup();

        let (link, status, status_emoji) = if let Some(entry) = created_lookup.get(item.id.as_str())
        {
            (
                format!("[!{}]({})", entry.iid, entry.url),
                "open".to_string(),
                "🟢".to_string(),
            )
        } else {
            (
                "pending".to_string(),
                "pending".to_string(),
                "🟡".to_string(),
            )
        };

        rows.push(serde_json::json!({
            "repo": item.id.as_str(),
            "link": link,
            "status": status,
            "status_emoji": status_emoji,
            "merge_order": merge_order_map.get(&item.id).copied().unwrap_or(0),
            "dependencies": dependencies,
            "dependents": dependents,
            "summary": item.changeset_summary.as_deref().unwrap_or(""),
        }));
    }

    rows.sort_by(|a, b| {
        let a_order = a
            .get("merge_order")
            .and_then(|value| value.as_u64())
            .unwrap_or(u64::MAX);
        let b_order = b
            .get("merge_order")
            .and_then(|value| value.as_u64())
            .unwrap_or(u64::MAX);
        a_order.cmp(&b_order)
    });
    rows
}

fn resolve_template_path(workspace: &Workspace, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        workspace.root.join(candidate)
    }
}

fn collect_mr_status_rows(
    forge: &dyn crate::forge::traits::Forge,
    tracked: &[TrackedMr],
) -> Result<Vec<MrStatusRow>> {
    let mut rows = Vec::new();
    for item in tracked {
        let mr = forge.get_mr(&item.forge_repo, &item.entry.mr_id)?;
        let ci = forge.get_ci_status(&item.forge_repo, &item.entry.source_branch)?;
        let ci_state = ci.state.clone();
        let checks = ci
            .checks
            .iter()
            .map(|check| (check.name.clone(), check.status.clone()))
            .collect::<Vec<_>>();
        let required = required_checks_for_repo(&item.repo);
        let required_result = evaluate_required_checks(&required, &ci.checks);
        let (missing_required_checks, failed_required_checks) = match required_result {
            RequiredChecksState::Pending(names) => (names, Vec::new()),
            RequiredChecksState::Failed(names) => (Vec::new(), names),
            RequiredChecksState::Satisfied => (Vec::new(), Vec::new()),
        };
        rows.push(MrStatusRow {
            repo: item.repo.id.clone(),
            iid: mr.iid,
            url: mr.url,
            state: mr.state,
            ci_state: Some(ci_state),
            approvals: mr
                .approvals
                .iter()
                .map(|user| user.username.clone())
                .collect(),
            checks,
            missing_required_checks,
            failed_required_checks,
        });
    }
    rows.sort_by(|a, b| a.repo.as_str().cmp(b.repo.as_str()));
    Ok(rows)
}

fn wait_for_ci_success(forge: &dyn crate::forge::traits::Forge, item: &TrackedMr) -> Result<()> {
    let timeout_minutes = item
        .repo
        .config
        .as_ref()
        .and_then(|config| config.ci.as_ref())
        .and_then(|ci| ci.timeout_minutes)
        .unwrap_or(30);
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(timeout_minutes.saturating_mul(60)))
        .unwrap_or_else(Instant::now);
    let required_checks = required_checks_for_repo(&item.repo);

    loop {
        let status = forge.get_ci_status(&item.forge_repo, &item.entry.source_branch)?;
        let required_result = evaluate_required_checks(&required_checks, &status.checks);
        match status.state {
            CiState::Success | CiState::Skipped => match required_result {
                RequiredChecksState::Satisfied => return Ok(()),
                RequiredChecksState::Pending(names) => {
                    if Instant::now() >= deadline {
                        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                            "timeout waiting for required checks for {}: {}",
                            item.repo.id.as_str(),
                            names.join(", ")
                        ))));
                    }
                    std::thread::sleep(Duration::from_secs(5));
                }
                RequiredChecksState::Failed(names) => {
                    return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                        "required checks failed for {}: {}",
                        item.repo.id.as_str(),
                        names.join(", ")
                    ))))
                }
            },
            CiState::Failed | CiState::Canceled => {
                return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                    "CI is not passing for {} (state: {})",
                    item.repo.id.as_str(),
                    ci_state_label(&status.state)
                ))))
            }
            CiState::Pending | CiState::Running => {
                if let RequiredChecksState::Failed(names) = required_result {
                    return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                        "required checks failed for {}: {}",
                        item.repo.id.as_str(),
                        names.join(", ")
                    ))));
                }
                if Instant::now() >= deadline {
                    return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                        "timeout waiting for CI for {}",
                        item.repo.id.as_str()
                    ))));
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn required_checks_for_repo(repo: &Repo) -> Vec<String> {
    let mut checks = repo
        .config
        .as_ref()
        .and_then(|config| config.ci.as_ref())
        .and_then(|ci| ci.required_checks.clone())
        .unwrap_or_default();
    checks.sort();
    checks.dedup();
    checks
}

#[derive(Debug)]
enum RequiredChecksState {
    Satisfied,
    Pending(Vec<String>),
    Failed(Vec<String>),
}

fn evaluate_required_checks(
    required: &[String],
    checks: &[crate::forge::CheckRun],
) -> RequiredChecksState {
    if required.is_empty() {
        return RequiredChecksState::Satisfied;
    }

    let mut pending = Vec::new();
    let mut failed = Vec::new();
    for required_name in required {
        let matched = checks
            .iter()
            .find(|check| check.name == *required_name)
            .map(|check| check.status.as_str());
        match matched {
            Some("success") | Some("passed") | Some("skipped") => {}
            Some(
                "pending"
                | "running"
                | "created"
                | "preparing"
                | "manual"
                | "scheduled"
                | "waiting_for_resource",
            ) => pending.push(required_name.clone()),
            Some(_) => failed.push(required_name.clone()),
            None => pending.push(required_name.clone()),
        }
    }

    if !failed.is_empty() {
        return RequiredChecksState::Failed(failed);
    }
    if !pending.is_empty() {
        return RequiredChecksState::Pending(pending);
    }
    RequiredChecksState::Satisfied
}

fn mr_status_row_to_json(row: &MrStatusRow) -> serde_json::Value {
    serde_json::json!({
        "repo": row.repo.as_str(),
        "mr_iid": row.iid,
        "url": row.url,
        "state": mr_state_label(&row.state),
        "ci_state": row.ci_state.as_ref().map(ci_state_label),
        "approvals": row.approvals,
        "checks": row.checks.iter().map(|(name, status)| {
            serde_json::json!({
                "name": name,
                "status": status,
            })
        }).collect::<Vec<_>>(),
        "missing_required_checks": row.missing_required_checks,
        "failed_required_checks": row.failed_required_checks,
    })
}

fn mr_state_label(state: &MrState) -> &'static str {
    match state {
        MrState::Open => "open",
        MrState::Merged => "merged",
        MrState::Closed => "closed",
        MrState::Draft => "draft",
    }
}

fn ci_state_label(state: &CiState) -> &'static str {
    match state {
        CiState::Pending => "pending",
        CiState::Running => "running",
        CiState::Success => "success",
        CiState::Failed => "failed",
        CiState::Canceled => "canceled",
        CiState::Skipped => "skipped",
    }
}

fn handle_shell(
    args: ShellArgs,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let workspace = load_workspace(workspace_root, config_path)?;
    let mut repos: Vec<Repo> = if args.repos.is_empty() {
        workspace
            .repos
            .values()
            .filter(|repo| !repo.ignored && repo.path.is_dir())
            .cloned()
            .collect()
    } else {
        select_repos(&workspace, &args.repos, None, false, true)?
            .into_iter()
            .filter(|repo| !repo.ignored && repo.path.is_dir())
            .collect()
    };
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    let mut path_prefixes = Vec::new();
    let mut pythonpath_prefixes = Vec::new();
    for repo in &repos {
        let bin_dir = repo.path.join("bin");
        if bin_dir.is_dir() {
            path_prefixes.push(bin_dir);
        }
        let src_dir = repo.path.join("src");
        if src_dir.is_dir() {
            pythonpath_prefixes.push(src_dir);
        }
    }

    let path_value = compose_shell_env_value("PATH", path_prefixes)?;
    let pythonpath_value = compose_shell_env_value("PYTHONPATH", pythonpath_prefixes)?;

    if let Some(command) = args.command.as_deref() {
        let split = split_command(command);
        if split.is_empty() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(
                "shell command cannot be empty"
            )));
        }
        return run_shell_command_with_env(
            &workspace.root,
            &split,
            &workspace.root,
            path_value.as_deref(),
            pythonpath_value.as_deref(),
        );
    }

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        let workspace_value = workspace.root.to_string_lossy().to_string();
        println!(
            "export HARMONIA_WORKSPACE={}",
            shell_single_quote(&workspace_value)
        );
        if let Some(path) = path_value.as_ref() {
            println!("export PATH={}", shell_single_quote(path));
        }
        if let Some(pythonpath) = pythonpath_value.as_ref() {
            println!("export PYTHONPATH={}", shell_single_quote(pythonpath));
        }
        return Ok(());
    }

    let shell = env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(windows) {
            "cmd".to_string()
        } else {
            "sh".to_string()
        }
    });

    let mut cmd = std::process::Command::new(&shell);
    cmd.current_dir(&workspace.root);
    cmd.env("HARMONIA_WORKSPACE", &workspace.root);
    if let Some(path) = path_value {
        cmd.env("PATH", path);
    }
    if let Some(pythonpath) = pythonpath_value {
        cmd.env("PYTHONPATH", pythonpath);
    }
    let status = cmd
        .status()
        .with_context(|| format!("failed to launch shell '{}'", shell))?;
    if status.success() {
        return Ok(());
    }

    Err(HarmoniaError::Other(anyhow::anyhow!(format!(
        "shell '{}' exited unsuccessfully",
        shell
    ))))
}

fn handle_completion(args: CompletionArgs) -> Result<()> {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    clap_complete::generate(args.shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}

#[derive(Debug)]
struct PlanSummary {
    changed: Vec<PlanChangedRepo>,
    merge_order: Vec<RepoId>,
    constraints: ConstraintReport,
    recommendations: Vec<String>,
    changeset: Option<PlanChangeset>,
}

#[derive(Debug)]
struct PlanChangedRepo {
    id: RepoId,
    branch: String,
    status: StatusSummary,
    diff_stat: String,
    changeset_summary: Option<String>,
}

#[derive(Debug, Clone)]
struct PlanChangeset {
    id: String,
    title: String,
    description: String,
    branch: String,
    repo_summaries: HashMap<RepoId, String>,
}

fn build_plan_summary(
    workspace: &Workspace,
    include: &[String],
    exclude: &[String],
) -> Result<PlanSummary> {
    let mut include_ids = resolve_plan_repo_ids(workspace, include, "include")?;
    let exclude_ids = resolve_plan_repo_ids(workspace, exclude, "exclude")?;

    for repo_id in &include_ids {
        let repo = workspace.repos.get(repo_id).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo {}",
                repo_id.as_str()
            )))
        })?;
        if !repo.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "repo {} not cloned",
                repo_id.as_str()
            ))));
        }
    }

    let mut repos: Vec<&Repo> = workspace
        .repos
        .values()
        .filter(|repo| !repo.ignored && !repo.external && repo.path.is_dir())
        .collect();
    repos.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    let mut candidates = Vec::new();
    let mut all_branches = HashSet::new();
    let mut changed_branches = HashSet::new();
    for repo in repos {
        if exclude_ids.contains(&repo.id) {
            continue;
        }
        let open = open_repo(&repo.path)?;
        let branch = current_branch(&open.repo)?;
        let status = repo_status(&open.repo)?;
        all_branches.insert(branch.clone());
        if !status.is_clean() {
            changed_branches.insert(branch.clone());
        }
        let diff_stat = git_shortstat_output(&repo.path);
        candidates.push((repo.clone(), branch, status, diff_stat));
    }

    let branch_scope = if changed_branches.is_empty() {
        &all_branches
    } else {
        &changed_branches
    };
    let changeset = load_active_changeset(workspace, branch_scope)?;
    if let Some(changeset) = changeset.as_ref() {
        for repo_id in changeset.repo_summaries.keys() {
            include_ids.insert(repo_id.clone());
        }
    }

    let mut changed = Vec::new();
    for (repo, branch, status, diff_stat) in candidates {
        if status.is_clean() && !include_ids.contains(&repo.id) {
            continue;
        }
        changed.push(PlanChangedRepo {
            id: repo.id.clone(),
            branch,
            status,
            diff_stat,
            changeset_summary: changeset
                .as_ref()
                .and_then(|changeset| changeset.repo_summaries.get(&repo.id))
                .filter(|summary| !summary.trim().is_empty())
                .cloned(),
        });
    }

    let merge_order = if changed.is_empty() {
        Vec::new()
    } else {
        let targets: Vec<RepoId> = changed.iter().map(|repo| repo.id.clone()).collect();
        merge_order(&workspace.graph, &workspace.repos, &targets).map_err(HarmoniaError::Other)?
    };
    let versions = collect_versions(workspace)?;
    let constraints = check_constraints(&workspace.graph, &workspace.repos, &versions);
    let recommendations = plan_recommendations(&changed, &constraints);

    Ok(PlanSummary {
        changed,
        merge_order,
        constraints,
        recommendations,
        changeset,
    })
}

fn resolve_plan_repo_ids(
    workspace: &Workspace,
    repos: &[String],
    flag: &str,
) -> Result<HashSet<RepoId>> {
    let mut ids = HashSet::new();
    for repo_name in repos {
        let repo_id = RepoId::new(repo_name.clone());
        if !workspace.repos.contains_key(&repo_id) {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "unknown repo '{}' passed to --{}",
                repo_name, flag
            ))));
        }
        ids.insert(repo_id);
    }
    Ok(ids)
}

fn load_active_changeset(
    workspace: &Workspace,
    branches: &HashSet<String>,
) -> Result<Option<PlanChangeset>> {
    let files = load_changeset_files(&workspace.root, &workspace.config)?;
    let selected = select_active_changeset(&files, branches)?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    normalize_changeset(workspace, selected).map(Some)
}

fn normalize_changeset(workspace: &Workspace, file: ChangesetFile) -> Result<PlanChangeset> {
    let mut repo_summaries = HashMap::new();
    for repo in &file.repos {
        let repo_id = RepoId::new(repo.repo.clone());
        let known = workspace.repos.get(&repo_id).ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "changeset '{}' references unknown repo '{}'",
                file.id, repo.repo
            )))
        })?;
        if known.ignored {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "changeset '{}' references ignored repo '{}'",
                file.id, repo.repo
            ))));
        }
        if known.external {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "changeset '{}' references external repo '{}'",
                file.id, repo.repo
            ))));
        }
        if !known.path.is_dir() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "changeset '{}' references repo '{}' which is not cloned",
                file.id, repo.repo
            ))));
        }
        repo_summaries.insert(repo_id, repo.summary.clone());
    }

    Ok(PlanChangeset {
        id: file.id,
        title: file.title,
        description: file.description,
        branch: file.branch,
        repo_summaries,
    })
}

fn git_shortstat_output(repo_path: &Path) -> String {
    let command = vec![
        "git".to_string(),
        "diff".to_string(),
        "--shortstat".to_string(),
    ];
    let output = match run_command_output_in_repo(repo_path, &command) {
        Ok(output) => output,
        Err(_) => return "diff stat unavailable".to_string(),
    };

    let trimmed = output.trim();
    if trimmed.is_empty() {
        "no textual diff stats".to_string()
    } else {
        trimmed.to_string()
    }
}

fn print_plan_summary(plan: &PlanSummary) {
    println!("Changeset Analysis");
    println!("==================");

    if let Some(changeset) = plan.changeset.as_ref() {
        println!();
        println!("Active changeset: {} ({})", changeset.id, changeset.branch);
        if !changeset.title.trim().is_empty() {
            println!("  title: {}", changeset.title);
        }
    }

    if plan.changed.is_empty() {
        println!();
        println!("No changed repositories detected.");
        return;
    }

    println!();
    println!("Changed repositories:");
    for repo in &plan.changed {
        println!(
            "  {:<18} {:<16} {}",
            repo.id.as_str(),
            repo.branch,
            plan_status_summary(&repo.status)
        );
        println!("    {}", repo.diff_stat);
        if let Some(summary) = repo.changeset_summary.as_deref() {
            println!("    changeset: {}", summary);
        }
    }

    println!();
    println!("Merge order:");
    for (index, repo) in plan.merge_order.iter().enumerate() {
        println!("  {}. {}", index + 1, repo.as_str());
    }

    println!();
    println!("Constraint analysis:");
    println!("  cycles: {}", plan.constraints.cycles.len());
    println!("  missing dependencies: {}", plan.constraints.missing.len());
    println!("  violations: {}", plan.constraints.violations.len());

    if !plan.constraints.violations.is_empty() {
        println!("  top violations:");
        for violation in plan.constraints.violations.iter().take(5) {
            println!(
                "    - {} -> {} [{}]",
                violation.from_repo.as_str(),
                violation.to_repo.as_str(),
                violation.violation_type.as_str()
            );
        }
    }

    println!();
    println!("Recommendations:");
    for recommendation in &plan.recommendations {
        println!("  - {}", recommendation);
    }
}

fn plan_to_json(plan: &PlanSummary) -> serde_json::Value {
    serde_json::json!({
        "changed_repos": plan.changed.iter().map(|repo| {
            serde_json::json!({
                "repo": repo.id.as_str(),
                "branch": repo.branch,
                "staged": repo.status.staged.len(),
                "modified": repo.status.modified.len(),
                "untracked": repo.status.untracked.len(),
                "conflicts": repo.status.conflicts.len(),
                "diff_stat": repo.diff_stat,
                "changeset_summary": repo.changeset_summary.as_deref(),
            })
        }).collect::<Vec<_>>(),
        "changeset": plan.changeset.as_ref().map(|changeset| {
            serde_json::json!({
                "id": changeset.id,
                "title": changeset.title,
                "description": changeset.description,
                "branch": changeset.branch,
                "repos": changeset.repo_summaries.iter().map(|(repo, summary)| {
                    serde_json::json!({
                        "repo": repo.as_str(),
                        "summary": summary,
                    })
                }).collect::<Vec<_>>(),
            })
        }),
        "merge_order": plan.merge_order.iter().map(|repo| repo.as_str()).collect::<Vec<_>>(),
        "constraint_analysis": {
            "cycles": plan.constraints.cycles.iter().map(|cycle| cycle.iter().map(|repo| repo.as_str()).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "missing": plan.constraints.missing.iter().map(|missing| {
                serde_json::json!({
                    "from": missing.from.as_str(),
                    "dependency": missing.dependency.name,
                    "constraint": missing.dependency.constraint.raw,
                })
            }).collect::<Vec<_>>(),
            "violations": plan.constraints.violations.iter().map(|violation| {
                serde_json::json!({
                    "from": violation.from_repo.as_str(),
                    "to": violation.to_repo.as_str(),
                    "constraint": violation.constraint.raw,
                    "actual_version": violation.actual_version.raw,
                    "violation_type": violation.violation_type.as_str(),
                })
            }).collect::<Vec<_>>(),
        },
        "recommendations": plan.recommendations,
    })
}

fn plan_recommendations(changed: &[PlanChangedRepo], report: &ConstraintReport) -> Vec<String> {
    let mut recommendations = Vec::new();

    if !report.cycles.is_empty() {
        recommendations
            .push("resolve dependency cycles before creating merge requests".to_string());
    }
    if !report.missing.is_empty() {
        recommendations
            .push("add missing internal dependency mappings in repository configs".to_string());
    }
    if report
        .violations
        .iter()
        .any(|violation| matches!(violation.violation_type, ViolationType::Unsatisfied))
    {
        recommendations.push(
            "run `harmonia deps update --dry-run` to preview required constraint updates"
                .to_string(),
        );
    }
    if report.violations.iter().any(|violation| {
        matches!(
            violation.violation_type,
            ViolationType::ExactPin | ViolationType::UpperBound
        )
    }) {
        recommendations.push(
            "review strict dependency constraints (exact pins and upper bounds) before merges"
                .to_string(),
        );
    }
    if changed.len() > 1 {
        recommendations.push("merge in the listed order and wait for CI between steps".to_string());
    }
    if recommendations.is_empty() {
        recommendations
            .push("no blocking issues detected for the current local changeset".to_string());
    }

    recommendations
}

fn plan_status_summary(status: &StatusSummary) -> String {
    format!(
        "{} staged, {} modified, {} untracked, {} conflicts",
        status.staged.len(),
        status.modified.len(),
        status.untracked.len(),
        status.conflicts.len()
    )
}

fn compose_shell_env_value(name: &str, mut prefixes: Vec<PathBuf>) -> Result<Option<String>> {
    if let Some(existing) = env::var_os(name) {
        prefixes.extend(env::split_paths(&existing));
    }
    if prefixes.is_empty() {
        return Ok(None);
    }

    let joined = env::join_paths(prefixes).map_err(|err| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "failed to compute {} for shell command: {}",
            name, err
        )))
    })?;
    Ok(Some(joined.to_string_lossy().to_string()))
}

fn resolve_workspace_paths(
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<(PathBuf, PathBuf)> {
    let cwd = env::current_dir()?;
    let resolved = resolve_workspace_with_overrides(cwd, workspace_root, config_path)?;
    Ok((resolved.root, resolved.config_path))
}

fn resolve_editor_command(editor: Option<&str>) -> Result<Vec<String>> {
    let command = editor
        .map(|value| value.to_string())
        .or_else(|| env::var("EDITOR").ok())
        .unwrap_or_else(|| "code".to_string());
    let command = split_command(&command);
    if command.is_empty() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "editor command cannot be empty"
        )));
    }
    Ok(command)
}

fn read_workspace_config_value(path: &Path) -> Result<toml::Value> {
    let contents = fs::read_to_string(path)?;
    toml::from_str(&contents).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
}

fn write_workspace_config_value(path: &Path, value: &toml::Value) -> Result<()> {
    let contents = toml::to_string_pretty(value)
        .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    fs::write(path, contents)?;
    Ok(())
}

fn workspace_config_get<'a>(value: &'a toml::Value, key: &str) -> Option<&'a toml::Value> {
    let segments: Vec<&str> = key
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }

    let mut current = value;
    for segment in segments {
        current = current.get(segment)?;
    }
    Some(current)
}

fn workspace_config_set(value: &mut toml::Value, key: &str, new_value: toml::Value) -> Result<()> {
    let segments: Vec<&str> = key
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "config key cannot be empty"
        )));
    }

    let mut current = value;
    for segment in &segments[..segments.len() - 1] {
        let table = current.as_table_mut().ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "cannot set '{}' because '{}' is not a table",
                key, segment
            )))
        })?;
        current = table
            .entry((*segment).to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if !current.is_table() {
            return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                "cannot set '{}' because '{}' is not a table",
                key, segment
            ))));
        }
    }

    let leaf = segments[segments.len() - 1];
    let table = current.as_table_mut().ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(format!(
            "cannot set '{}' because parent path is not a table",
            key
        )))
    })?;
    table.insert(leaf.to_string(), new_value);
    Ok(())
}

fn parse_config_value(raw: &str) -> Result<toml::Value> {
    let snippet = format!("value = {raw}");
    if let Ok(parsed) = toml::from_str::<toml::Value>(&snippet) {
        if let Some(value) = parsed.get("value") {
            return Ok(value.clone());
        }
    }

    Ok(toml::Value::String(raw.to_string()))
}

fn format_config_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(value) => value.clone(),
        toml::Value::Integer(value) => value.to_string(),
        toml::Value::Float(value) => value.to_string(),
        toml::Value::Boolean(value) => value.to_string(),
        toml::Value::Datetime(value) => value.to_string(),
        _ => value.to_string(),
    }
}

fn workspace_repos_table(value: &toml::Value) -> Result<&toml::map::Map<String, toml::Value>> {
    let repos = value
        .get("repos")
        .and_then(|value| value.as_table())
        .ok_or_else(|| HarmoniaError::Other(anyhow::anyhow!("[repos] must be a table")))?;
    Ok(repos)
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

fn run_shell_command_with_env(
    repo_path: &Path,
    command: &[String],
    workspace_root: &Path,
    path_value: Option<&str>,
    pythonpath_value: Option<&str>,
) -> Result<()> {
    if command.is_empty() {
        return Err(HarmoniaError::Other(anyhow::anyhow!("missing command")));
    }
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
    cmd.current_dir(repo_path);
    cmd.env("HARMONIA_WORKSPACE", workspace_root);
    if let Some(path) = path_value {
        cmd.env("PATH", path);
    }
    if let Some(pythonpath) = pythonpath_value {
        cmd.env("PYTHONPATH", pythonpath);
    }

    let status = cmd
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

fn ahead_behind_for_repo(repo_path: &Path) -> (usize, usize) {
    let command = vec![
        "git".to_string(),
        "rev-list".to_string(),
        "--left-right".to_string(),
        "--count".to_string(),
        "@{upstream}...HEAD".to_string(),
    ];
    match run_command_output_in_repo(repo_path, &command) {
        Ok(output) => parse_ahead_behind_counts(&output).unwrap_or((0, 0)),
        Err(_) => (0, 0),
    }
}

fn parse_ahead_behind_counts(output: &str) -> Option<(usize, usize)> {
    let mut parts = output.split_whitespace();
    let behind: usize = parts.next()?.parse().ok()?;
    let ahead: usize = parts.next()?.parse().ok()?;
    Some((ahead, behind))
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

fn log_git_command_for_repo(repo_name: &str, command: &[String]) {
    let rendered = if command.len() > 1 {
        command[1..].join(" ")
    } else if let Some(command_name) = command.first() {
        command_name.clone()
    } else {
        String::new()
    };
    output::git_op(&format!("{} (repo {})", rendered, repo_name));
}

fn git_diff_files(
    repo_path: &Path,
    repo_name: &str,
    staged: bool,
    include_untracked: bool,
) -> Result<Vec<String>> {
    let mut cmd = vec![
        "git".to_string(),
        "diff".to_string(),
        "--name-only".to_string(),
    ];
    if staged {
        cmd.push("--staged".to_string());
    }
    log_git_command_for_repo(repo_name, &cmd);
    let output = run_command_output_in_repo(repo_path, &cmd)?;
    let mut files: Vec<String> = output
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect();

    if include_untracked {
        let untracked_cmd = vec![
            "git".to_string(),
            "ls-files".to_string(),
            "--others".to_string(),
            "--exclude-standard".to_string(),
        ];
        log_git_command_for_repo(repo_name, &untracked_cmd);
        let untracked = run_command_output_in_repo(repo_path, &untracked_cmd)?;
        files.extend(
            untracked
                .lines()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .map(|line| line.to_string()),
        );
    }

    files.sort();
    files.dedup();
    Ok(files)
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
    path: PathBuf,
    branch: String,
    ahead: usize,
    behind: usize,
    status: StatusSummary,
}

fn print_status_table(workspace: &Workspace, rows: &[StatusRow], short: bool) -> Result<()> {
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
    if short {
        for row in rows {
            println!(
                "{}\t{}\t+{}\t-{}\t{}",
                row.repo,
                row.branch,
                row.ahead,
                row.behind,
                status_summary(row)
            );
        }
        return Ok(());
    }

    println!("Workspace: {} ({} repos)", workspace_name, rows.len());
    println!();
    let repo_width = rows
        .iter()
        .map(|row| row.repo.len())
        .chain(std::iter::once("Repo".len()))
        .max()
        .unwrap_or("Repo".len());
    let branch_width = rows
        .iter()
        .map(|row| row.branch.len())
        .chain(std::iter::once("Branch".len()))
        .max()
        .unwrap_or("Branch".len());
    println!(
        "{:<repo_width$} {:<branch_width$} {:>4} {:>4} {:<5}",
        "Repo",
        "Branch",
        "↑",
        "↓",
        "Status",
        repo_width = repo_width,
        branch_width = branch_width
    );
    let header_len = format!(
        "{:<repo_width$} {:<branch_width$} {:>4} {:>4} {:<5}",
        "Repo",
        "Branch",
        "↑",
        "↓",
        "Status",
        repo_width = repo_width,
        branch_width = branch_width
    )
    .len();
    println!("{}", "-".repeat(header_len));
    for row in rows {
        println!(
            "{:<repo_width$} {:<branch_width$} {:>4} {:>4} {}",
            row.repo,
            row.branch,
            row.ahead,
            row.behind,
            status_summary(row),
            repo_width = repo_width,
            branch_width = branch_width
        );
    }
    Ok(())
}

fn status_summary(row: &StatusRow) -> String {
    if row.status.is_clean() {
        "clean".to_string()
    } else {
        format!(
            "{} staged, {} modified, {} untracked, {} conflicts",
            row.status.staged.len(),
            row.status.modified.len(),
            row.status.untracked.len(),
            row.status.conflicts.len()
        )
    }
}

fn print_status_long(rows: &[StatusRow], include_untracked: bool) -> Result<()> {
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("== {} ==", row.repo);
        println!(
            "branch: {} | ahead: {} | behind: {}",
            row.branch, row.ahead, row.behind
        );
        let mut command = vec!["git".to_string(), "status".to_string()];
        if !include_untracked {
            command.push("--untracked-files=no".to_string());
        }
        run_command_in_repo(&row.path, &command)?;
    }
    Ok(())
}

fn print_status_porcelain(rows: &[StatusRow]) {
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.repo,
            row.branch,
            row.ahead,
            row.behind,
            row.status.staged.len(),
            row.status.modified.len(),
            row.status.untracked.len(),
            row.status.conflicts.len()
        );
    }
}

fn print_status_json(rows: &[StatusRow]) -> Result<()> {
    let json = serde_json::to_string_pretty(
        &rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "repo": row.repo,
                    "branch": row.branch,
                    "ahead": row.ahead,
                    "behind": row.behind,
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

fn include_untracked_by_default(workspace: &Workspace) -> bool {
    workspace
        .config
        .defaults
        .as_ref()
        .and_then(|defaults| defaults.include_untracked)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::{
        format_mr_branch_conflict_error, parse_ahead_behind_counts, parse_depth, resolve_clone_url,
        to_https_url, to_ssh_url, MrBranchConflict,
    };
    use crate::core::repo::RepoId;

    #[test]
    fn parse_ahead_behind_output() {
        assert_eq!(parse_ahead_behind_counts("4\t9\n"), Some((9, 4)));
        assert_eq!(parse_ahead_behind_counts("0 0"), Some((0, 0)));
        assert_eq!(parse_ahead_behind_counts("x y"), None);
    }

    #[test]
    fn clone_url_protocol_conversion() {
        assert_eq!(
            to_https_url("git@gitlab.example.com:team/repo.git"),
            Some("https://gitlab.example.com/team/repo.git".to_string())
        );
        assert_eq!(
            to_ssh_url("https://gitlab.example.com/team/repo.git"),
            Some("git@gitlab.example.com:team/repo.git".to_string())
        );
        assert_eq!(
            resolve_clone_url("file:///tmp/repo.git", Some("https")),
            "file:///tmp/repo.git".to_string()
        );
    }

    #[test]
    fn parse_depth_from_defaults() {
        assert_eq!(
            parse_depth(None, false, Some("5")).expect("depth parse"),
            Some(5)
        );
        assert_eq!(
            parse_depth(None, false, Some("full")).expect("depth parse"),
            None
        );
        assert_eq!(
            parse_depth(None, true, Some("5")).expect("depth parse"),
            None
        );
    }

    #[test]
    fn mr_branch_conflict_error_is_actionable() {
        let message = format_mr_branch_conflict_error(
            &[
                MrBranchConflict {
                    repo: RepoId::new("python/api".to_string()),
                    source_branch: "main".to_string(),
                    target_branch: "main".to_string(),
                },
                MrBranchConflict {
                    repo: RepoId::new("python/utils".to_string()),
                    source_branch: "main".to_string(),
                    target_branch: "main".to_string(),
                },
            ],
            Some("feature/harmonia-test"),
        );

        assert!(
            message.contains(
                "cannot create merge requests where source and target branches are the same"
            ),
            "message:\n{message}"
        );
        assert!(message.contains("python/api"), "message:\n{message}");
        assert!(message.contains("python/utils"), "message:\n{message}");
        assert!(
            message.contains("harmonia mr create --auto-branch"),
            "message:\n{message}"
        );
        assert!(
            message.contains("default auto-branch name for this run: feature/harmonia-test"),
            "message:\n{message}"
        );
        assert!(
            message.contains("example: harmonia branch <feature-name> --create --changed"),
            "message:\n{message}"
        );
    }
}
