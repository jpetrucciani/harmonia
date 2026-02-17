# Plan and MR Workflow

## Planning

`harmonia plan` summarizes:

- changed repositories
- repo status and diff stats
- dependency merge order (from manifest-parsed deps and workspace `depends_on`)
- constraint analysis
- recommendations

Use JSON for tooling:

```bash
harmonia plan --json
```

Scope controls:

```bash
harmonia plan --include core,app
harmonia plan --exclude scratch
```

If you want to declare ordering without per-repo config, set
`[repos].<name>.depends_on` in workspace config and re-run `harmonia plan`.

## Changeset-Driven Planning

When changesets are enabled in workspace config, Harmonia can select an active changeset by branch and include its repo summaries in plan output.

```toml
[changesets]
enabled = true
dir = "changesets"
```

## MR Lifecycle Commands

```bash
harmonia mr create --title "feat: auth flow"
harmonia mr status --wait --timeout 30
harmonia mr update --labels platform,backend
harmonia mr merge --yes
harmonia mr close --yes
```

## Useful MR Config Fields

```toml
[mr]
template = ".harmonia/templates/mr.md"
link_strategy = "all" # related | description | issue | all
create_tracking_issue = true
issue_template = ".harmonia/templates/issue.md"
labels = ["platform"]
require_tests = true
draft = false
```

## CI Gating

Per-repo CI settings are used by MR status/merge orchestration:

```toml
[ci]
required_checks = ["test", "lint"]
timeout_minutes = 30
```

If required checks are missing, pending, or failed, merge orchestration blocks accordingly.

## Current Caveat

`mr.add_trailers` is currently not mutating commits automatically. It is informational/manual for now.
