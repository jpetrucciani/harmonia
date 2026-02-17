use std::collections::HashSet;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use anyhow::Context;
use gix::bstr::{BString, ByteSlice};
use gix::progress::Discard;
use gix::remote;
use gix::status::index_worktree::iter::Summary;

use crate::error::{HarmoniaError, Result};
use crate::git::status::StatusSummary;

pub struct OpenRepo {
    pub path: PathBuf,
    pub repo: gix::Repository,
}

#[derive(Debug, Clone, Copy)]
pub struct SyncOptions {
    pub fetch_only: bool,
    pub ff_only: bool,
    pub rebase: bool,
    pub autostash: bool,
    pub prune: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SyncOutcome {
    pub fast_forwarded: bool,
    pub rebased: bool,
    pub merged: bool,
    pub autostashed: bool,
    pub pruned: usize,
}

pub fn open_repo(path: &Path) -> Result<OpenRepo> {
    let mut repo = gix::open(path).map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    repo.committer_or_set_generic_fallback()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    Ok(OpenRepo {
        path: path.to_path_buf(),
        repo,
    })
}

pub fn clone_repo(url: &str, dest: &Path, depth: Option<u32>) -> Result<()> {
    let mut prepare =
        gix::prepare_clone(url, dest).map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;

    if let Some(depth) = depth {
        if let Some(depth) = NonZeroU32::new(depth) {
            prepare = prepare.with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(depth));
        }
    }

    let cancel = AtomicBool::new(false);
    let (mut checkout, _outcome) = prepare
        .fetch_then_checkout(Discard, &cancel)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;

    checkout
        .main_worktree(Discard, &cancel)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;

    Ok(())
}

pub fn sync_repo(repo: &gix::Repository, options: SyncOptions) -> Result<SyncOutcome> {
    let fetch = fetch_repo(repo, options.prune)?;
    if options.fetch_only {
        return Ok(SyncOutcome {
            fast_forwarded: false,
            rebased: false,
            merged: false,
            autostashed: false,
            pruned: fetch.pruned,
        });
    }

    if options.rebase {
        let rebase = rebase_repo(repo, fetch.remote_name.as_deref(), options.autostash)?;
        return Ok(SyncOutcome {
            fast_forwarded: false,
            rebased: rebase.rebased,
            merged: false,
            autostashed: rebase.autostashed,
            pruned: fetch.pruned,
        });
    }

    let update = update_after_fetch(
        repo,
        fetch.remote_name.as_deref(),
        options.ff_only,
        options.autostash,
    )?;
    Ok(SyncOutcome {
        fast_forwarded: matches!(update.update, SyncUpdate::FastForward),
        rebased: false,
        merged: matches!(update.update, SyncUpdate::Merged),
        autostashed: update.autostashed,
        pruned: fetch.pruned,
    })
}

pub fn repo_status(repo: &gix::Repository) -> Result<StatusSummary> {
    let platform = repo
        .status(Discard)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let mut summary = StatusSummary::default();

    for item in platform
        .into_iter(Vec::new())
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
    {
        let item = item.map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
        match item {
            gix::status::Item::TreeIndex(tree_index) => {
                let path = PathBuf::from(tree_index.location().to_str_lossy().to_string());
                summary.staged.push(path);
            }
            gix::status::Item::IndexWorktree(index_item) => {
                let path = PathBuf::from(index_item.rela_path().to_str_lossy().to_string());
                match index_item.summary() {
                    Some(Summary::Added) | Some(Summary::IntentToAdd) => {
                        summary.untracked.push(path);
                    }
                    Some(Summary::Conflict) => {
                        summary.conflicts.push(path);
                    }
                    _ => {
                        summary.modified.push(path);
                    }
                }
            }
        }
    }

    Ok(summary)
}

pub fn current_branch(repo: &gix::Repository) -> Result<String> {
    if let Some(work_dir) = repo.workdir() {
        let output = Command::new("git")
            .current_dir(work_dir)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .map_err(|err| HarmoniaError::Other(anyhow::anyhow!(format!("{}", err))))?;
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return Ok(name);
            }
        }
    }

    let head = repo
        .head()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    if matches!(head.kind, gix::head::Kind::Detached { .. }) {
        return Ok("(detached)".to_string());
    }

    Ok(head.name().shorten().to_string())
}

pub fn ensure_repo_dir(path: &Path) -> Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create repo dir {}", path.display()))
        .map_err(HarmoniaError::Other)
}

pub fn branch_exists(repo: &gix::Repository, name: &str) -> Result<bool> {
    let full_name = format!("refs/heads/{name}");
    Ok(repo
        .try_find_reference(full_name.as_str())
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .is_some())
}

pub fn create_branch(repo: &gix::Repository, name: &str, force: bool) -> Result<()> {
    let target = repo
        .head_id()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();
    let full_name = format!("refs/heads/{name}");
    let constraint = if force {
        gix::refs::transaction::PreviousValue::Any
    } else {
        gix::refs::transaction::PreviousValue::MustNotExist
    };
    repo.reference(
        full_name,
        target,
        constraint,
        format!("branch: created {name}"),
    )
    .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    Ok(())
}

pub fn checkout_branch(repo: &gix::Repository, name: &str) -> Result<()> {
    let full_name = format!("refs/heads/{name}");
    let mut branch_ref = repo
        .find_reference(full_name.as_str())
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let target = branch_ref
        .peel_to_id()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();

    let status = repo_status(repo)?;
    if !status.is_clean() {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "working tree has uncommitted changes"
        )));
    }

    checkout_tree(repo, target)?;
    set_head_symbolic(repo, &full_name)?;
    Ok(())
}

pub fn create_and_checkout_branch(repo: &gix::Repository, name: &str) -> Result<()> {
    run_git_command(
        repo,
        &["checkout", "-b", name],
        "create and checkout branch",
    )
}

pub fn set_branch_upstream(
    repo: &gix::Repository,
    local_branch: &str,
    upstream: &str,
) -> Result<()> {
    run_git_command(
        repo,
        &["branch", "--set-upstream-to", upstream, local_branch],
        "set branch upstream",
    )
}

struct FetchOutcome {
    remote_name: Option<String>,
    pruned: usize,
}

fn fetch_repo(repo: &gix::Repository, prune: bool) -> Result<FetchOutcome> {
    let remote = repo
        .find_fetch_remote(None)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let remote_name = remote
        .name()
        .and_then(|name| name.as_symbol())
        .map(|name| name.to_string());
    let connection = remote
        .connect(remote::Direction::Fetch)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let prepare = connection
        .prepare_fetch(Discard, gix::remote::ref_map::Options::default())
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let cancel = AtomicBool::new(false);
    let outcome = prepare
        .receive(Discard, &cancel)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;

    let pruned = if prune {
        if let Some(symbolic) = remote_name.as_deref() {
            prune_remote_refs(repo, symbolic, &outcome.ref_map)?
        } else {
            0
        }
    } else {
        0
    };

    Ok(FetchOutcome {
        remote_name,
        pruned,
    })
}

enum SyncUpdate {
    None,
    FastForward,
    Merged,
}

struct SyncUpdateOutcome {
    update: SyncUpdate,
    autostashed: bool,
}

fn update_after_fetch(
    repo: &gix::Repository,
    remote_name: Option<&str>,
    ff_only: bool,
    autostash: bool,
) -> Result<SyncUpdateOutcome> {
    let tracking = tracking_ref_name_for_head(repo, remote_name)?.ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!("no upstream tracking branch configured"))
    })?;
    let tracking_name = tracking.to_str_lossy().to_string();

    let mut head_ref = repo
        .head_ref()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .ok_or_else(|| {
            HarmoniaError::Other(anyhow::anyhow!(
                "cannot fast-forward with detached or unborn HEAD"
            ))
        })?;

    let local_id = head_ref
        .peel_to_id()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();

    let mut tracking_ref = repo
        .find_reference(tracking.as_bstr())
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let remote_id = tracking_ref
        .peel_to_id()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();

    if local_id == remote_id {
        return Ok(SyncUpdateOutcome {
            update: SyncUpdate::None,
            autostashed: false,
        });
    }

    let merge_base = repo
        .merge_base(local_id, remote_id)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();
    if merge_base == remote_id {
        return Ok(SyncUpdateOutcome {
            update: SyncUpdate::None,
            autostashed: false,
        });
    }

    if merge_base == local_id {
        let (_, autostashed) = with_optional_autostash(repo, autostash, || {
            checkout_tree(repo, remote_id)?;
            head_ref
                .set_target_id(remote_id, "fast-forward")
                .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
            Ok(())
        })?;
        return Ok(SyncUpdateOutcome {
            update: SyncUpdate::FastForward,
            autostashed,
        });
    }

    if ff_only {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "fast-forward is not possible"
        )));
    }

    let (_, autostashed) = with_optional_autostash(repo, autostash, || {
        run_git_command(
            repo,
            &["merge", "--no-edit", tracking_name.as_str()],
            "merge tracking branch",
        )
    })?;
    Ok(SyncUpdateOutcome {
        update: SyncUpdate::Merged,
        autostashed,
    })
}

struct RebaseOutcome {
    rebased: bool,
    autostashed: bool,
}

fn rebase_repo(
    repo: &gix::Repository,
    remote_name: Option<&str>,
    autostash: bool,
) -> Result<RebaseOutcome> {
    let tracking = tracking_ref_name_for_head(repo, remote_name)?.ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!("no upstream tracking branch configured"))
    })?;
    let tracking_name = tracking.to_str_lossy().to_string();

    let local_id = repo
        .head_id()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();
    let mut tracking_ref = repo
        .find_reference(tracking.as_bstr())
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let remote_id = tracking_ref
        .peel_to_id()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();

    if local_id == remote_id {
        return Ok(RebaseOutcome {
            rebased: false,
            autostashed: false,
        });
    }

    let merge_base = repo
        .merge_base(local_id, remote_id)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();
    if merge_base == remote_id {
        return Ok(RebaseOutcome {
            rebased: false,
            autostashed: false,
        });
    }

    let (_, autostashed) = with_optional_autostash(repo, autostash, || {
        run_git_command(
            repo,
            &["rebase", tracking_name.as_str()],
            "rebase onto tracking branch",
        )
    })?;

    let updated = repo
        .head_id()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();
    Ok(RebaseOutcome {
        rebased: updated != local_id,
        autostashed,
    })
}

fn with_optional_autostash<T, F>(
    repo: &gix::Repository,
    autostash: bool,
    operation: F,
) -> Result<(T, bool)>
where
    F: FnOnce() -> Result<T>,
{
    let status = repo_status(repo)?;
    if status.is_clean() {
        return operation().map(|value| (value, false));
    }
    if !autostash {
        return Err(HarmoniaError::Other(anyhow::anyhow!(
            "working tree has uncommitted changes, use --autostash to stash and re-apply local changes automatically or use --fetch-only"
        )));
    }

    let stashed = push_autostash(repo)?;
    let result = operation();
    match result {
        Ok(value) => {
            if stashed {
                pop_autostash(repo).map_err(|err| {
                    HarmoniaError::Other(anyhow::anyhow!(format!(
                        "sync completed but failed to re-apply stashed changes: {}. recover manually with 'git stash list' and 'git stash pop'",
                        err
                    )))
                })?;
            }
            Ok((value, stashed))
        }
        Err(err) => {
            if stashed {
                return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
                    "{}. local changes were stashed and are still available in 'git stash list'",
                    err
                ))));
            }
            Err(err)
        }
    }
}

fn push_autostash(repo: &gix::Repository) -> Result<bool> {
    let before = stash_entry_count(repo)?;
    run_git_command(
        repo,
        &[
            "stash",
            "push",
            "--include-untracked",
            "--message",
            "harmonia-sync",
        ],
        "stash local changes before sync",
    )?;
    let after = stash_entry_count(repo)?;
    Ok(after > before)
}

fn pop_autostash(repo: &gix::Repository) -> Result<()> {
    run_git_command(
        repo,
        &["stash", "pop", "--index"],
        "re-apply stashed changes after sync",
    )
}

fn stash_entry_count(repo: &gix::Repository) -> Result<usize> {
    let output = run_git_command_output(
        repo,
        &["stash", "list", "--format=%H"],
        "list local stashes",
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count())
}

fn run_git_command(repo: &gix::Repository, args: &[&str], context: &str) -> Result<()> {
    let workdir = repo.workdir().ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(
            "operation requires a worktree but repository is bare"
        ))
    })?;
    let status = Command::new("git")
        .args(args)
        .current_dir(workdir)
        .status()
        .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    if status.success() {
        return Ok(());
    }

    Err(HarmoniaError::Other(anyhow::anyhow!(format!(
        "git {} failed",
        context
    ))))
}

fn run_git_command_output(repo: &gix::Repository, args: &[&str], context: &str) -> Result<String> {
    let workdir = repo.workdir().ok_or_else(|| {
        HarmoniaError::Other(anyhow::anyhow!(
            "operation requires a worktree but repository is bare"
        ))
    })?;
    let output = Command::new("git")
        .args(args)
        .current_dir(workdir)
        .output()
        .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    Err(HarmoniaError::Other(anyhow::anyhow!(format!(
        "git {} failed",
        context
    ))))
}

fn tracking_ref_name_for_head(
    repo: &gix::Repository,
    remote_name: Option<&str>,
) -> Result<Option<BString>> {
    let head = repo
        .head()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let head_ref = match head.referent_name() {
        Some(name) => name,
        None => return Ok(None),
    };

    if let Some(tracking) = repo
        .branch_remote_tracking_ref_name(head_ref, remote::Direction::Fetch)
        .transpose()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
    {
        return Ok(Some(tracking.into_owned().into()));
    }

    let remote = match remote_name {
        Some(remote) => remote,
        None => return Ok(None),
    };
    let short = head_ref.shorten().to_str_lossy();
    Ok(Some(BString::from(format!(
        "refs/remotes/{remote}/{short}"
    ))))
}

fn checkout_tree(repo: &gix::Repository, target: gix::hash::ObjectId) -> Result<()> {
    let workdir = match repo.workdir() {
        Some(path) => path,
        None => return Ok(()),
    };

    let commit = repo
        .find_commit(target)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let tree_id = commit
        .tree_id()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?
        .detach();

    let mut index = repo
        .index_from_tree(&tree_id)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;

    let mut opts = repo
        .checkout_options(gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping)
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    opts.destination_is_initially_empty = false;

    let files = Discard;
    let bytes = Discard;
    let cancel = AtomicBool::new(false);
    gix::worktree::state::checkout(
        &mut index,
        workdir,
        repo.objects
            .clone()
            .into_arc()
            .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?,
        &files,
        &bytes,
        &cancel,
        opts,
    )
    .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    index
        .write(Default::default())
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;

    Ok(())
}

fn set_head_symbolic(repo: &gix::Repository, target: &str) -> Result<()> {
    let full: gix::refs::FullName = target
        .try_into()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    repo.edit_reference(gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange::default(),
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Symbolic(full),
        },
        name: "HEAD".try_into().expect("HEAD is valid"),
        deref: false,
    })
    .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    Ok(())
}

fn prune_remote_refs(
    repo: &gix::Repository,
    remote_name: &str,
    ref_map: &gix::remote::fetch::RefMap,
) -> Result<usize> {
    let prefix = format!("refs/remotes/{remote_name}/");
    let mut expected = HashSet::new();
    for mapping in &ref_map.mappings {
        if let Some(local) = mapping.local.as_ref() {
            if local.as_bstr().starts_with(prefix.as_bytes()) {
                expected.insert(local.clone());
            }
        }
    }

    let head_name = BString::from(format!("refs/remotes/{remote_name}/HEAD"));
    let mut pruned = 0;
    let refs = repo
        .references()
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    let iter = refs
        .prefixed(prefix.as_str())
        .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
    for reference in iter {
        let reference =
            reference.map_err(|err| HarmoniaError::Git(anyhow::anyhow!(err.to_string())))?;
        let name = reference.name().as_bstr().to_owned();
        if name == head_name {
            continue;
        }
        if !expected.contains(&name) {
            reference
                .delete()
                .map_err(|err| HarmoniaError::Git(anyhow::Error::new(err)))?;
            pruned += 1;
        }
    }

    Ok(pruned)
}
