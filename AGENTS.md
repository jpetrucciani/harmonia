# Repository Guidelines

## Project Structure & Module Organization

- `spec.md` is the primary design document and source of truth for scope, commands, and architecture.
- `default.nix` defines the Nix dev environment and build tooling.
- `.envrc` and `.direnv/` support direnv based shell setup.
- No source code is committed yet. When implementation begins, follow the module layout described in `spec.md` under “Architecture”.

## Build, Test, and Development Commands

- `nix-shell` from the repo root to enter the pinned dev environment defined in `default.nix`.
- `build_static` to build a static Linux binary via `cargo-zigbuild` once Rust sources and `Cargo.toml` exist.
- There are no build or test targets committed yet, add commands here as tooling lands.

## Coding Style & Naming Conventions

- No codebase conventions exist yet. When Rust code is added, format with `cargo fmt` and lint with `cargo clippy --all --benches --tests --examples --all-features`.
- Follow module and file naming patterns in `spec.md` to keep CLI, core, graph, forge, and ecosystem code separated.

## Testing Guidelines

- No test framework or tests are present yet.
- When tests are introduced, prefer real unit or integration tests over mocks, and document the runner command in this section.

## Commit & Pull Request Guidelines

- The repository has no commits yet, so no commit message convention is established.
- Keep changes focused, update `spec.md` when behavior changes, and include command examples if you add tooling.
- For PRs, include a short summary, relevant command output, and any new configuration files.

## Security & Configuration Tips

- Do not commit forge tokens or secrets. The design expects environment variables like `HARMONIA_FORGE_TOKEN` for authentication.
- Prefer configuration in `.harmonia/config.toml` and `.harmonia.toml` as described in `spec.md` once the CLI is implemented.
