# Troubleshooting

## `workspace not found` or `config file not found`

Cause:

- You are outside a Harmonia workspace
- `.harmonia/config.toml` and `.harmonia.toml` are both missing
- `--workspace` or `--config` points to the wrong path

Fix:

```bash
harmonia --workspace /path/to/workspace status
```

Or set overrides:

```bash
export HARMONIA_WORKSPACE=/path/to/workspace
export HARMONIA_CONFIG=/path/to/workspace/.harmonia/config.toml
# or:
# export HARMONIA_CONFIG=/path/to/workspace/.harmonia.toml
```

## `unknown repo <name>` or `unknown group <name>`

Cause:

- The repo/group is not defined in workspace config

Fix:

```bash
harmonia repo list
harmonia config show
```

Verify `[repos]` and `[groups]` entries in workspace config.

## `sync` fails with uncommitted local changes

Cause:

- `harmonia sync` updates branches and requires a clean worktree by default

Fix:

```bash
# preserve local changes while syncing
harmonia sync --autostash

# or fetch-only if you do not want branch updates yet
harmonia sync --fetch-only
```

## Forge token errors for MR operations

Cause:

- No forge token configured

Fix:

```bash
export HARMONIA_FORGE_TOKEN=<token>
```

This env var takes precedence over `[forge].token`.

## MR merge waits forever or times out

Cause:

- Required checks are pending/missing/failed
- Timeout too low for current CI workload

Fix:

```bash
harmonia mr status --wait --timeout 60
```

Review repo-level CI config:

```toml
[ci]
required_checks = ["test", "lint"]
timeout_minutes = 60
```

## Hook command failed unexpectedly

Cause:

- Hook commands are split by whitespace and executed directly, not via a shell parser
- shell operators like `>>`, pipes, and chained control operators are not interpreted

Fix:

- Use direct executables and arguments
- Wrap complex shell behavior in a script file and call that script from the hook

## Debug selection and graph behavior

Useful checks:

```bash
harmonia status --json
harmonia graph show --format json
harmonia graph check --json
harmonia plan --json
```
