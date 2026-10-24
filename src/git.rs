//! Everything that touches the repository lives here. Functions open the
//! repository themselves, so they can run on worker threads without sharing
//! `git2::Repository` (which is not `Sync`).

use std::{fs, path::Path};

use anyhow::{bail, Context};
use chrono::{DateTime, Local};
use git2::{
    build::CheckoutBuilder, BranchType, Diff, DiffFormat, DiffOptions, IndexAddOption, Oid,
    Repository, Sort, Status, StatusOptions,
};

pub type Result<T> = anyhow::Result<T>;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    /// `true` if the change is in the index, `false` if it is only in the worktree.
    pub staged: bool,
    /// A/M/D/R/T for changes, `?` for untracked files.
    pub code: char,
}

#[derive(Clone, Debug)]
pub struct CommitRow {
    pub id: String,
    pub full: String,
    pub author: String,
    pub date: String,
    pub summary: String,
}

#[derive(Clone, Debug)]
pub struct BranchRow {
    pub name: String,
    pub is_head: bool,
    pub tip: String,
}

#[derive(Clone, Debug)]
pub struct StashRow {
    pub index: usize,
    pub oid: String,
    pub message: String,
}

#[derive(Default)]
pub struct Snapshot {
    pub branch: String,
    pub files: Vec<FileEntry>,
    pub conflicts: Vec<String>,
    pub commits: Vec<CommitRow>,
    pub branches: Vec<BranchRow>,
    pub stashes: Vec<StashRow>,
}

#[derive(Clone, Copy, Debug)]
pub enum Side {
    Ours,
    Theirs,
}

/// Something whose text is computed off the UI thread.
#[derive(Clone, Debug)]
pub enum Preview {
    Diff { path: String, staged: bool },
    Commit(String),
    Conflict(String),
}

fn open(workdir: &Path) -> Result<Repository> {
    Repository::open(workdir).context("could not open repository")
}

fn short(id: &str) -> &str {
    &id[..id.len().min(7)]
}

fn format_time(secs: i64) -> String {
    DateTime::from_timestamp(secs, 0)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn index_code(s: Status) -> Option<char> {
    if s.contains(Status::INDEX_NEW) {
        Some('A')
    } else if s.contains(Status::INDEX_MODIFIED) {
        Some('M')
    } else if s.contains(Status::INDEX_DELETED) {
        Some('D')
    } else if s.contains(Status::INDEX_RENAMED) {
        Some('R')
    } else if s.contains(Status::INDEX_TYPECHANGE) {
        Some('T')
    } else {
        None
    }
}

fn worktree_code(s: Status) -> Option<char> {
    if s.contains(Status::WT_NEW) {
        Some('?')
    } else if s.contains(Status::WT_MODIFIED) {
        Some('M')
    } else if s.contains(Status::WT_DELETED) {
        Some('D')
    } else if s.contains(Status::WT_RENAMED) {
        Some('R')
    } else if s.contains(Status::WT_TYPECHANGE) {
        Some('T')
    } else {
        None
    }
}

/// Gathers everything the UI shows in one pass.
pub fn load_snapshot(workdir: &Path) -> Result<Snapshot> {
    let mut repo = open(workdir)?;
    let branch = match repo.head() {
        Ok(head) => head.shorthand().unwrap_or("detached").to_string(),
        Err(_) => "no commits yet".to_string(),
    };
    let mut snap = Snapshot {
        branch,
        ..Default::default()
    };

    // Status: a single file can appear twice (staged and unstaged), and
    // conflicted files only show up in the conflicts list.
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    {
        let statuses = repo.statuses(Some(&mut opts))?;
        for entry in statuses.iter() {
            let Some(path) = entry.path() else { continue };
            let status = entry.status();
            if status.contains(Status::CONFLICTED) {
                snap.conflicts.push(path.to_string());
                continue;
            }
            if let Some(code) = index_code(status) {
                snap.files.push(FileEntry {
                    path: path.to_string(),
                    staged: true,
                    code,
                });
            }
            if let Some(code) = worktree_code(status) {
                snap.files.push(FileEntry {
                    path: path.to_string(),
                    staged: false,
                    code,
                });
            }
        }
    }

    if repo.head().is_ok() {
        let mut walk = repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(Sort::TIME)?;
        for oid in walk.take(300) {
            let commit = repo.find_commit(oid?)?;
            let full = commit.id().to_string();
            snap.commits.push(CommitRow {
                id: short(&full).to_string(),
                full,
                author: commit.author().name().unwrap_or("?").to_string(),
                date: format_time(commit.time().seconds()),
                summary: commit.summary().unwrap_or("").to_string(),
            });
        }
    }

    for item in repo.branches(Some(BranchType::Local))? {
        let (branch, _) = item?;
        let name = branch.name()?.unwrap_or("?").to_string();
        let tip = match branch.get().peel_to_commit() {
            Ok(c) => format!(
                "{} {}",
                short(&c.id().to_string()),
                c.summary().unwrap_or("")
            ),
            Err(_) => String::new(),
        };
        snap.branches.push(BranchRow {
            name,
            is_head: branch.is_head(),
            tip,
        });
    }

    repo.stash_foreach(|index, message, oid| {
        snap.stashes.push(StashRow {
            index,
            oid: oid.to_string(),
            message: message.to_string(),
        });
        true
    })?;

    Ok(snap)
}

/// Renders a diff as patch text, prefixing each content line with its origin
/// so the UI can colour it.
fn patch_text(diff: &Diff<'_>) -> Result<String> {
    let mut out = String::new();
    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        let text = String::from_utf8_lossy(line.content());
        match line.origin() {
            origin @ ('+' | '-' | ' ') => {
                out.push(origin);
                out.push_str(&text);
            }
            _ => out.push_str(&text),
        }
        true
    })?;
    Ok(out)
}

/// Text for the preview pane. Runs on a worker thread.
pub fn preview(workdir: &Path, spec: &Preview) -> Result<String> {
    match spec {
        Preview::Diff { path, staged } => {
            let repo = open(workdir)?;
            let mut opts = DiffOptions::new();
            opts.pathspec(path)
                .include_untracked(true)
                .show_untracked_content(true)
                .recurse_untracked_dirs(true);
            let index = repo.index()?;
            let diff = if *staged {
                let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
                repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut opts))?
            } else {
                repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?
            };
            let text = patch_text(&diff)?;
            Ok(if text.is_empty() {
                "no textual changes".into()
            } else {
                text
            })
        }
        Preview::Commit(oid) => {
            let repo = open(workdir)?;
            let commit = repo.find_commit(Oid::from_str(oid)?)?;
            let tree = commit.tree()?;
            let parent_tree = if commit.parent_count() > 0 {
                Some(commit.parent(0)?.tree()?)
            } else {
                None
            };
            let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
            let header = format!(
                "commit {}\nAuthor: {}\n\n    {}\n\n",
                commit.id(),
                commit.author().name().unwrap_or("?"),
                commit.summary().unwrap_or("")
            );
            Ok(header + &patch_text(&diff)?)
        }
        Preview::Conflict(path) => {
            let full = workdir.join(path);
            Ok(fs::read_to_string(&full).unwrap_or_else(|e| format!("cannot read {path}: {e}")))
        }
    }
}

// ---------------------------------------------------------------------------
// Mutations. Each returns a one-line human message on success.
// ---------------------------------------------------------------------------

/// Stages a path. If the file is gone from the worktree, stages the deletion.
pub fn stage(workdir: &Path, path: &str) -> Result<String> {
    let repo = open(workdir)?;
    let mut index = repo.index()?;
    if workdir.join(path).exists() {
        index.add_path(Path::new(path))?;
    } else {
        index.remove_path(Path::new(path))?;
    }
    index.write()?;
    Ok(format!("staged {path}"))
}

/// Resets one path's index entry back to HEAD.
pub fn unstage(workdir: &Path, path: &str) -> Result<String> {
    let repo = open(workdir)?;
    match repo.head().and_then(|h| h.peel_to_commit()) {
        Ok(head) => repo.reset_default(Some(head.as_object()), [path])?,
        Err(_) => {
            // No commits yet: there is no HEAD to reset to.
            let mut index = repo.index()?;
            index.remove_path(Path::new(path))?;
            index.write()?;
        }
    }
    Ok(format!("unstaged {path}"))
}

pub fn stage_all(workdir: &Path) -> Result<String> {
    let repo = open(workdir)?;
    let mut index = repo.index()?;
    index.add_all(["*"], IndexAddOption::DEFAULT, None)?;
    index.update_all(["*"], None)?;
    index.write()?;
    Ok("staged all changes".into())
}

pub fn mark_resolved(workdir: &Path, path: &str) -> Result<String> {
    stage(workdir, path)?;
    Ok(format!("marked {path} as resolved"))
}

pub fn commit(workdir: &Path, message: &str) -> Result<String> {
    let repo = open(workdir)?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        bail!("resolve conflicts before committing");
    }
    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    if let Some(p) = &parent {
        if p.tree_id() == tree_oid {
            bail!("nothing staged to commit");
        }
    }
    let sig = repo
        .signature()
        .context("set user.name and user.email with git config first")?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;
    // Finishes a cherry-pick or merge whose conflicts were resolved by hand.
    let _ = repo.cleanup_state();
    Ok(format!("committed {}", short(&oid.to_string())))
}

pub fn stash_save(workdir: &Path, message: &str) -> Result<String> {
    let mut repo = open(workdir)?;
    let sig = repo.signature()?;
    repo.stash_save(&sig, message, None)?;
    Ok(format!("stashed: {message}"))
}

pub fn stash_apply(workdir: &Path, index: usize) -> Result<String> {
    let mut repo = open(workdir)?;
    repo.stash_apply(index, None)?;
    Ok(format!("applied stash@{{{index}}}"))
}

pub fn stash_pop(workdir: &Path, index: usize) -> Result<String> {
    let mut repo = open(workdir)?;
    repo.stash_pop(index, None)?;
    Ok(format!("popped stash@{{{index}}}"))
}

pub fn stash_drop(workdir: &Path, index: usize) -> Result<String> {
    let mut repo = open(workdir)?;
    repo.stash_drop(index)?;
    Ok(format!("dropped stash@{{{index}}}"))
}

pub fn checkout(workdir: &Path, branch: &str) -> Result<String> {
    let repo = open(workdir)?;
    let refname = format!("refs/heads/{branch}");
    let object = repo.revparse_single(&refname)?;
    let mut builder = CheckoutBuilder::new();
    builder.safe();
    repo.checkout_tree(&object, Some(&mut builder))?;
    repo.set_head(&refname)?;
    Ok(format!("switched to {branch}"))
}

/// Applies a commit onto HEAD. If it conflicts, the conflicts are left in the
/// index for the Conflicts view and the commit is finished later from Status.
pub fn cherry_pick(workdir: &Path, oid: &str) -> Result<String> {
    let repo = open(workdir)?;
    let commit = repo.find_commit(Oid::from_str(oid)?)?;
    repo.cherrypick(&commit, None)?;

    let mut index = repo.index()?;
    if index.has_conflicts() {
        return Ok(format!(
            "cherry-pick of {} hit conflicts; resolve them in Conflicts, then commit from Status",
            short(oid)
        ));
    }

    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;
    let head = repo.head()?.peel_to_commit()?;
    let committer = repo.signature()?;
    let message = commit.message().unwrap_or("cherry-pick").to_string();
    let author = commit.author();
    let new = repo.commit(Some("HEAD"), &author, &committer, &message, &tree, &[&head])?;
    let _ = repo.cleanup_state();
    Ok(format!(
        "cherry-picked {} as {}",
        short(oid),
        short(&new.to_string())
    ))
}

/// Resolves one conflicted file by taking the whole file from one side.
pub fn take_side(workdir: &Path, path: &str, side: Side) -> Result<String> {
    let repo = open(workdir)?;
    let mut index = repo.index()?;

    // None = path not in conflict; Some(None) = that side deleted the file.
    let mut chosen: Option<Option<Oid>> = None;
    for conflict in index.conflicts()? {
        let conflict = conflict?;
        let entry_path = conflict
            .our
            .as_ref()
            .or(conflict.their.as_ref())
            .map(|e| String::from_utf8_lossy(&e.path).into_owned());
        if entry_path.as_deref() == Some(path) {
            let entry = match side {
                Side::Ours => conflict.our.as_ref(),
                Side::Theirs => conflict.their.as_ref(),
            };
            chosen = Some(entry.map(|e| e.id));
            break;
        }
    }

    let Some(oid) = chosen else {
        bail!("{path} is not in conflict");
    };
    let full = workdir.join(path);
    match oid {
        Some(oid) => {
            let blob = repo.find_blob(oid)?;
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full, blob.content())?;
            index.add_path(Path::new(path))?;
        }
        None => {
            if full.exists() {
                fs::remove_file(&full)?;
            }
            index.remove_path(Path::new(path))?;
        }
    }
    index.write()?;
    let label = match side {
        Side::Ours => "ours",
        Side::Theirs => "theirs",
    };
    Ok(format!("took {label} for {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Creates a repo under target/ with a local identity configured.
    fn fresh_repo(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-repos")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let repo = Repository::init_opts(
            &dir,
            git2::RepositoryInitOptions::new().initial_head("main"),
        )
        .unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Test").unwrap();
        cfg.set_str("user.email", "test@example.com").unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn stage_and_commit() {
        let dir = fresh_repo("stage_and_commit");
        write(&dir, "a.txt", "one\n");
        stage(&dir, "a.txt").unwrap();
        let snap = load_snapshot(&dir).unwrap();
        assert!(snap
            .files
            .iter()
            .any(|f| f.path == "a.txt" && f.staged && f.code == 'A'));

        commit(&dir, "first").unwrap();
        let snap = load_snapshot(&dir).unwrap();
        assert_eq!(snap.commits.len(), 1);
        assert_eq!(snap.commits[0].summary, "first");
        assert!(snap.files.is_empty());
        assert!(
            commit(&dir, "nothing here").is_err(),
            "empty commit must be refused"
        );
    }

    #[test]
    fn unstage_modify_and_untracked() {
        let dir = fresh_repo("unstage_modify");
        write(&dir, "a.txt", "one\n");
        stage(&dir, "a.txt").unwrap();
        commit(&dir, "first").unwrap();

        write(&dir, "a.txt", "one\ntwo\n");
        write(&dir, "new.txt", "fresh\n");
        let snap = load_snapshot(&dir).unwrap();
        assert!(snap
            .files
            .iter()
            .any(|f| f.path == "a.txt" && !f.staged && f.code == 'M'));
        assert!(snap
            .files
            .iter()
            .any(|f| f.path == "new.txt" && !f.staged && f.code == '?'));

        let diff = preview(
            &dir,
            &Preview::Diff {
                path: "a.txt".into(),
                staged: false,
            },
        )
        .unwrap();
        assert!(diff.contains("+two"), "diff was: {diff}");
        let untracked = preview(
            &dir,
            &Preview::Diff {
                path: "new.txt".into(),
                staged: false,
            },
        )
        .unwrap();
        assert!(
            untracked.contains("+fresh"),
            "untracked diff was: {untracked}"
        );

        stage(&dir, "a.txt").unwrap();
        unstage(&dir, "a.txt").unwrap();
        let snap = load_snapshot(&dir).unwrap();
        assert!(!snap.files.iter().any(|f| f.staged));
        assert!(snap.files.iter().any(|f| f.path == "a.txt" && !f.staged));
    }

    #[test]
    fn stash_round_trip() {
        let dir = fresh_repo("stash_round_trip");
        write(&dir, "a.txt", "one\n");
        stage(&dir, "a.txt").unwrap();
        commit(&dir, "first").unwrap();

        write(&dir, "a.txt", "changed\n");
        stash_save(&dir, "wip").unwrap();
        let snap = load_snapshot(&dir).unwrap();
        assert!(snap.files.is_empty());
        assert_eq!(snap.stashes.len(), 1);
        assert!(snap.stashes[0].message.contains("wip"));

        stash_pop(&dir, 0).unwrap();
        let snap = load_snapshot(&dir).unwrap();
        assert!(snap.stashes.is_empty());
        assert!(snap.files.iter().any(|f| f.path == "a.txt" && !f.staged));
    }

    #[test]
    fn cherry_pick_conflict_and_resolution() {
        let dir = fresh_repo("cherry_pick_conflict");
        write(&dir, "a.txt", "base\n");
        stage(&dir, "a.txt").unwrap();
        commit(&dir, "base").unwrap();

        {
            let repo = open(&dir).unwrap();
            let head = repo.head().unwrap().peel_to_commit().unwrap();
            repo.branch("feature", &head, false).unwrap();
        }
        checkout(&dir, "feature").unwrap();
        write(&dir, "a.txt", "feature\n");
        stage(&dir, "a.txt").unwrap();
        commit(&dir, "feature change").unwrap();
        let feature_oid = load_snapshot(&dir).unwrap().commits[0].full.clone();

        checkout(&dir, "main").unwrap();
        write(&dir, "a.txt", "main\n");
        stage(&dir, "a.txt").unwrap();
        commit(&dir, "main change").unwrap();

        cherry_pick(&dir, &feature_oid).unwrap();
        let snap = load_snapshot(&dir).unwrap();
        assert_eq!(snap.conflicts, vec!["a.txt".to_string()]);

        take_side(&dir, "a.txt", Side::Theirs).unwrap();
        let snap = load_snapshot(&dir).unwrap();
        assert!(snap.conflicts.is_empty());

        commit(&dir, "resolve conflict").unwrap();
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "feature\n");
    }

    #[test]
    fn clean_cherry_pick_commits_directly() {
        let dir = fresh_repo("clean_cherry_pick");
        write(&dir, "a.txt", "base\n");
        stage(&dir, "a.txt").unwrap();
        commit(&dir, "base").unwrap();
        {
            let repo = open(&dir).unwrap();
            let head = repo.head().unwrap().peel_to_commit().unwrap();
            repo.branch("feature", &head, false).unwrap();
        }
        checkout(&dir, "feature").unwrap();
        write(&dir, "b.txt", "extra\n");
        stage(&dir, "b.txt").unwrap();
        commit(&dir, "add b").unwrap();
        let oid = load_snapshot(&dir).unwrap().commits[0].full.clone();

        checkout(&dir, "main").unwrap();
        cherry_pick(&dir, &oid).unwrap();
        let snap = load_snapshot(&dir).unwrap();
        assert_eq!(snap.commits.len(), 2);
        assert_eq!(snap.commits[0].summary, "add b");
        assert!(snap.conflicts.is_empty());
        assert!(snap.files.is_empty());
    }
}
