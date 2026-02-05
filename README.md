# Harmonia

[![uses nix](https://img.shields.io/badge/uses-nix-%237EBAE4)](https://nixos.org/)
![rust](https://img.shields.io/badge/Rust-1.95%2B-orange.svg)

Poly-repo orchestrator for coordinating work across multiple repositories.

## What works today

Core workspace
- `harmonia init`, `clone`, `status`, `sync` (fetch + fast-forward only)
- `exec`, `run` (custom hooks), `each` (run per repo)

Git coordination
- `branch`, `checkout` (basic; no `--track`, `--with-deps`, or `--with-all-deps` yet)
- `add`, `commit`, `push`, `diff`
- Pre-commit and pre-push hooks (workspace + repo-level)

Dependency graph
- Build graph from repo configs + ecosystem plugins
- `graph show/deps/dependents/order/check`
- Formats: tree, flat, dot, json

Versioning and deps
- `version show/check/bump` (semver, calver, tinyinc)
- `deps show/check/update` and cascade dependency updates

## Known gaps

- `sync --rebase` and non fast-forward merges are not implemented
- `branch --track`, `--with-deps`, `--with-all-deps` not implemented
- Forge integration and MR features not implemented yet
- `harmonia test` and `harmonia lint` not implemented yet

## Configuration

Harmonia reads workspace config from `.harmonia/config.toml` and optional per-repo config from `.harmonia.toml` inside each repo.

### Workspace config (required)

Create `.harmonia/config.toml` at the workspace root:

```toml
[workspace]
name = "my-platform"
repos_dir = "repos"

[forge]
type = "gitlab"
# host = "gitlab.example.com"
# default_group = "platform-team"

[repos]
"shared-lib" = {}
"service-a" = { url = "git@gitlab.example.com:platform-team/service-a.git" }
"legacy-api" = { url = "git@gitlab.example.com:legacy/api.git", default_branch = "master" }

# external stays in the graph, but excluded from default mutating ops
"external-sdk" = { url = "git@github.com:vendor/sdk.git", external = true }

# ignored is excluded from graph and default ops
"scratch" = { url = "git@gitlab.example.com:platform-team/scratch.git", ignored = true }

[groups]
core = ["shared-lib", "service-a"]
# default = "core"

[defaults]
default_branch = "main"
clone_protocol = "ssh"
clone_depth = "full"
include_untracked = true

[hooks]
pre_commit = "harmonia test --changed --fail-fast"
pre_push = "harmonia lint --changed"

[versioning]
strategy = "semver"      # semver | calver | none
bump_mode = "semver"     # semver | calver | tinyinc
# calver_format = "YYYY.0M.MICRO"
# cascade_bumps = false
```

### Repo config (optional)

Create `.harmonia.toml` in a repo root to define versioning, deps, and hooks:

```toml
[package]
name = "shared-lib"
ecosystem = "python"  # python | rust | node | go | custom

[versioning]
file = "pyproject.toml"
path = "project.version"
strategy = "semver"
bump_mode = "semver"
# pattern = 'VERSION = "(\d+\.\d+\.\d+)"'

[dependencies]
file = "pyproject.toml"
path = "project.dependencies"
internal_pattern = "^(shared-|service-|legacy-)"
# internal_packages = ["shared-lib", "service-a-client"]

[hooks]
# disable_workspace_hooks = ["pre_commit"]
pre_commit = "uv run pytest -x"
pre_push = "uv run ruff check . && uv run mypy ."

[hooks.custom]
format = "uv run ruff format ."
```

## Quick start

```bash
# from a new workspace directory
harmonia init
# or initialize from existing config
harmonia init ./config-repo --directory ./my-workspace

# clone repos
harmonia clone --all

# sync
harmonia sync

# show graph
harmonia graph --format=tree

# bump versions (dry-run)
harmonia version bump patch --dry-run
```

## Environment variables

- `HARMONIA_WORKSPACE` override workspace root
- `HARMONIA_CONFIG` override config path
- `HARMONIA_REPOS_DIR` override repos dir
- `HARMONIA_PARALLEL` default parallelism
- `HARMONIA_LOG_LEVEL` logging verbosity
- `HARMONIA_NO_COLOR` disable color

## Notes

- Git operations use the git CLI for add/commit/push/diff.
- Sync currently fetches and fast-forwards only.
