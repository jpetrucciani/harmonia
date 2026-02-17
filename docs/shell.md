# Shell and Completions

## `harmonia shell`

`harmonia shell` prepares a workspace-aware shell environment.

Environment behavior:

- sets `HARMONIA_WORKSPACE`
- prepends repo `bin` directories to `PATH`
- prepends repo `src` directories to `PYTHONPATH`

Interactive shell:

```bash
harmonia shell
```

Scope to selected repos:

```bash
harmonia shell --repos core,app
```

Run one command with the computed environment:

```bash
harmonia shell --repos core --command 'printf %s "$HARMONIA_WORKSPACE"'
```

In non-interactive mode, `harmonia shell` prints `export ...` lines you can eval in scripts.

## Completions

Generate one shell completion script:

```bash
harmonia completion bash > ./harmonia.bash
harmonia completion zsh > ~/.zfunc/_harmonia
harmonia completion fish > ~/.config/fish/completions/harmonia.fish
```

Generate all supported shell completions into `docs/completions`:

```bash
mkdir -p docs/completions
for shell in bash zsh fish elvish powershell; do
  harmonia completion "$shell" > "docs/completions/harmonia.$shell"
done
```

Supported shells: `bash`, `zsh`, `fish`, `elvish`, `powershell`.
