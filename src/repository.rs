use std::path::{Path, PathBuf};

use git2::{
    Delta, DiffFindOptions, DiffOptions, Index, IndexEntry, Oid, Repository as GitRepository,
    Signature, Sort, Tree, WorktreeAddOptions, WorktreePruneOptions, build::CheckoutBuilder,
};
use jiff::Timestamp;
use json_patch::Patch;
use serde_json::Value;

use crate::error::{Error, Result};

const DEFAULT_PRIMARY_BRANCH: &str = "main";

#[derive(Clone, Debug)]
pub struct Author {
    pub name: String,
    pub email: String,
}

pub struct Repository {
    repo: GitRepository,
    repo_dir: PathBuf,
    default_author: Author,
}

impl Repository {
    pub fn open(repo_dir: &Path, default_author: &Author) -> Result<Self> {
        std::fs::create_dir_all(repo_dir)?;

        let git_dir = repo_dir.join(".git");

        let repo = if std::fs::exists(&git_dir)? {
            GitRepository::open_bare(git_dir)?
        } else {
            std::fs::create_dir_all(&git_dir)?;
            let repo = GitRepository::init_bare(&git_dir)?;
            Self::init_repo(&repo, repo_dir, default_author)?;
            repo
        };

        Ok(Self {
            repo,
            repo_dir: repo_dir.to_path_buf(),
            default_author: default_author.clone(),
        })
    }

    fn init_repo(repo: &GitRepository, repo_dir: &Path, author: &Author) -> Result<()> {
        let sig = Signature::now(&author.name, &author.email)?;
        let tree_id = {
            let mut index = repo.index()?;
            index.write_tree()?
        };
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;

        repo.worktree(
            DEFAULT_PRIMARY_BRANCH,
            &repo_dir.join(DEFAULT_PRIMARY_BRANCH),
            Some(
                WorktreeAddOptions::new()
                    .checkout_existing(true)
                    .reference(None),
            ),
        )?;

        Ok(())
    }

    pub fn remove(&self) -> Result<()> {
        std::fs::remove_dir_all(&self.repo_dir)?;
        Ok(())
    }

    pub fn primary_worktree(&self) -> Result<Worktree> {
        self.worktree(DEFAULT_PRIMARY_BRANCH)
    }

    pub fn worktree(&self, name: &str) -> Result<Worktree> {
        let path = self.repo_dir.join(name);

        let worktree = if self
            .repo
            .worktrees()?
            .iter()
            .any(|worktree| worktree == Ok(Some(name)))
        {
            self.repo.find_worktree(name)?
        } else {
            self.repo
                .worktree(name, &path, Some(&WorktreeAddOptions::new()))?
        };

        let repo = GitRepository::open_from_worktree(&worktree)?;
        Ok(Worktree::open(
            repo,
            name,
            path,
            self.default_author.clone(),
        ))
    }

    pub fn checkout_tag(&self, name: &str) -> Result<PathBuf> {
        let reference = self.repo.find_reference(&format!("refs/tags/{}", name))?;

        let branch = self.repo.branch(name, &reference.peel_to_commit()?, true)?;
        let branch_ref = branch.into_reference();

        let path = self.repo_dir.join(name);
        let mut opts = WorktreeAddOptions::new();
        opts.checkout_existing(true);
        opts.reference(Some(&branch_ref));

        self.repo.worktree(name, &path, Some(&opts))?;

        Ok(path.to_path_buf())
    }
}

pub struct Worktree {
    repo: GitRepository,
    name: String,
    path: PathBuf,
    default_author: Author,
}

impl Worktree {
    fn open(repo: GitRepository, name: &str, path: PathBuf, default_author: Author) -> Self {
        Self {
            repo,
            name: name.to_owned(),
            path,
            default_author,
        }
    }

    pub fn new(&self, name: &str) -> Result<Self> {
        let path = self.path.parent().unwrap().join(name);

        let worktree = self
            .repo
            .worktree(name, &path, Some(&WorktreeAddOptions::new()))?;

        let repo = GitRepository::open_from_worktree(&worktree)?;
        Ok(Self::open(repo, name, path, self.default_author.clone()))
    }

    pub fn remove(&self) -> Result<()> {
        std::fs::remove_dir_all(self.path())?;

        let worktree = self.repo.find_worktree(&self.name)?;

        let mut opts = WorktreePruneOptions::new();
        opts.working_tree(true);
        worktree.prune(Some(&mut opts))?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn head(&self) -> Result<git2::Reference<'_>> {
        let head = self.repo.head()?;
        Ok(head)
    }

    pub fn revert(&self, reference: &str) -> Result<Oid> {
        let oid = Oid::from_str(reference)?;
        let commit = self.repo.find_commit(oid)?;

        let mut checkout = CheckoutBuilder::new();
        checkout.allow_conflicts(true).conflict_style_merge(true);

        let mut opts = git2::RevertOptions::new();
        if commit.parent_count() > 1 {
            opts.mainline(1);
        }
        opts.checkout_builder(checkout);
        self.repo.revert(&commit, Some(&mut opts))?;

        let sig = Signature::now(&self.default_author.name, &self.default_author.email)?;
        let oid = self.commit(
            &format!("Reverted {}", reference),
            &Author {
                name: sig.name()?.to_owned(),
                email: sig.email()?.to_owned(),
            },
        )?;

        Ok(oid)
    }

    pub fn merge(&self, reference: &git2::Reference<'_>, dry_run: bool) -> Result<Merge> {
        let remote_commit = self
            .repo
            .find_annotated_commit(reference.peel_to_commit()?.id())?;

        let local_commit = self
            .repo
            .reference_to_annotated_commit(&self.repo.head()?)?;

        self.normal_merge(&local_commit, &remote_commit, dry_run)
    }

    pub fn commit_all(&self, message: &str, author: &Author) -> Result<Oid> {
        let signature = Signature::now(&author.name, &author.email)?;

        let mut index = self.repo.index()?;
        index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let parent_commit = self.repo.head()?.peel_to_commit()?;
        let tree = self.repo.find_tree(tree_id)?;
        let oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent_commit],
        )?;

        Ok(oid)
    }

    pub fn log(
        &self,
        sort_order: LogOrder,
        parent_branch: Option<&str>,
    ) -> Result<impl Iterator<Item = LogEntry>> {
        let mut revwalk = self.repo.revwalk()?;
        revwalk.push_head()?;

        if let Some(branch) = parent_branch {
            let branch_commit = self
                .repo
                .resolve_reference_from_short_name(branch)?
                .peel_to_commit()?;
            revwalk.hide(branch_commit.id())?;
        }

        match sort_order {
            LogOrder::Normal => revwalk.set_sorting(Sort::TOPOLOGICAL)?,
            LogOrder::Reverse => revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?,
        }

        revwalk.set_sorting(Sort::TOPOLOGICAL)?;

        Ok(revwalk
            .flatten()
            .flat_map(|id| self.repo.find_commit(id))
            .map(|commit| {
                let author = commit.author();

                let time = Timestamp::from_second(commit.time().seconds()).unwrap_or_default();

                LogEntry {
                    message: commit.message().unwrap().to_owned(),
                    author: Author {
                        name: author.name().unwrap().to_owned(),
                        email: author.email().unwrap().to_owned(),
                    },
                    commit: commit.id().to_string(),
                    time,
                }
            }))
    }

    pub fn list_branches(&self) -> Result<Vec<String>> {
        let worktrees = self.repo.worktrees()?;
        Ok(worktrees
            .iter()
            .flatten()
            .flatten()
            .map(|w| w.to_owned())
            .filter(|w| w != DEFAULT_PRIMARY_BRANCH)
            .collect())
    }

    pub fn delete_branch(&self, name: &str) -> Result<()> {
        let mut branch = self.repo.find_branch(name, git2::BranchType::Local)?;
        branch.delete()?;
        Ok(())
    }

    pub fn list_tags(&self) -> Result<Vec<String>> {
        let tags = self.repo.tag_names(None)?;
        Ok(tags
            .iter()
            .flatten()
            .flatten()
            .map(|w| w.to_string())
            .collect())
    }

    pub fn create_tag(
        &self,
        name: &str,
        author: &Author,
        message: &str,
        commit: Option<&str>,
        force: bool,
    ) -> Result<()> {
        let signature = Signature::now(&author.name, &author.email)?;

        let reference = match commit {
            Some(commit) => self.repo.find_reference(commit)?,
            None => self.repo.head()?,
        };

        let target = reference.peel(git2::ObjectType::Commit)?;

        self.repo.tag(name, &target, &signature, message, force)?;

        Ok(())
    }

    pub fn delete_tag(&self, name: &str) -> Result<()> {
        self.repo.tag_delete(name)?;
        Ok(())
    }

    pub fn diff(&self, base_reference: &str, target_reference: &str) -> Result<Vec<PatchDiff>> {
        let mut opts = DiffOptions::new();
        opts.force_text(true);

        let base_tree = resolve_to_tree(&self.repo, base_reference)?;
        let target_tree = resolve_to_tree(&self.repo, target_reference)?;

        let mut diff =
            self.repo
                .diff_tree_to_tree(Some(&base_tree), Some(&target_tree), Some(&mut opts))?;

        let mut opts = DiffFindOptions::new();
        opts.renames(true);
        diff.find_similar(Some(&mut opts))?;

        let mut results = Vec::new();

        diff.foreach(
            &mut |delta, _| {
                let status = delta.status();

                if matches!(
                    status,
                    Delta::Added | Delta::Deleted | Delta::Modified | Delta::Renamed
                ) {
                    let file_path = match status {
                        Delta::Deleted => delta.old_file().path(),
                        _ => delta.new_file().path(),
                    };

                    if file_path.is_some()
                        && let Ok(patch) = diff_to_patch(&self.repo, &delta)
                    {
                        results.push(PatchDiff {
                            status,
                            old_path: delta.old_file().path().map(|p| p.to_path_buf()),
                            new_path: delta.new_file().path().map(|p| p.to_path_buf()),
                            patch,
                        });
                    }
                }
                true
            },
            None,
            None,
            None,
        )?;

        Ok(results)
    }

    pub fn clean(&self) -> Result<bool> {
        Ok(self.repo.statuses(None)?.is_empty())
    }

    fn commit(&self, message: &str, author: &Author) -> Result<Oid> {
        let signature = Signature::now(&author.name, &author.email)?;

        let mut index = self.repo.index()?;
        index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let parent_commit = self.repo.head()?.peel_to_commit()?;
        let tree = self.repo.find_tree(tree_id)?;
        let oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent_commit],
        )?;
        self.repo.cleanup_state()?;
        self.repo
            .checkout_head(Some(CheckoutBuilder::default().force()))?;
        Ok(oid)
    }

    fn normal_merge(
        &self,
        local: &git2::AnnotatedCommit,
        remote: &git2::AnnotatedCommit,
        dry_run: bool,
    ) -> Result<Merge> {
        let local_tree = self.repo.find_commit(local.id())?.tree()?;
        let remote_tree = self.repo.find_commit(remote.id())?.tree()?;
        let ancestor = self
            .repo
            .find_commit(self.repo.merge_base(local.id(), remote.id())?)?
            .tree()?;
        let mut idx = self
            .repo
            .merge_trees(&ancestor, &local_tree, &remote_tree, None)?;

        if idx.has_conflicts() {
            let conflicts = conflict_diffs(&self.repo, &idx)?;
            return Ok(Merge::Conflicts(conflicts));
        }

        if dry_run {
            return Ok(Merge::Ok(None));
        }

        let result_tree = self.repo.find_tree(idx.write_tree_to(&self.repo)?)?;
        let msg = format!("Merge: {} into {}", remote.id(), local.id());
        let sig = Signature::now(&self.default_author.name, &self.default_author.email)?;
        let local_commit = self.repo.find_commit(local.id())?;
        let remote_commit = self.repo.find_commit(remote.id())?;
        let merge_commit = self.repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &msg,
            &result_tree,
            &[&local_commit, &remote_commit],
        )?;
        self.repo
            .checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;

        Ok(Merge::Ok(Some(merge_commit)))
    }
}

#[derive(Debug)]
pub struct LogEntry {
    pub message: String,
    pub author: Author,
    pub commit: String,
    pub time: Timestamp,
}

fn resolve_to_tree<'a>(repo: &'a GitRepository, reference: &str) -> Result<Tree<'a>> {
    let object = repo.revparse_single(reference)?;
    let commit = object
        .into_commit()
        .map_err(|_| Error::InvalidCommit(reference.to_owned()))?;
    let tree = commit.tree()?;
    Ok(tree)
}

fn diff_to_patch(repo: &GitRepository, delta: &git2::DiffDelta) -> Result<Patch> {
    let status = delta.status();

    let old_json = if status == Delta::Added {
        Value::Object(serde_json::Map::new())
    } else {
        let old_blob = repo.find_blob(delta.old_file().id())?;
        serde_json::from_slice(old_blob.content())?
    };

    let new_json = if status == Delta::Deleted {
        Value::Object(serde_json::Map::new())
    } else {
        let new_blob = repo.find_blob(delta.new_file().id())?;
        serde_json::from_slice(new_blob.content())?
    };

    Ok(json_patch::diff(&old_json, &new_json))
}

fn conflict_diffs(repo: &GitRepository, index: &Index) -> Result<Vec<ConflictDiff>> {
    let mut diffs = Vec::new();

    for conflict in index.conflicts()? {
        let conflict = conflict?;
        let entry = conflict
            .ancestor
            .as_ref()
            .or(conflict.our.as_ref())
            .or(conflict.their.as_ref());

        if let Some(entry) = entry {
            let path = bytes2path(&entry.path);

            let ancestor = object_to_json(repo, &conflict.ancestor)?;
            let ours = object_to_json(repo, &conflict.our)?;
            let theirs = object_to_json(repo, &conflict.their)?;

            diffs.push(ConflictDiff {
                path: path.to_path_buf(),
                ours: json_patch::diff(&ancestor, &ours),
                theirs: json_patch::diff(&ancestor, &theirs),
            });
        }
    }

    Ok(diffs)
}

fn object_to_json(repo: &GitRepository, index_entry: &Option<IndexEntry>) -> Result<Value> {
    match index_entry {
        Some(entry) => {
            let blob = repo.find_blob(entry.id)?;
            let json = serde_json::from_slice(blob.content())?;
            Ok(json)
        }
        None => Ok(serde_json::json!({})),
    }
}

#[cfg(unix)]
pub fn bytes2path(b: &[u8]) -> &Path {
    use std::os::unix::ffi::OsStrExt;
    Path::new(std::ffi::OsStr::from_bytes(b))
}

#[cfg(windows)]
pub fn bytes2path(b: &[u8]) -> &Path {
    Path::new(str::from_utf8(b).unwrap())
}

#[derive(Debug)]
pub struct PatchDiff {
    pub status: git2::Delta,
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub patch: json_patch::Patch,
}

#[derive(Debug)]
pub enum Merge {
    Ok(Option<Oid>),
    Conflicts(Vec<ConflictDiff>),
}

#[derive(Debug)]
pub struct ConflictDiff {
    pub path: PathBuf,
    pub ours: json_patch::Patch,
    pub theirs: json_patch::Patch,
}

#[derive(Clone, Copy, Default)]
pub enum LogOrder {
    #[default]
    Normal,
    Reverse,
}
