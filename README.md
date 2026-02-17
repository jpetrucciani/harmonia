# harmonia

[![uses nix](https://img.shields.io/badge/uses-nix-%237EBAE4)](https://nixos.org/)
![rust](https://img.shields.io/badge/Rust-1.95%2B-orange.svg)

Poly-repo orchestrator for coordinating changes across many repositories with dependency awareness.

## Why Harmonia

- Work in one workspace with many repos and shared config.
- See dependency order before you cut MRs.
- Run test/lint/exec commands across selected repos.
- Keep version and internal dependency updates consistent.
- Drive merge orchestration with CI-aware MR workflows.

## Status

The core workflow is implemented and tested:

- Workspace/repo management: `init`, `clone`, `sync`, `status`, `config`, `repo`, `edit`, `clean`
- Multi-repo execution: `exec`, `run`, `each`, `test`, `lint`
- Git coordination: `branch`, `checkout`, `add`, `commit`, `push`, `diff`
- Dependency graph: `graph show|deps|dependents|order|check`
- Version/deps: `version show|check|bump`, `deps show|check|update`
- Planning and MR workflow: `plan`, `mr create|status|update|merge|close`
- Shell/docs utilities: `shell`, `completion`

Current known limitation:

- `mr.add_trailers` is not automated yet. Trailer handling is manual/informational.
- `forge` support is available for GitHub and GitLab. Bitbucket and Gitea are intentionally not implemented yet.

## Install

### Nix-first installation

Nix is required for this repository.

Start by entering the pinned shell first:

```bash
nix-shell
```

Then install from source:

```bash
cargo install --path .
```

## Quick Start

1. Create a workspace config:

```bash
mkdir -p .harmonia
cp config.example.toml .harmonia/config.toml
# or create a single-file workspace config at .harmonia.toml
```

2. Edit repos in `.harmonia/config.toml`.
3. Clone and inspect:

```bash
harmonia clone --all
harmonia status --long
harmonia graph show --format=tree
```

4. Run checks and plan:

```bash
harmonia test --changed --graph-order
harmonia lint --changed
harmonia plan
```

5. Create MRs:

```bash
harmonia mr create --title "feat: my change"
```

## Day-to-Day Flow

```bash
# sync local state
harmonia sync
# if you keep local changes around, use:
harmonia sync --autostash

# branch only repos you need plus dependency context
harmonia branch feature/auth --create --repos app --with-all-deps

# implement changes, then validate in dependency order
harmonia test --changed --graph-order --fail-fast
harmonia lint --changed

# stage and commit selected repos
harmonia add --repos core,app --all
harmonia commit --repos core,app --message "feat: auth flow"
harmonia push --repos core,app --set-upstream

# inspect merge order and constraints
harmonia plan

# create or update MRs
harmonia mr create --title "feat: auth flow"
harmonia mr status --wait --timeout 30
```

## Configuration

Harmonia reads:

- Workspace config: `.harmonia/config.toml` (preferred) or `.harmonia.toml`
- Optional repo config: `<repo>/.harmonia.toml`

Minimal workspace config shape:

```toml
[workspace]
name = "platform"
repos_dir = "repos"

[forge]
type = "github"

[repos]
"core" = { package_name = "core-pkg" }
"app" = { url = "file:///abs/path/to/app.git", depends_on = ["core"] }

[groups]
core = ["core", "app"]
default = "core"

[defaults]
default_branch = "main"
clone_protocol = "ssh"
clone_depth = "full"
include_untracked = true

[hooks]
pre_commit = "harmonia test --changed --fail-fast"
pre_push = "harmonia lint --changed"
```

Workspace-level dependency edges can be declared directly in `[repos].<name>.depends_on`.
This controls graph order and merge planning without requiring per-repo config files.

Example:

```toml
[repos]
"core" = { package_name = "core-pkg" }
"lib" = { depends_on = ["core"] }          # by repo key
"api" = { depends_on = ["core-pkg", "lib"] } # by package name or repo key
```

Inspect the resulting order with:

```bash
harmonia graph order
harmonia plan
```

`sync` default behavior:

- fetches upstream for each selected repo
- fast-forwards when possible
- merges with `--no-edit` when histories diverge
- requires a clean working tree for branch updates by default

Use `harmonia sync --autostash` to stash local changes before update and re-apply them after.
Use `harmonia sync --fetch-only` to only fetch and not update local branches.

See `config.example.toml` and `docs/configuration.md` for full details.

## Documentation

Primary docs:

- `docs/index.md`
- `docs/getting-started.md`
- `docs/configuration.md`
- `docs/cli/index.md`
- `docs/workflows.md`
- `docs/plan-and-mr.md`
- `docs/shell.md`
- `docs/troubleshooting.md`
- `docs/release.md`

Generated reference assets:

- CLI help snapshots: `docs/cli/harmonia-help.txt` and `docs/cli/harmonia-*-help.txt`
- Manual page: `docs/man/harmonia.1`
- Completions: `docs/completions/`

## Docs Tooling

VitePress site scripts:

```bash
cd docs
npm install
npm run docs:dev
```

Generate completions:

```bash
generate_completions
# or
generate_completions ./docs/completions
```

Generate CLI help snapshots + man page:

```bash
generate_docs
# or
generate_docs ./docs
```

Preview the man page:

```bash
man -l docs/man/harmonia.1
```

## Local Fixture Smoke Flow

```bash
fixture_workspace --force /tmp/harmonia-local-fixture
smoke_fixture /tmp/harmonia-local-fixture/workspace
```

## Environment Variables

- `HARMONIA_FORGE_TOKEN` forge API token, overrides config token
- `HARMONIA_WORKSPACE` workspace root override
- `HARMONIA_CONFIG` config path override
- `HARMONIA_REPOS_DIR` repos directory override
- `HARMONIA_PARALLEL` default parallelism override
- `HARMONIA_LOG_LEVEL` logging verbosity
- `HARMONIA_NO_COLOR` disable color output

## Development

Rust checks:

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo test --all --tests --lib
```

Docs (VitePress) scripts are in `docs/package.json`.
