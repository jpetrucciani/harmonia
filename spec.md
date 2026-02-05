# Harmonia

**A poly-repo orchestrator for coordinated multi-repository development**

*Bringing disparate repositories into harmony*

---

## Table of Contents

1. [Overview](#1-overview)
2. [Core Concepts](#2-core-concepts)
3. [Configuration](#3-configuration)
4. [CLI Reference](#4-cli-reference)
5. [Dependency Graph Engine](#5-dependency-graph-engine)
6. [Forge Integration](#6-forge-integration)
7. [Ecosystem Plugins](#7-ecosystem-plugins)
8. [Data Structures](#8-data-structures)
9. [Architecture](#9-architecture)
10. [Implementation Phases](#10-implementation-phases)

---

## 1. Overview

### 1.1 Problem Statement

Organizations with modular codebases often split code across multiple repositories for good reasons: independent versioning, separate CI, team ownership boundaries. However, this creates friction when changes span multiple repositories:

- Coordinating branches across repos is manual and error-prone
- Testing changes together requires ad-hoc environment setup
- Merge order matters due to dependencies, but is often implicit knowledge
- Linking related MRs/PRs is tedious and inconsistent
- Version bumps and dependency updates are manual

### 1.2 Mission

Harmonia is a **language-agnostic, forge-agnostic** tool that orchestrates development across multiple repositories while preserving their independence. It provides:

- **Workspace management**: Clone, sync, and manage multiple repos as a unit
- **Coordinated operations**: Branch, commit, push across repos simultaneously
- **Dependency awareness**: Understand inter-repo dependencies, compute merge order
- **Forge integration**: Create linked MRs/PRs with proper sequencing information
- **Ecosystem integration**: Parse version and dependency info from various languages

### 1.3 Design Principles

1. **Convention over configuration** - Sensible defaults, override when needed
2. **Language agnostic** - Python, Rust, Go, Node, or anything else
3. **Forge agnostic** - GitLab, GitHub, Gitea, Forgejo, etc.
4. **Composable** - Small commands that chain together, Unix philosophy
5. **Transparent** - Always show what git operations are being run
6. **Non-destructive** - Never force-push, overwrite, or delete without explicit confirmation (or `--yes`)
7. **Offline-first** - Core operations work without network; forge features degrade gracefully

### 1.4 Non-Goals

- **Monorepo conversion**: Harmonia manages poly-repos, not monorepos
- **Build system**: Use your existing build tools; Harmonia orchestrates, not builds
- **CI replacement**: Harmonia helps create MRs; CI systems validate them
- **Package publishing**: Harmonia bumps versions; your CI publishes artifacts

---

## 2. Core Concepts

### 2.1 Workspace

A **workspace** is a directory containing multiple cloned repositories and Harmonia configuration. It is typically a git repository itself (a "meta-repo") that tracks configuration but ignores the cloned repos.

```
my-workspace/
├── .git/                    # Workspace is itself a repo
├── .harmonia/
│   ├── config.toml          # Workspace configuration
│   ├── templates/           # MR templates, etc.
│   └── changesets/          # Pending changesets (optional)
├── .gitignore               # Ignores repos/
└── repos/                   # Cloned repositories
    ├── service-a/
    ├── service-b/
    └── shared-lib/
```

### 2.2 Repo

A **repo** is a single git repository within the workspace. Each repo may optionally contain a `.harmonia.toml` file with repo-specific configuration (versioning, dependencies, hooks).
Repos can be marked `external` (in graph, excluded from changesets and default mutating ops) or `ignored` (excluded from graph and default ops).

### 2.3 Changeset

A **changeset** is a coordinated set of changes across one or more repositories, typically sharing a branch name. Changesets are the unit of work for Harmonia operations like `harmonia mr create`. When enabled, changesets can be recorded as files under `.harmonia/changesets/`.

### 2.4 Dependency Graph

The **dependency graph** models internal dependencies between repos in the workspace. This enables:

- Computing correct merge order
- Impact analysis ("what breaks if I change X?")
- Automatic inclusion of dependent repos in changesets
- Constraint validation ("will this version bump break dependents?")

### 2.5 Forge

A **forge** is a git hosting platform (GitLab, GitHub, etc.). Harmonia integrates with forges for:

- Creating and linking MRs/PRs
- Checking CI status
- Creating tracking issues
- Automated merge sequencing

---

## 3. Configuration

### 3.1 Workspace Configuration

Location: `.harmonia/config.toml`

```toml
# ============================================================================
# WORKSPACE CONFIGURATION
# ============================================================================

[workspace]
# Human-readable name for this workspace
name = "my-platform"

# Directory where repos are cloned (relative to workspace root)
# Default: "repos"
repos_dir = "repos"

# ============================================================================
# FORGE CONFIGURATION
# ============================================================================

[forge]
# Forge type: "gitlab", "github", "gitea", "forgejo", "bitbucket"
type = "gitlab"

# Forge host (omit for github.com, gitlab.com, etc.)
host = "gitlab.example.com"

# Authentication (can also use environment variables)
# HARMONIA_FORGE_TOKEN takes precedence
# token = "glpat-xxxx"  # Don't commit this; use env var

# Default group/org for repos (if not fully qualified)
default_group = "platform-team"

# ============================================================================
# REPOSITORY DEFINITIONS
# ============================================================================

[repos]
# Minimal: just the repo path (uses defaults)
"shared-lib" = {}

# With explicit remote URL
"service-a" = { url = "git@gitlab.example.com:platform-team/service-a.git" }

# With overrides
"legacy-api" = { 
    url = "git@gitlab.example.com:legacy/api.git",
    default_branch = "master",  # Override default
    package_name = "legacy-api-client",  # If different from repo name
}

# External dependency (not owned, but part of graph)
"external-sdk" = {
    url = "git@github.com:vendor/sdk.git",
    external = true,  # Excluded from changesets and default mutating ops
}

# Ignored repo (tracked in config, but unmanaged)
"scratch" = {
    url = "git@gitlab.example.com:platform-team/scratch.git",
    ignored = true,  # Excluded from graph and default operations
}

# ============================================================================
# REPOSITORY GROUPS
# ============================================================================

# Repo flags:
# - external = true keeps the repo in the dependency graph, but excludes it from changesets and default mutating operations.
# - ignored = true excludes the repo from the dependency graph and all default operations.

[groups]
# Named groups for partial operations
core = ["shared-lib", "service-a", "service-b"]
legacy = ["legacy-api"]
all = ["shared-lib", "service-a", "service-b", "legacy-api"]

# Default group for `harmonia clone` with no arguments
# default = "core"

# ============================================================================
# DEFAULTS
# ============================================================================

[defaults]
# Default branch name for repos (can be overridden per-repo)
default_branch = "main"

# Clone protocol: "ssh" or "https"
clone_protocol = "ssh"

# Clone depth: "full" or number for shallow clone
clone_depth = "full"

# Include untracked files in status/diff operations
include_untracked = true

# ============================================================================
# HOOKS
# ============================================================================

[hooks]
# Workspace-level hooks (run from workspace root)
# These run in addition to repo-level hooks

# Before committing any repo
pre_commit = "harmonia test --changed --fail-fast"

# Before pushing any repo  
pre_push = "harmonia lint --changed"

# After successful MR creation
post_mr_create = "echo 'MRs created successfully'"

# Custom named hooks (invoked with `harmonia run <hook>`)
[hooks.custom]
format = "harmonia exec --changed -- ruff format ."

# ============================================================================
# MERGE REQUEST / PULL REQUEST SETTINGS
# ============================================================================

[mr]
# Template file for MR descriptions (relative to .harmonia/)
template = "templates/mr_description.md"

# How to link related MRs: "related", "description", "issue", "all"
# - related: Use forge's related MR feature (GitLab)
# - description: Add links in MR description
# - issue: Create tracking issue linking all MRs
# - all: All of the above
link_strategy = "all"

# Automatically create tracking issue for multi-repo changesets
create_tracking_issue = true

# Issue template for tracking issues
issue_template = "templates/tracking_issue.md"

# Add git trailers to commits (Changeset-ID, Related-MR, etc.)
add_trailers = true

# Default MR labels
labels = ["harmonia", "multi-repo"]

# Require all tests to pass before allowing `harmonia mr create`
require_tests = true

# Draft MRs by default
draft = false

# ============================================================================
# VERSIONING
# ============================================================================

[versioning]
# Default versioning strategy: "semver", "calver", "none"
strategy = "semver"

# Default bump behavior: "semver", "calver", "tinyinc"
# - semver: standard major/minor/patch increments
# - calver: update date tokens, then increment the rightmost numeric segment (if present)
# - tinyinc: increment the rightmost numeric segment, ignore level
bump_mode = "semver"

# strategy controls parsing/validation, bump_mode controls bump behavior

# For calver: format string
# calver_format = "YYYY.0M.MICRO"

# Automatically bump dependents when a dependency is bumped
cascade_bumps = false

# ============================================================================
# CHANGESETS (optional feature)
# ============================================================================

[changesets]
# Enable changeset files (like changesets in JS ecosystem)
enabled = false

# Directory for changeset files
dir = "changesets"
```

### 3.2 Repository Configuration

Location: `.harmonia.toml` in repository root (optional)

```toml
# ============================================================================
# REPOSITORY CONFIGURATION
# ============================================================================

[package]
# Package/artifact name (may differ from repo name)
# Used for dependency resolution
name = "shared-lib"

# Ecosystem: "python", "rust", "node", "go", "java", "custom"
ecosystem = "python"

# ============================================================================
# VERSIONING
# ============================================================================

[versioning]
# File containing version
file = "pyproject.toml"

# Path to version within file (TOML/JSON/YAML path)
# Use dot notation for nested keys
path = "project.version"

# Versioning strategy (overrides workspace default)
strategy = "semver"

# Bump mode (overrides workspace default)
bump_mode = "semver"

# For non-standard version locations, use regex
# pattern = 'VERSION = "(\d+\.\d+\.\d+)"'

# ============================================================================
# DEPENDENCIES
# ============================================================================

[dependencies]
# File containing dependencies
file = "pyproject.toml"

# Path to dependencies array/table
path = "project.dependencies"

# Pattern to identify internal packages (regex)
# Matched against package names; only matches are included in graph
internal_pattern = "^(shared-|service-|legacy-)"

# Alternative: explicit list of internal package names
# internal_packages = ["shared-lib", "service-a-client"]

# Repo identity is always the repo key defined in workspace `[repos]`. Dependency matching uses package names
# (`package.name` or `package_name`) to map back to repo keys when building the graph.

# ============================================================================
# HOOKS
# ============================================================================

[hooks]
# Repo-specific hooks (run from repo root)

# Disable workspace hooks by name for this repo
disable_workspace_hooks = ["pre_commit"]

# Before commit
pre_commit = "uv run pytest -x"

# Before push
pre_push = "uv run ruff check . && uv run mypy ."

# Custom named hooks (can be invoked with `harmonia run <hook>`)
[hooks.custom]
test = "uv run pytest"
lint = "uv run ruff check ."
format = "uv run ruff format ."
typecheck = "uv run mypy ."

# ============================================================================
# CI
# ============================================================================

[ci]
# Required pipeline/check names that must pass before merge
required_checks = ["test", "lint", "typecheck"]

# Timeout for waiting on CI (for `harmonia mr merge --wait`)
timeout_minutes = 30
```

### 3.3 Environment Variables

| Variable | Purpose |
|----------|---------|
| `HARMONIA_FORGE_TOKEN` | Authentication token for forge API |
| `HARMONIA_WORKSPACE` | Override workspace root detection |
| `HARMONIA_CONFIG` | Override config file location |
| `HARMONIA_REPOS_DIR` | Override repos directory |
| `HARMONIA_LOG_LEVEL` | Logging verbosity: error, warn, info, debug, trace |
| `HARMONIA_NO_COLOR` | Disable colored output |
| `HARMONIA_PARALLEL` | Default parallelism for operations (default: num CPUs) |

### 3.4 Configuration Resolution

Configuration is resolved in layers (later overrides earlier):

1. Built-in defaults
2. Workspace config (`.harmonia/config.toml`)
3. Repo config (`.harmonia.toml` in each repo)
4. Environment variables
5. CLI flags

Hooks are merged, not overridden. Workspace hooks run first, then repo hooks of the same name, unless the repo config disables the workspace hook via `hooks.disable_workspace_hooks`.

---

### 3.5 Changeset Files

When `changesets.enabled = true`, Harmonia can read and write changeset files in `.harmonia/changesets/`.
These files capture intent and per-repo summaries, and are used by `harmonia plan` and `harmonia mr create`.

Example (`.harmonia/changesets/cs-2026-02-05-auth.toml`):

```toml
id = "cs-2026-02-05-auth"
title = "feat: add authentication"
description = "Introduce shared auth helpers and integrate them into service-a."
branch = "feature/auth"

[[repos]]
repo = "shared-lib"
summary = "Add token validation helpers."

[[repos]]
repo = "service-a"
summary = "Use shared-lib auth helpers."
```

## 4. CLI Reference

### 4.1 Global Options

```
harmonia [OPTIONS] <COMMAND>

Options:
    -w, --workspace <PATH>    Workspace root (default: auto-detect)
    -c, --config <PATH>       Config file path
    -v, --verbose             Increase verbosity (-v, -vv, -vvv)
    -q, --quiet               Suppress non-essential output
        --no-color            Disable colored output
    -h, --help                Print help
    -V, --version             Print version
```

Repo arguments (`REPOS`, `<REPO>`) always refer to repo keys from workspace `[repos]`.

### 4.2 Workspace Commands

#### `harmonia init`

Initialize a new workspace.

```
harmonia init [OPTIONS] [SOURCE]

Arguments:
    [SOURCE]    Git URL or local path to clone workspace config from
                If omitted, creates empty workspace in current directory

Options:
    -n, --name <NAME>         Workspace name
    -d, --directory <PATH>    Target directory (default: current or derived from SOURCE)
        --no-clone            Don't clone repos after init
        --group <GROUP>       Only clone repos in this group
    
Examples:
    harmonia init
    harmonia init --name my-workspace
    harmonia init git@gitlab.com:team/workspace-config.git
    harmonia init ./existing-config --directory ./my-workspace
```

#### `harmonia clone`

Clone repositories into the workspace.

```
harmonia clone [OPTIONS] [REPOS]...

Arguments:
    [REPOS]...    Specific repos to clone (default: all or default group)

Options:
    -g, --group <GROUP>       Clone repos in this group
    -a, --all                 Clone all defined repos
        --depth <N|full>      Shallow clone depth or "full"
        --full                Force full clone
        --protocol <PROTO>    Clone protocol: ssh, https

Examples:
    harmonia clone                      # Clone default group
    harmonia clone --all                # Clone everything
    harmonia clone --group=core         # Clone 'core' group
    harmonia clone service-a shared-lib # Clone specific repos
```

#### `harmonia sync`

Synchronize repos with their remotes.

```
harmonia sync [OPTIONS] [REPOS]...

Arguments:
    [REPOS]...    Repos to sync (default: all cloned)

Options:
    -r, --rebase              Pull with rebase instead of merge
        --ff-only             Only fast-forward (fail if not possible)
    -f, --fetch-only          Fetch without merging
    -p, --prune               Prune deleted remote branches
        --parallel <N>        Parallelism (default: from config)

Examples:
    harmonia sync
    harmonia sync --rebase
    harmonia sync service-a service-b
```

#### `harmonia status`

Show workspace status overview.

```
harmonia status [OPTIONS]

Options:
    -s, --short               Short format (one line per repo)
    -l, --long                Long format (full git status per repo)
        --json                Output as JSON
        --changed             Only show repos with changes
        --porcelain           Machine-readable format

Examples:
    harmonia status
    harmonia status --short
    harmonia status --changed --json
```

Output:
```
Workspace: my-platform (4 repos)

Repo          Branch              Status       ↑↓    Deps
────────────────────────────────────────────────────────────
shared-lib    feature/auth        2 staged     ↑2    ─
service-a     feature/auth        3 modified   ↑1    shared-lib
service-b     main                clean        ✓     shared-lib
legacy-api    main                clean        ↓3    ─

Legend: ↑ ahead, ↓ behind, ─ no internal deps
```

### 4.3 Graph Commands

#### `harmonia graph`

Display and query the dependency graph.

```
harmonia graph [OPTIONS] [COMMAND]

Commands:
    show        Display the dependency graph (default)
    deps        Show dependencies of a repo
    dependents  Show what depends on a repo
    order       Show topological order
    check       Validate dependency constraints

Options (for 'show'):
        --changed             Only show changed repos and their deps
        --format <FMT>        Output format: tree, flat, dot, json
        --direction <DIR>     Direction: down (deps), up (dependents), both

Examples:
    harmonia graph
    harmonia graph --changed
    harmonia graph --format=dot | dot -Tpng > graph.png
    harmonia graph deps shared-lib
    harmonia graph dependents shared-lib
    harmonia graph order --changed
    harmonia graph check
```

Output (tree format):
```
Dependency Graph
════════════════

shared-lib (v1.2.0)
├─► service-a (v2.0.0)
│   └─► gateway (v1.5.0)
├─► service-b (v1.8.0)
│   └─► gateway (v1.5.0)
└─► legacy-api (v0.9.0)

Legend: ─► depends on
```

#### `harmonia graph deps`

```
harmonia graph deps <REPO>

Arguments:
    <REPO>    Repo key to query

Options:
    -t, --transitive    Include transitive dependencies
        --json          Output as JSON
```

#### `harmonia graph dependents`

```
harmonia graph dependents <REPO>

Arguments:
    <REPO>    Repo key to query

Options:
    -t, --transitive    Include transitive dependents
        --json          Output as JSON
```

#### `harmonia graph check`

```
harmonia graph check [OPTIONS]

Options:
        --fix               Suggest fixes (no file edits)
        --json              Output as JSON

Checks:
    - Circular dependencies
    - Unsatisfiable version constraints
    - Missing internal dependencies
    - Version conflicts
```

### 4.4 Git Coordination Commands

#### `harmonia branch`

Switch branches across repos. Use `-c` or `-C` to create if missing.

```
harmonia branch [OPTIONS] <NAME>

Arguments:
    <NAME>    Branch name to create or switch to

Options:
    -c, --create              Create new branch (error if exists)
    -C, --force-create        Create new branch (overwrite if exists)
        --yes                 Skip confirmation prompts
        --repos <REPOS>       Specific repos (comma-separated)
        --changed             Only repos with changes
        --with-deps           Include downstream dependents
        --with-all-deps       Include full dependency tree
    -t, --track <BRANCH>      Set upstream tracking branch
    
Behavior:
    - `--force-create` prompts for confirmation unless `--yes` is provided.

Examples:
    harmonia branch feature/auth              # Switch (error if missing)
    harmonia branch -c feature/auth           # Create in all repos
    harmonia branch feature/auth --changed    # Only repos with changes
    harmonia branch feature/auth --with-deps  # Include dependents
```

#### `harmonia checkout`

Checkout branches across repos.

```
harmonia checkout [OPTIONS] <BRANCH>

Arguments:
    <BRANCH>    Branch to checkout

Options:
        --repos <REPOS>       Specific repos
        --all                 All cloned repos
        --graceful            Don't error if branch missing (stay on current)
        --fallback <BRANCH>   Fallback branch if target missing

Examples:
    harmonia checkout main --all
    harmonia checkout feature/auth --graceful
    harmonia checkout feature/auth --fallback=main
```

#### `harmonia add`

Stage changes.

```
harmonia add [OPTIONS] [PATHSPEC]...

Arguments:
    [PATHSPEC]...    Paths to add (default: all changes)

Options:
        --repos <REPOS>    Specific repos
    -A, --all              Stage all changes in all repos
    -p, --patch            Interactive staging
```

#### `harmonia commit`

Commit changes across repos.

```
harmonia commit [OPTIONS]

Options:
    -m, --message <MSG>       Commit message
    -a, --all                 Stage all changes before committing
        --repos <REPOS>       Specific repos
        --amend               Amend previous commit (prompts for confirmation)
        --no-hooks            Skip pre-commit hooks
        --yes                 Skip confirmation prompts
        --allow-empty         Allow empty commits
        --trailer <K=V>       Add git trailer (can be repeated)

Examples:
    harmonia commit -m "feat: add auth support"
    harmonia commit -am "fix: resolve race condition"
    harmonia commit -m "chore: update deps" --repos=service-a,service-b
```

#### `harmonia push`

Push commits to remotes.

```
harmonia push [OPTIONS]

Options:
        --repos <REPOS>       Specific repos
    -f, --force               Force push (DANGEROUS)
        --force-with-lease    Force push with lease (safer)
    -u, --set-upstream        Set upstream for new branches
        --no-hooks            Skip pre-push hooks
        --yes                 Skip confirmation prompts
        --dry-run             Show what would be pushed

Behavior:
    - Force options require confirmation unless `--yes` is provided.

Examples:
    harmonia push
    harmonia push --set-upstream
    harmonia push --force-with-lease  # After rebase
```

#### `harmonia diff`

Show diffs across repos.

```
harmonia diff [OPTIONS] [REPOS]...

Options:
        --staged              Show staged changes
        --stat                Show diffstat only
        --name-only           Show only file names
        --unified <N>         Context lines (default: 3)
        --format <FMT>        Output format: patch, stat, json

Examples:
    harmonia diff
    harmonia diff --stat
    harmonia diff --staged service-a
```

### 4.5 Testing & Validation Commands

#### `harmonia test`

Run tests across repos.

```
harmonia test [OPTIONS] [REPOS]...

Arguments:
    [REPOS]...    Repos to test (default: all with changes)

Options:
        --all                 Test all repos
        --changed             Only repos with changes (default)
        --graph-order         Test in dependency order
        --parallel <N>        Parallel jobs (default: 1 for graph-order, N otherwise)
        --fail-fast           Stop on first failure
        --coverage            Collect coverage
    -k, --filter <PATTERN>    Filter tests by pattern

Examples:
    harmonia test
    harmonia test --all --parallel=4
    harmonia test --graph-order --fail-fast
    harmonia test service-a -k "test_auth"
```

#### `harmonia lint`

Run linters across repos.

```
harmonia lint [OPTIONS] [REPOS]...

Options:
        --all                 Lint all repos
        --changed             Only repos with changes (default)
        --fix                 Auto-fix where possible
        --parallel <N>        Parallel jobs

Examples:
    harmonia lint
    harmonia lint --fix
    harmonia lint --all --parallel=4
```

#### `harmonia exec`

Execute arbitrary command in repos.

```
harmonia exec [OPTIONS] -- <COMMAND>...

Arguments:
    <COMMAND>...    Command to execute

Options:
        --repos <REPOS>       Specific repos
        --all                 All repos
        --changed             Only changed repos
        --parallel <N>        Parallel execution
        --fail-fast           Stop on first failure
        --ignore-errors       Continue despite errors

Examples:
    harmonia exec -- git log --oneline -5
    harmonia exec --parallel=4 -- make build
    harmonia exec --repos=service-a,service-b -- cargo check
```

### 4.6 Versioning Commands

#### `harmonia version`

Display and manage versions.

```
harmonia version [OPTIONS] [COMMAND]

Commands:
    show        Show current versions (default)
    check       Verify constraints are satisfiable
    bump        Bump versions

Options (for 'show'):
        --json              Output as JSON
        --with-deps         Show with dependency info

Examples:
    harmonia version
    harmonia version check
    harmonia version bump patch
```

#### `harmonia version bump`

Bump versions in repos.

```
harmonia version bump [OPTIONS] [LEVEL]

Arguments:
    [LEVEL]    Bump level: major, minor, patch (default: auto from commits)

Options:
        --repos <REPOS>       Specific repos
        --changed             Only changed repos (default)
        --mode <MODE>         Bump mode: semver, calver, tinyinc (default: from config)
        --dry-run             Show what would change
        --cascade             Also bump dependents
        --no-commit           Don't commit changes
        --pre <TAG>           Prerelease tag (e.g., alpha, beta, rc.1)

Notes:
    - For `tinyinc`, the bump level is ignored and the rightmost numeric segment is incremented.
    - For `calver`, date tokens are updated to today, then the rightmost numeric segment (if any) is incremented.

Examples:
    harmonia version bump patch
    harmonia version bump minor --repos=shared-lib --cascade
    harmonia version bump --dry-run
```

#### `harmonia deps`

Manage internal dependencies.

```
harmonia deps [OPTIONS] [COMMAND]

Commands:
    show        Show internal dependency versions
    check       Check for constraint violations
    update      Update internal deps to current versions

Examples:
    harmonia deps show
    harmonia deps check
    harmonia deps update              # Update all internal deps
    harmonia deps update shared-lib   # Update shared-lib dep in all dependents
```

### 4.7 MR/PR Commands

#### `harmonia plan`

Analyze and plan a changeset.

```
harmonia plan [OPTIONS]

Options:
        --json              Output as JSON
        --include <REPOS>   Force include repos
        --exclude <REPOS>   Exclude repos

Output:
    - Changed repos with diff stats
    - Computed merge order
    - Dependency constraint analysis
    - Potential issues/warnings
```

Example output:
```
Changeset Analysis
══════════════════════════════════════════════════════════════════════

Changed Repositories
────────────────────
  shared-lib    +142 -23  (4 files)   [feature/auth]
  service-a     +89 -12   (2 files)   [feature/auth]  
  gateway       +15 -3    (1 file)    [feature/auth]

Merge Order (topological)
─────────────────────────
  1. shared-lib
     ├─ No internal dependencies
     └─ Dependents in changeset: service-a, gateway
     
  2. service-a
     ├─ Depends on: shared-lib (>=1.2.0) ✓
     └─ Dependents in changeset: gateway
     
  3. gateway
     ├─ Depends on: shared-lib (>=1.2.0) ✓, service-a (>=2.0.0) ✓
     └─ No dependents in changeset

Validation
──────────
  ✓ All dependency constraints satisfiable
  ✓ No circular dependencies
  ⚠ service-a pins shared-lib==1.2.0 (exact); bump will require update

Recommendations
───────────────
  • Run: harmonia deps update shared-lib
  • Wait for CI/publish between merge steps
```

#### `harmonia mr`

Manage merge/pull requests.

```
harmonia mr [COMMAND]

Commands:
    create      Create MRs for current changeset
    status      Check status of changeset MRs
    update      Update MR descriptions
    merge       Merge MRs in order
    close       Close all MRs in changeset
```

#### `harmonia mr create`

```
harmonia mr create [OPTIONS]

Options:
    -t, --title <TITLE>       MR title (shared across all MRs)
    -d, --description <DESC>  Additional description
        --draft               Create as draft/WIP
        --no-link             Don't link MRs together
        --no-issue            Don't create tracking issue
        --labels <LABELS>     Additional labels (comma-separated)
        --reviewers <USERS>   Assign reviewers (comma-separated)
        --dry-run             Show what would be created

Examples:
    harmonia mr create -t "feat: add authentication"
    harmonia mr create --draft
    harmonia mr create --dry-run
```

#### `harmonia mr status`

```
harmonia mr status [OPTIONS]

Options:
        --json              Output as JSON
        --wait              Wait for CI to complete
        --timeout <MIN>     Timeout for --wait (default: 30)

Output:
    - MR state (open, merged, closed)
    - CI status
    - Approvals
    - Merge conflicts
```

#### `harmonia mr merge`

```
harmonia mr merge [OPTIONS]

Options:
        --dry-run             Show merge order without merging
        --no-wait             Don't wait for CI between merges
        --squash              Squash merge
        --delete-branch       Delete source branches after merge
    -y, --yes                 Skip confirmation prompts

Behavior:
    1. Validates all MRs are approved and CI passing
    2. Merges in dependency order
    3. Waits for CI/publish between steps (unless --no-wait)
    4. Updates dependent MR branches if needed
```

### 4.8 Utility Commands

#### `harmonia run`

Run a named hook defined in `hooks.custom`.

```
harmonia run [OPTIONS] <HOOK>

Arguments:
    <HOOK>    Hook name to run (from `hooks.custom`)

Options:
        --repos <REPOS>       Specific repos
        --all                 All repos
        --changed             Only repos with changes
        --parallel <N>        Parallel execution
        --fail-fast           Stop on first failure

Behavior:
    - Runs the workspace hook once at the workspace root, if defined, unless any selected repo disables it.
    - Runs the repo hook in each selected repo.
```

Examples:
    harmonia run test
    harmonia run format --changed

#### `harmonia each`

Run command in each repo (lower-level than `exec`).

```
harmonia each [OPTIONS] -- <COMMAND>...

Options:
        --repos <REPOS>       Specific repos
        --parallel <N>        Parallel execution
        --shell               Run through shell

Examples:
    harmonia each -- pwd
    harmonia each --parallel=4 -- make clean
```

#### `harmonia shell`

Enter a shell with workspace environment.

```
harmonia shell [OPTIONS]

Options:
        --repos <REPOS>       Set up for specific repos
        --command <CMD>       Run command instead of interactive shell

Environment:
    - PYTHONPATH prepended with repo src directories
    - PATH includes repo bin directories
    - HARMONIA_WORKSPACE set
```

#### `harmonia edit`

Open repos in editor.

```
harmonia edit [OPTIONS] [REPOS]...

Arguments:
    [REPOS]...    Repos to open (default: workspace root)

Options:
        --editor <EDITOR>     Editor command (default: $EDITOR or code)
        --all                 Open all changed repos

Examples:
    harmonia edit                  # Open workspace root
    harmonia edit service-a        # Open specific repo
    harmonia edit --all            # Open all changed repos
```

#### `harmonia clean`

Clean workspace.

```
harmonia clean [OPTIONS]

Options:
        --repos <REPOS>       Specific repos
    -f, --force               Actually delete (default: dry run)
    -d, --directories         Remove untracked directories
    -x, --ignored             Remove ignored files too
```

### 4.9 Configuration Commands

#### `harmonia config`

View and manage configuration.

```
harmonia config [COMMAND]

Commands:
    show        Show resolved configuration
    get         Get a config value
    set         Set a config value
    edit        Open config in editor

Examples:
    harmonia config show
    harmonia config get forge.type
    harmonia config set defaults.default_branch main
    harmonia config edit
```

#### `harmonia repo`

Manage repo definitions.

```
harmonia repo [COMMAND]

Commands:
    list        List defined repos
    add         Add a repo to config
    remove      Remove a repo from config
    show        Show repo details

Examples:
    harmonia repo list
    harmonia repo add new-service --url=git@... --group=core
    harmonia repo remove deprecated-thing
    harmonia repo show service-a
```

---

## 5. Dependency Graph Engine

### 5.1 Graph Construction

The dependency graph is built by:

1. Scanning each repo for `.harmonia.toml` (or using ecosystem defaults)
2. Parsing the dependency file (pyproject.toml, Cargo.toml, etc.)
3. Filtering to only internal packages (using `internal_pattern` or `internal_packages`)
4. Building a directed acyclic graph (DAG)

### 5.2 Core Operations

```rust
/// Dependency graph for workspace repositories
pub struct DependencyGraph {
    /// Adjacency list: repo -> repos it depends on
    dependencies: HashMap<RepoId, HashSet<RepoId>>,
    /// Reverse adjacency: repo -> repos that depend on it  
    dependents: HashMap<RepoId, HashSet<RepoId>>,
    /// Version constraints: (from, to) -> constraint
    constraints: HashMap<(RepoId, RepoId), VersionReq>,
    /// Current versions
    versions: HashMap<RepoId, Version>,
}

impl DependencyGraph {
    /// Build graph from workspace
    pub fn from_workspace(workspace: &Workspace) -> Result<Self>;
    
    /// Direct dependencies of a repo
    pub fn dependencies(&self, repo: &RepoId) -> &HashSet<RepoId>;
    
    /// Direct dependents of a repo
    pub fn dependents(&self, repo: &RepoId) -> &HashSet<RepoId>;
    
    /// Transitive dependencies (full tree down)
    pub fn transitive_dependencies(&self, repo: &RepoId) -> HashSet<RepoId>;
    
    /// Transitive dependents (full tree up)
    pub fn transitive_dependents(&self, repo: &RepoId) -> HashSet<RepoId>;
    
    /// Topological sort (all repos)
    pub fn topological_order(&self) -> Result<Vec<RepoId>>;
    
    /// Topological sort (subset)
    pub fn merge_order(&self, repos: &[RepoId]) -> Result<Vec<RepoId>>;
    
    /// Check for cycles
    pub fn find_cycles(&self) -> Vec<Vec<RepoId>>;
    
    /// Validate all constraints
    pub fn check_constraints(&self) -> Vec<ConstraintViolation>;
    
    /// Check if bumping a repo would break constraints
    pub fn validate_bump(&self, repo: &RepoId, new_version: &Version) -> Vec<ConstraintViolation>;
    
    /// Get repos that would need updates if repo is bumped
    pub fn cascade_impact(&self, repo: &RepoId) -> HashSet<RepoId>;
}
```

### 5.3 Constraint Validation

```rust
pub struct ConstraintViolation {
    pub from_repo: RepoId,
    pub to_repo: RepoId,
    pub constraint: VersionReq,
    pub actual_version: Version,
    pub violation_type: ViolationType,
}

pub enum ViolationType {
    /// Current version doesn't satisfy constraint
    Unsatisfied,
    /// Exact pin that would break on bump
    ExactPin,
    /// Upper bound that would break on bump
    UpperBound,
    /// Circular dependency detected
    Circular,
}
```

### 5.4 Graph Visualization

Support multiple output formats:

- **Tree**: ASCII tree representation (default)
- **Flat**: Simple list with indentation
- **DOT**: Graphviz DOT format for visualization
- **JSON**: Machine-readable format

---

## 6. Forge Integration

### 6.1 Forge Abstraction

```rust
#[async_trait]
pub trait Forge: Send + Sync {
    /// Create a merge/pull request
    async fn create_mr(&self, repo: &RepoId, params: CreateMrParams) -> Result<MergeRequest>;
    
    /// Get MR details
    async fn get_mr(&self, repo: &RepoId, mr_id: MrId) -> Result<MergeRequest>;
    
    /// Update MR
    async fn update_mr(&self, repo: &RepoId, mr_id: MrId, params: UpdateMrParams) -> Result<MergeRequest>;
    
    /// Link MRs as related
    async fn link_mrs(&self, mrs: &[(RepoId, MrId)]) -> Result<()>;
    
    /// Merge MR
    async fn merge_mr(&self, repo: &RepoId, mr_id: MrId, params: MergeMrParams) -> Result<()>;
    
    /// Close MR without merging
    async fn close_mr(&self, repo: &RepoId, mr_id: MrId) -> Result<()>;
    
    /// Get CI/pipeline status
    async fn get_ci_status(&self, repo: &RepoId, ref_: &str) -> Result<CiStatus>;
    
    /// Create issue
    async fn create_issue(&self, params: CreateIssueParams) -> Result<Issue>;
    
    /// Get user info (for mentions, assignments)
    async fn get_user(&self, username: &str) -> Result<User>;
}
```

### 6.2 Supported Forges

| Forge | MR Linking | Tracking Issues | CI Status | Notes |
|-------|------------|-----------------|-----------|-------|
| GitLab | ✓ Related MRs | ✓ | ✓ | Full support |
| GitHub | ✓ Linked PRs | ✓ | ✓ | Full support |
| Gitea | ✓ Description | ✓ | ✓ | Limited API |
| Forgejo | ✓ Description | ✓ | ✓ | Gitea-compatible |
| Bitbucket | ✓ Description | ✓ | ✓ | Partial support |

### 6.3 MR Description Template

Default template (`.harmonia/templates/mr_description.md`):

```markdown
{{ description }}

---

## 🔗 Coordinated Changeset

This MR is part of a coordinated change across multiple repositories.

| Repo | MR | Status | Merge Order |
|------|-----|--------|-------------|
{% for mr in changeset.mrs %}
| {{ mr.repo }} | {{ mr.link }} | {{ mr.status_emoji }} {{ mr.status }} | {{ mr.merge_order }} |
{% endfor %}

**Merge instruction**: Merge in dependency order. Wait for CI/publish between steps.

<details>
<summary>Dependency details</summary>

{% for mr in changeset.mrs %}
### {{ mr.repo }}
- **Depends on**: {{ mr.dependencies | join(", ") | default("none") }}
- **Depended on by**: {{ mr.dependents | join(", ") | default("none") }}
{% endfor %}

</details>

<!-- harmonia:changeset:{{ changeset.id }} -->
```

### 6.4 Tracking Issue Template

Default template (`.harmonia/templates/tracking_issue.md`):

```markdown
## Coordinated Change: {{ title }}

### Summary

{{ description }}

### Merge Requests

{% for mr in changeset.mrs %}
- [ ] {{ mr.repo }} {{ mr.link }} (merge order: {{ mr.merge_order }})
{% endfor %}

### Merge Order

{% for mr in changeset.mrs | sort(attribute="merge_order") %}
{{ mr.merge_order }}. **{{ mr.repo }}** {{ mr.link }}
   - Depends on: {{ mr.dependencies | join(", ") | default("none") }}
{% endfor %}

### Status

- **Created**: {{ now | date }}
- **Changeset ID**: `{{ changeset.id }}`
- **Branch**: `{{ changeset.branch }}`

<!-- harmonia:tracking:{{ changeset.id }} -->
```

---

## 7. Ecosystem Plugins

### 7.1 Plugin Interface

```rust
pub trait EcosystemPlugin: Send + Sync {
    /// Plugin identifier
    fn id(&self) -> &str;
    
    /// File patterns this plugin can parse
    fn file_patterns(&self) -> &[&str];
    
    /// Parse version from file content
    fn parse_version(&self, path: &Path, content: &str) -> Result<Option<Version>>;
    
    /// Parse dependencies from file content
    fn parse_dependencies(&self, path: &Path, content: &str) -> Result<Vec<Dependency>>;
    
    /// Update version in file content
    fn update_version(&self, path: &Path, content: &str, new_version: &Version) -> Result<String>;
    
    /// Update dependency constraint in file content
    fn update_dependency(&self, path: &Path, content: &str, dep: &str, constraint: &str) -> Result<String>;
    
    /// Get default test command
    fn default_test_command(&self) -> Option<&str>;
    
    /// Get default lint command
    fn default_lint_command(&self) -> Option<&str>;
}
```

### 7.2 Built-in Plugins

#### Python (`python`)

- **Version files**: `pyproject.toml`, `setup.py`, `__version__.py`
- **Dependency files**: `pyproject.toml`, `requirements.txt`, `setup.py`
- **Default test**: `pytest` or `python -m pytest`
- **Default lint**: `ruff check .` or `flake8`

#### Rust (`rust`)

- **Version files**: `Cargo.toml`
- **Dependency files**: `Cargo.toml`
- **Default test**: `cargo test`
- **Default lint**: `cargo clippy`

#### Node.js (`node`)

- **Version files**: `package.json`
- **Dependency files**: `package.json`
- **Default test**: `npm test` or `yarn test`
- **Default lint**: `npm run lint` or `eslint .`

#### Go (`go`)

- **Version files**: (Go modules don't have traditional versions)
- **Dependency files**: `go.mod`
- **Default test**: `go test ./...`
- **Default lint**: `golangci-lint run`

### 7.3 Custom Plugins

Users can define custom parsers in config:

```toml
[ecosystems.custom-cpp]
version_file = "VERSION.txt"
version_regex = '^(\d+\.\d+\.\d+)$'
deps_file = "deps.cmake"
deps_regex = 'find_package\((\w+)\s+(\d+\.\d+)\)'
internal_pattern = "^mycompany-"
test_command = "make test"
lint_command = "clang-tidy"
```

---

## 8. Data Structures

### 8.1 Core Types

```rust
/// Unique identifier for a repository in the workspace
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoId(String);

/// Repository information
#[derive(Debug, Clone)]
pub struct Repo {
    pub id: RepoId,
    pub path: PathBuf,
    pub remote_url: String,
    pub default_branch: String,
    pub package_name: Option<String>,
    pub ecosystem: Option<Ecosystem>,
    pub config: Option<RepoConfig>,
}

/// Workspace state
#[derive(Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: WorkspaceConfig,
    pub repos: HashMap<RepoId, Repo>,
    pub graph: DependencyGraph,
}

/// Changeset representing coordinated changes
#[derive(Debug, Clone)]
pub struct Changeset {
    pub id: ChangesetId,
    pub branch: String,
    pub repos: Vec<RepoId>,
    pub merge_order: Vec<RepoId>,
    pub mrs: HashMap<RepoId, MergeRequest>,
    pub tracking_issue: Option<Issue>,
}

/// Repository status
#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub repo: RepoId,
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub staged: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub untracked: Vec<PathBuf>,
    pub conflicts: Vec<PathBuf>,
}

/// Version parsing strategy
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionKind {
    Semver,
    Calver,
    Raw,
}

/// Version with optional parsed semver
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub raw: String,
    pub kind: VersionKind,
    pub semver: Option<semver::Version>,
}

/// Version requirement/constraint (raw plus parsed semver when applicable)
#[derive(Debug, Clone)]
pub struct VersionReq {
    pub raw: String,
    pub semver: Option<semver::VersionReq>,
}

/// Dependency information
#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub constraint: VersionReq,
    pub is_internal: bool,
}
```

### 8.2 Forge Types

```rust
/// Merge/Pull request
#[derive(Debug, Clone)]
pub struct MergeRequest {
    pub id: MrId,
    pub iid: u64,  // Internal ID (GitLab)
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
    pub state: MrState,
    pub url: String,
    pub ci_status: Option<CiStatus>,
    pub approvals: Vec<User>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MrState {
    Open,
    Merged,
    Closed,
    Draft,
}

#[derive(Debug, Clone)]
pub struct CiStatus {
    pub state: CiState,
    pub pipelines: Vec<Pipeline>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiState {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Skipped,
}

/// Tracking issue
#[derive(Debug, Clone)]
pub struct Issue {
    pub id: IssueId,
    pub iid: u64,
    pub title: String,
    pub url: String,
    pub state: IssueState,
}
```

### 8.3 Configuration Types

```rust
/// Workspace configuration (from .harmonia/config.toml)
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceConfig {
    pub workspace: WorkspaceSettings,
    pub forge: Option<ForgeConfig>,
    pub repos: HashMap<String, RepoEntry>,
    pub groups: Option<HashMap<String, Vec<String>>>,
    pub defaults: Option<DefaultsConfig>,
    pub hooks: Option<HooksConfig>,
    pub mr: Option<MrConfig>,
    pub versioning: Option<VersioningConfig>,
    pub changesets: Option<ChangesetsConfig>,
}

/// Repository configuration (from .harmonia.toml in repo)
#[derive(Debug, Clone, Deserialize)]
pub struct RepoConfig {
    pub package: Option<PackageConfig>,
    pub versioning: Option<RepoVersioningConfig>,
    pub dependencies: Option<DepsConfig>,
    pub hooks: Option<RepoHooksConfig>,
    pub ci: Option<CiConfig>,
}
```

---

## 9. Architecture

### 9.1 Module Structure

```
harmonia/
├── Cargo.toml
├── src/
│   ├── main.rs                 # Entry point, CLI setup
│   ├── lib.rs                  # Library root
│   │
│   ├── cli/                    # CLI command implementations
│   │   ├── mod.rs
│   │   ├── init.rs
│   │   ├── clone.rs
│   │   ├── status.rs
│   │   ├── branch.rs
│   │   ├── commit.rs
│   │   ├── graph.rs
│   │   ├── mr.rs
│   │   ├── version.rs
│   │   └── ...
│   │
│   ├── core/                   # Core domain logic
│   │   ├── mod.rs
│   │   ├── workspace.rs        # Workspace management
│   │   ├── repo.rs             # Repository operations
│   │   ├── changeset.rs        # Changeset tracking
│   │   └── version.rs          # Version parsing/manipulation
│   │
│   ├── graph/                  # Dependency graph engine
│   │   ├── mod.rs
│   │   ├── builder.rs          # Graph construction
│   │   ├── ops.rs              # Graph operations
│   │   ├── constraint.rs       # Constraint checking
│   │   └── viz.rs              # Visualization (DOT, etc.)
│   │
│   ├── forge/                  # Forge integrations
│   │   ├── mod.rs
│   │   ├── traits.rs           # Forge trait definition
│   │   ├── gitlab.rs
│   │   ├── github.rs
│   │   ├── gitea.rs
│   │   └── bitbucket.rs
│   │
│   ├── ecosystem/              # Language ecosystem plugins
│   │   ├── mod.rs
│   │   ├── traits.rs           # Plugin trait definition
│   │   ├── python.rs
│   │   ├── rust.rs
│   │   ├── node.rs
│   │   ├── go.rs
│   │   └── custom.rs           # Custom plugin from config
│   │
│   ├── git/                    # Git operations
│   │   ├── mod.rs
│   │   ├── ops.rs              # High-level operations
│   │   ├── status.rs           # Status parsing
│   │   └── diff.rs             # Diff operations
│   │
│   ├── config/                 # Configuration handling
│   │   ├── mod.rs
│   │   ├── workspace.rs        # Workspace config parsing
│   │   ├── repo.rs             # Repo config parsing
│   │   └── resolve.rs          # Config resolution logic
│   │
│   └── util/                   # Utilities
│       ├── mod.rs
│       ├── output.rs           # Terminal output, colors
│       ├── template.rs         # Template rendering
│       └── parallel.rs         # Parallel execution
│
└── tests/
    ├── integration/
    └── fixtures/
```

### 9.2 Key Dependencies

```toml
[dependencies]
# CLI
clap = { version = "4", features = ["derive", "env"] }
clap_complete = "4"

# Async runtime
tokio = { version = "1", features = ["full"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
serde_yaml = "0.9"

# Git
gix = "0.78"

# HTTP client (for forge APIs)
reqwest = { version = "0.11", features = ["json"] }

# Graph algorithms
petgraph = "0.6"

# Semver
semver = "1"

# Template rendering
tera = "1"

# Terminal output
console = "0.15"
indicatif = "0.17"
dialoguer = "0.11"

# Regex
regex = "1"

# Error handling
thiserror = "1"
anyhow = "1"

# Parallel execution
rayon = "1"

# Globbing
glob = "0.3"
```

### 9.3 Error Handling Strategy

```rust
/// Top-level error type
#[derive(Debug, thiserror::Error)]
pub enum HarmoniaError {
    #[error("Workspace error: {0}")]
    Workspace(#[from] WorkspaceError),
    
    #[error("Git error: {0}")]
    Git(#[from] anyhow::Error),
    
    #[error("Forge error: {0}")]
    Forge(#[from] ForgeError),
    
    #[error("Graph error: {0}")]
    Graph(#[from] GraphError),
    
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type alias
pub type Result<T> = std::result::Result<T, HarmoniaError>;
```

---

## 10. Implementation Phases

### Phase 1: Core Foundation (MVP)

**Goal**: Basic workspace management and status visibility

**Deliverables**:
- [ ] Project scaffolding (Cargo, CI, tests)
- [ ] Configuration parsing (workspace + repo configs)
- [ ] `harmonia init` - Initialize workspace
- [ ] `harmonia clone` - Clone repos
- [ ] `harmonia status` - Show workspace status
- [ ] `harmonia sync` - Pull repos
- [ ] `harmonia exec` - Run commands across repos
- [ ] Basic terminal output formatting

**Duration**: ~2 weeks

### Phase 2: Git Coordination

**Goal**: Coordinated git operations across repos

**Deliverables**:
- [ ] `harmonia branch` - Coordinated branching
- [ ] `harmonia checkout` - Coordinated checkout
- [ ] `harmonia add` - Stage changes
- [ ] `harmonia commit` - Coordinated commits
- [ ] `harmonia push` - Coordinated push
- [ ] `harmonia diff` - Cross-repo diff
- [ ] Hook system (pre-commit, pre-push)

**Duration**: ~2 weeks

### Phase 3: Dependency Graph

**Goal**: Dependency awareness and analysis

**Deliverables**:
- [ ] Ecosystem plugins (Python first, then Rust/Node)
- [ ] Graph construction from repo configs
- [ ] `harmonia graph show` - Visualize graph
- [ ] `harmonia graph deps/dependents` - Query graph
- [ ] `harmonia graph check` - Validate constraints
- [ ] Topological sorting for merge order
- [ ] Impact analysis

**Duration**: ~2 weeks

### Phase 4: Versioning

**Goal**: Version management and bumping

**Deliverables**:
- [ ] Version parsing (semver + calver + tinyinc)
- [ ] `harmonia version show` - Display versions
- [ ] `harmonia version bump` - Bump versions
- [ ] `harmonia deps update` - Update internal deps
- [ ] Cascade bump detection
- [ ] Constraint validation on bump

**Duration**: ~1 week

### Phase 5: Forge Integration (GitLab)

**Goal**: MR creation and management for GitLab

**Deliverables**:
- [ ] GitLab API client
- [ ] `harmonia plan` - Changeset analysis
- [ ] `harmonia mr create` - Create linked MRs
- [ ] `harmonia mr status` - Check MR status
- [ ] `harmonia mr update` - Update MR descriptions
- [ ] Tracking issue creation
- [ ] MR description templates

**Duration**: ~2 weeks

### Phase 6: Testing & Quality

**Goal**: Test/lint orchestration

**Deliverables**:
- [ ] `harmonia test` - Run tests across repos
- [ ] `harmonia lint` - Run linters
- [ ] Graph-order execution
- [ ] Parallel execution
- [ ] Coverage aggregation (optional)

**Duration**: ~1 week

### Phase 7: Additional Forges

**Goal**: Support for GitHub, Gitea

**Deliverables**:
- [ ] GitHub API client
- [ ] Gitea API client
- [ ] Forge abstraction refinement
- [ ] Cross-forge testing

**Duration**: ~2 weeks

### Phase 8: Polish & Advanced Features

**Goal**: Production readiness

**Deliverables**:
- [ ] `harmonia mr merge` - Orchestrated merging
- [ ] `harmonia shell` - Environment shell
- [ ] Interactive mode / TUI
- [ ] Shell completions
- [ ] Man pages / documentation
- [ ] Homebrew / cargo-binstall packaging
- [ ] Performance optimization

**Duration**: ~2 weeks

---

## Appendix A: Example Workflows

### A.1 New Feature Across Multiple Repos

```bash
# Start fresh
cd my-workspace
harmonia sync

# Create feature branch in repos you'll touch
harmonia branch -c feature/auth --repos=shared-lib,service-a

# Make changes...
# (edit files in shared-lib and service-a)

# Check status
harmonia status
# Shows: shared-lib and service-a have changes

# Run tests
harmonia test --graph-order

# Bump versions
harmonia version bump patch

# Commit
harmonia commit -m "feat: add authentication support"

# Push
harmonia push -u

# See the plan
harmonia plan

# Create MRs
harmonia mr create -t "feat: add authentication support"
# Creates linked MRs in dependency order, tracking issue

# Monitor
harmonia mr status

# After approvals, merge in order
harmonia mr merge
```

### A.2 Updating a Shared Library

```bash
# See what depends on shared-lib
harmonia graph dependents shared-lib

# Create branch
harmonia branch -c fix/performance --repos=shared-lib

# Make changes, test
harmonia test shared-lib

# Check if bump would break dependents
harmonia version bump minor --dry-run

# Bump and update dependents
harmonia version bump minor --cascade

# This creates changes in dependent repos too
harmonia status
# Shows: shared-lib (bumped), service-a (dep updated), service-b (dep updated)

# Commit all
harmonia commit -m "perf: improve shared-lib performance"

# Create MRs (will include all affected repos)
harmonia mr create
```

---

## Appendix B: Comparison with Existing Tools

| Feature | Harmonia | meta | Lerna | Nx | Turborepo |
|---------|----------|------|-------|-----|-----------|
| Poly-repo | ✓ | ✓ | ✗ | ✗ | ✗ |
| Monorepo | ✗ | ✗ | ✓ | ✓ | ✓ |
| Language agnostic | ✓ | ✓ | ✗ (JS) | ✗ (JS) | ✗ (JS) |
| Dependency graph | ✓ | ✗ | ✓ | ✓ | ✓ |
| Coordinated MRs | ✓ | ✗ | ✗ | ✗ | ✗ |
| Version bumping | ✓ | ✗ | ✓ | ✗ | ✗ |
| Forge integration | ✓ | ✗ | ✗ | ✗ | ✗ |

---

## Appendix C: Glossary

| Term | Definition |
|------|------------|
| **Workspace** | A directory containing Harmonia configuration and cloned repos |
| **Repo** | A single git repository within the workspace |
| **Changeset** | A coordinated set of changes across repos, typically sharing a branch |
| **Forge** | A git hosting platform (GitLab, GitHub, etc.) |
| **Ecosystem** | A language/package ecosystem (Python/PyPI, Rust/crates.io, etc.) |
| **Internal dependency** | A dependency on another repo within the workspace |
| **Merge order** | The sequence in which MRs should be merged based on dependencies |
| **Tracking issue** | An issue that links all MRs in a changeset |
| **External repo** | A repo included in the dependency graph but excluded from changesets and default mutating operations |
| **Ignored repo** | A repo excluded from the dependency graph and all default operations |

---

*Harmonia: Bringing repositories into harmony*
