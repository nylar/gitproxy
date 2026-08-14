use std::path::PathBuf;

use buffa::EnumValue;
use git2::Oid;

use crate::{
    error::{Error, Result},
    proto::gitproxy::v1,
    repository::{Author, ConflictDiff, LogEntry, Merge, PatchDiff, Repository},
};

pub struct Service {
    repo_dir: PathBuf,
    author: Author,
}

impl Service {
    pub fn new(repo_dir: PathBuf, author: Author) -> Self {
        Self { repo_dir, author }
    }

    pub fn fetch_repository_head_commit(&self) -> Result<Oid> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        let head = main.head()?.peel_to_commit()?;
        Ok(head.id())
    }

    pub fn remove_repository(&self) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        repo.remove()?;
        Ok(())
    }

    pub fn list_branches(&self) -> Result<Vec<String>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        let branches = main.list_branches()?;
        Ok(branches)
    }

    pub fn create_branch(&self, branch: String) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;

        let branches = main.list_branches()?;

        if branches.iter().any(|b| b == &branch) {
            return Err(Error::BranchExists(branch));
        }

        main.new(&branch)?;
        Ok(())
    }

    pub fn delete_branch(&self, branch: String) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        let worktree = repo.worktree(&branch)?;
        worktree.remove()?;
        main.delete_branch(&branch)?;
        Ok(())
    }

    pub fn list_tags(&self) -> Result<Vec<String>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        let tags = main.list_tags()?;
        Ok(tags)
    }

    pub fn create_tag(
        &self,
        name: String,
        message: String,
        commit: Option<String>,
        author: Author,
        overwrite: bool,
    ) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;

        main.create_tag(&name, &author, &message, commit.as_deref(), overwrite)?;
        Ok(())
    }

    pub fn delete_tag(&self, name: String) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        main.delete_tag(&name)?;
        Ok(())
    }

    pub fn checkout_tag(&self, name: String) -> Result<PathBuf> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let path = repo.checkout_tag(&name)?;
        Ok(path)
    }

    pub fn commit(&self, branch: String, message: String, author: Author) -> Result<Oid> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let branch = repo.worktree(&branch)?;
        let commit = branch.commit_all(&message, &author)?;
        Ok(commit)
    }

    pub fn merge(&self, branch: String, dry_run: bool) -> Result<Merge> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        let worktree = repo.worktree(&branch)?;

        let head = worktree.head()?;
        let merge = main.merge(&head, dry_run)?;

        // Only clear out old branch if we are able to successfully merge
        if let Merge::Ok(Some(_)) = merge {
            worktree.remove()?;
            main.delete_branch(&branch)?;
        }
        Ok(merge)
    }

    pub fn log(&self, branch: Option<String>) -> Result<Vec<LogEntry>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let worktree = match branch {
            Some(branch) => repo.worktree(&branch),
            None => repo.primary_worktree(),
        }?;
        Ok(worktree.log()?.collect())
    }

    pub fn revert_merge(&self, commit: String) -> Result<Oid> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        let commit = main.revert(&commit)?;
        Ok(commit)
    }

    pub fn diff(&self, base_reference: String, target_reference: String) -> Result<Vec<PatchDiff>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        let diff = main.diff(&base_reference, &target_reference)?;
        Ok(diff)
    }

    pub fn status(&self, branch: Option<String>) -> Result<bool> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;

        let worktree = match branch {
            Some(branch) => repo.worktree(&branch),
            None => repo.primary_worktree(),
        }?;

        let clean = worktree.clean()?;
        Ok(clean)
    }

    pub fn revert_commit(&self, branch: Option<String>, commit: String) -> Result<Oid> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;

        let worktree = match branch {
            Some(branch) => repo.worktree(&branch),
            None => repo.primary_worktree(),
        }?;

        let commit = worktree.revert(&commit)?;
        Ok(commit)
    }
}

impl From<&PatchDiff> for v1::DiffFile {
    fn from(value: &PatchDiff) -> Self {
        let patches = value.patch.0.iter().map(Into::into).collect();

        Self {
            status: EnumValue::Known(value.status.into()),
            old_path: value.old_path.as_ref().map(|p| p.display().to_string()),
            new_path: value.new_path.as_ref().map(|p| p.display().to_string()),
            patches,
            ..Default::default()
        }
    }
}

impl From<&json_patch::PatchOperation> for v1::DiffPatch {
    fn from(value: &json_patch::PatchOperation) -> Self {
        match value {
            json_patch::PatchOperation::Add(add_operation) => {
                let value = serde_json::to_string(&add_operation.value).unwrap();
                v1::DiffPatch {
                    operation: Some(v1::diff_patch::Operation::Add(Box::new(
                        v1::diff_patch::Add {
                            path: add_operation.path.to_string(),
                            value,
                            ..Default::default()
                        },
                    ))),
                    ..Default::default()
                }
            }
            json_patch::PatchOperation::Remove(remove_operation) => v1::DiffPatch {
                operation: Some(v1::diff_patch::Operation::Remove(Box::new(
                    v1::diff_patch::Remove {
                        path: remove_operation.path.to_string(),
                        ..Default::default()
                    },
                ))),
                ..Default::default()
            },
            json_patch::PatchOperation::Replace(replace_operation) => {
                let value = serde_json::to_string(&replace_operation.value).unwrap();
                v1::DiffPatch {
                    operation: Some(v1::diff_patch::Operation::Replace(Box::new(
                        v1::diff_patch::Replace {
                            path: replace_operation.path.to_string(),
                            value,
                            ..Default::default()
                        },
                    ))),
                    ..Default::default()
                }
            }
            json_patch::PatchOperation::Move(move_operation) => v1::DiffPatch {
                operation: Some(v1::diff_patch::Operation::Move(Box::new(
                    v1::diff_patch::Move {
                        from: move_operation.from.to_string(),
                        path: move_operation.path.to_string(),
                        ..Default::default()
                    },
                ))),
                ..Default::default()
            },
            json_patch::PatchOperation::Copy(copy_operation) => v1::DiffPatch {
                operation: Some(v1::diff_patch::Operation::Copy(Box::new(
                    v1::diff_patch::Copy {
                        from: copy_operation.from.to_string(),
                        path: copy_operation.path.to_string(),
                        ..Default::default()
                    },
                ))),
                ..Default::default()
            },
            json_patch::PatchOperation::Test(test_operation) => {
                let value = serde_json::to_string(&test_operation.value).unwrap();
                v1::DiffPatch {
                    operation: Some(v1::diff_patch::Operation::Test(Box::new(
                        v1::diff_patch::Test {
                            path: test_operation.path.to_string(),
                            value,
                            ..Default::default()
                        },
                    ))),
                    ..Default::default()
                }
            }
        }
    }
}

impl From<git2::Delta> for v1::diff_file::Status {
    fn from(value: git2::Delta) -> Self {
        match value {
            git2::Delta::Unmodified => Self::Unmodified,
            git2::Delta::Added => Self::Added,
            git2::Delta::Deleted => Self::Deleted,
            git2::Delta::Modified => Self::Modified,
            git2::Delta::Renamed => Self::Renamed,
            git2::Delta::Copied => Self::Copied,
            git2::Delta::Ignored => Self::Ignored,
            git2::Delta::Untracked => Self::Untracked,
            git2::Delta::Typechange => Self::Typechange,
            git2::Delta::Unreadable => Self::Unreadable,
            git2::Delta::Conflicted => Self::Conflicted,
        }
    }
}

impl From<&ConflictDiff> for v1::ConflictDiff {
    fn from(value: &ConflictDiff) -> Self {
        let ours = value.ours.0.iter().map(Into::into).collect();
        let theirs = value.theirs.0.iter().map(Into::into).collect();

        Self {
            path: value.path.display().to_string(),
            ours,
            theirs,
            ..Default::default()
        }
    }
}

impl From<Merge> for v1::MergeResponse {
    fn from(value: Merge) -> Self {
        let mut resp = v1::MergeResponse {
            commit: None,
            conflicts: Vec::new(),
            ..Default::default()
        };

        match value {
            Merge::Ok(oid) => resp.commit = oid.map(|oid| oid.to_string()),
            Merge::Conflicts(conflict_diffs) => {
                resp.conflicts = conflict_diffs.iter().map(Into::into).collect()
            }
        }

        resp
    }
}
