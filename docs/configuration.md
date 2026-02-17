# Configuration

Harmonia reads configuration from:

- Workspace: `.harmonia/config.toml` (preferred) or `.harmonia.toml`
- Repo override (optional): `<repo>/.harmonia.toml`

## Workspace Config

```toml
[workspace]
name = "platform"
repos_dir = "repos"

[forge]
type = "github"
# host = "github.com"
# default_group = "platform-team"
# token = "" # prefer HARMONIA_FORGE_TOKEN

[repos]
"core" = { package_name = "core-pkg" }
"app" = { url = "file:///abs/path/to/app.git", depends_on = ["core"] }
"vendor-sdk" = { url = "file:///abs/path/to/vendor-sdk.git", external = true }
"scratch" = { url = "file:///abs/path/to/scratch.git", ignored = true }

[groups]
core = ["core", "app"]
default = "core"

[defaults]
default_branch = "main"
clone_protocol = "ssh" # ssh | https
clone_depth = "full"   # full | integer depth string
include_untracked = true

[hooks]
pre_commit = "harmonia test --changed --fail-fast"
pre_push = "harmonia lint --changed"
post_mr_create = "echo mr-created"

[hooks.custom]
fmt = "harmonia each -- cargo fmt"

[mr]
template = ".harmonia/templates/mr.md"
link_strategy = "all" # related | description | issue | all
create_tracking_issue = true
issue_template = ".harmonia/templates/issue.md"
add_trailers = false
labels = ["platform", "automation"]
require_tests = true
draft = false

[versioning]
strategy = "semver"   # semver | calver | none
bump_mode = "semver"  # semver | calver | tinyinc
# calver_format = "YYYY.0M.MICRO"
# cascade_bumps = true

[changesets]
enabled = true
dir = "changesets"
```

### Workspace Dependency Declarations

You can declare internal dependency edges directly at workspace level with
`[repos].<name>.depends_on`.

This affects graph order and merge planning, even when repos do not have
repo-level config files.

```toml
[repos]
"core" = { package_name = "core-pkg" }
"lib" = { depends_on = ["core"] }                # by repo key
"api" = { depends_on = ["core-pkg", "lib"] }     # by package name or repo key
```

Use these commands to verify the resolved order:

```bash
harmonia graph order
harmonia plan
```

## Repo Config

```toml
[package]
name = "core"
ecosystem = "rust" # rust | python | node | go | custom

[versioning]
file = "Cargo.toml"
path = "package.version"
strategy = "semver"
bump_mode = "semver"
# pattern = "VERSION = \"(\\d+\\.\\d+\\.\\d+)\""

[dependencies]
file = "Cargo.toml"
path = "dependencies"
internal_pattern = "^(core|app|service-)"
# internal_packages = ["core", "app"]

[hooks]
disable_workspace_hooks = ["pre_push"]
pre_commit = "cargo test"
pre_push = "cargo clippy --all-targets --all-features"

[hooks.custom]
fmt = "cargo fmt"

[ci]
required_checks = ["test", "lint"]
timeout_minutes = 30
```

Repo-level dependency parsing and workspace-level `depends_on` are combined.
Duplicate edges are de-duplicated automatically.

## Changeset Files

When `[changesets].enabled = true`, Harmonia reads `*.toml` files under the configured directory.

Example `.harmonia/changesets/cs-auth.toml`:

```toml
id = "cs-auth"
title = "feat: auth"
description = "introduce auth flow"
branch = "feature/auth"

[[repos]]
repo = "core"
summary = "shared auth primitives"

[[repos]]
repo = "app"
summary = "integrate auth flow"
```

## Environment Overrides

| Variable | Purpose |
|---|---|
| `HARMONIA_FORGE_TOKEN` | Forge token override |
| `HARMONIA_WORKSPACE` | Workspace root override |
| `HARMONIA_CONFIG` | Config path override |
| `HARMONIA_REPOS_DIR` | Repos directory override |
| `HARMONIA_PARALLEL` | Default parallel worker count |
| `HARMONIA_LOG_LEVEL` | Log verbosity |
| `HARMONIA_NO_COLOR` | Disable colored output |

## Validation Rules

Config loading fails early for invalid combinations, including:

- invalid `[defaults].clone_protocol`
- invalid `[mr].link_strategy`
- invalid changesets directory when changesets are enabled
- repo entries with both `external = true` and `ignored = true`
