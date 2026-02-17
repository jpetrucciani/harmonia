# Core Workflows

## 1. Feature Across Multiple Repos

```bash
# sync and start feature branches
harmonia sync
# if you keep local work-in-progress changes:
harmonia sync --autostash
harmonia branch feature/auth --create --repos app --with-all-deps

# develop, then validate in graph order
harmonia test --changed --graph-order --fail-fast
harmonia lint --changed

# commit and push
harmonia add --repos core,app --all
harmonia commit --repos core,app --message "feat: auth flow"
harmonia push --repos core,app --set-upstream

# inspect plan and open MRs
harmonia plan
harmonia mr create --title "feat: auth flow"
```

If your workspace uses `[repos].<name>.depends_on`, graph-order commands and
planning honor those declarations in addition to manifest-parsed dependencies.

## 2. Single-Repo Hotfix

```bash
harmonia branch hotfix/critical --create --repos api
harmonia test api --fail-fast
harmonia add --repos api --all
harmonia commit --repos api --message "fix: critical bug"
harmonia push --repos api --set-upstream
harmonia mr create --title "fix: critical bug"
```

## 3. Hook-Driven Team Policy

Define workspace hooks once:

```toml
[hooks]
pre_commit = "harmonia test --changed --fail-fast"
pre_push = "harmonia lint --changed"
```

Repo-level opt-out for specific hooks:

```toml
[hooks]
disable_workspace_hooks = ["pre_push"]
```

## 4. Version and Internal Dependency Updates

```bash
harmonia version check
harmonia version bump patch --changed --dry-run
harmonia deps check
harmonia deps update --dry-run
```

Use `--dry-run` before applying bulk updates in active branches.
