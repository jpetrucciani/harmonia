# Getting Started

## Prerequisites

- `git` available in `PATH`
- Nix (`nix-shell`) for the pinned dev environment (required for this repository)
- Rust toolchain for source builds (`cargo`)

## Install

### Local source install

```bash
nix-shell
cargo install --path .
```

### Development shell
```bash
nix-shell
```

`nix-shell` is used for all development tasks.

## Bootstrap a Workspace

```bash
mkdir -p .harmonia
cp config.example.toml .harmonia/config.toml
# or use a single-file workspace config:
# cp config.example.toml .harmonia.toml
```

Then edit `.harmonia/config.toml`:

- Set your repo names under `[repos]`
- Add explicit `url` values or configure `[forge].default_group` and `[defaults].clone_protocol`
- Optionally set default `[groups]`
- Optionally declare repo-level dependency order with `[repos].<name>.depends_on`

## First Commands

```bash
harmonia clone --all
harmonia refresh
harmonia status --long
harmonia graph show --format=tree
```

## First Change Flow

```bash
# implement changes first, then:
harmonia submit
# optional message override:
harmonia submit -m "feat: example"

# after merge, reset and update everything:
harmonia refresh
```

`submit` defaults commit message to `updates` when `-m/--message` is not provided.

Manual flow (if you want explicit per-step control):

```bash
harmonia branch feature/example --create --repos app --with-all-deps
harmonia test --changed --graph-order --fail-fast
harmonia lint --changed
harmonia plan
harmonia mr create --title "feat: example"
```

## Smoke Check

```bash
cargo test --all --tests --lib
```

## Next

- Configuration details: `/configuration`
- Command reference: `/cli/`
- Merge workflow: `/plan-and-mr`
