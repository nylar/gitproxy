use std::path::{Path, PathBuf};

use buffa::{EnumValue, MessageField};
use buffa_types::Timestamp;
use git2::Oid;

use crate::{
    error::{Error, Result},
    proto::gitproxy::v1::{self, CommitAuthor, File},
    repository::{
        Author, ConflictDiff, LogEntry, LogOrder, Merge, PatchDiff, Repository, Strategy, Tag,
    },
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
        let head = repo.head("main")?;
        Ok(head)
    }

    pub fn remove_repository(&self) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        repo.remove()?;
        Ok(())
    }

    pub fn list_branches(&self) -> Result<Vec<String>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let branches = repo.list_branches()?;
        Ok(branches.collect())
    }

    pub fn create_branch(&self, branch: String) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let mut branches = repo.list_branches()?;

        if branches.any(|b| b == branch) {
            return Err(Error::BranchExists(branch));
        }

        repo.branch_commit(&branch)?;
        Ok(())
    }

    pub fn delete_branch(&self, branch: String) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        repo.delete_branch(&branch)?;
        Ok(())
    }

    pub fn list_tags(&self) -> Result<Vec<v1::Tag>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let tags = repo.list_tags()?.iter().map(Into::into).collect();
        Ok(tags)
    }

    pub fn create_tag(
        &self,
        name: String,
        message: String,
        commit: String,
        author: Author,
        overwrite: bool,
    ) -> Result<v1::Tag> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let tag = repo.create_tag(&name, &author, &message, &commit, overwrite)?;
        Ok(v1::Tag::from(&tag))
    }

    pub fn delete_tag(&self, name: String) -> Result<()> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        repo.delete_tag(&name)?;
        Ok(())
    }

    pub fn commit(
        &self,
        branch: String,
        message: String,
        author: Author,
        files: Vec<File>,
    ) -> Result<Oid> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let commit = repo.commit_files(
            &branch,
            &message,
            &author,
            files
                .iter()
                .map(|f| (Path::new(&f.path), f.contents.as_ref())),
        )?;
        Ok(commit)
    }

    pub fn merge(
        &self,
        source_branch: String,
        target_branch: String,
        dry_run: bool,
    ) -> Result<Merge> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let merge = repo.merge(&source_branch, &target_branch, dry_run)?;

        // Only clear out old branch if we are able to successfully merge
        if let Merge::Ok(Some(_)) = merge {
            repo.delete_branch(&source_branch)?;
        }
        Ok(merge)
    }

    pub fn log(
        &self,
        source_branch: String,
        order: LogOrder,
        target_branch: Option<String>,
    ) -> Result<Vec<LogEntry>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        Ok(repo
            .log(order, &source_branch, target_branch.as_deref())?
            .collect())
    }

    pub fn revert_merge(
        &self,
        target_branch: String,
        commit: String,
        strategy: Option<v1::revert_request::Strategy>,
        dry_run: bool,
    ) -> Result<Merge> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let merge = repo.revert(&target_branch, &commit, strategy.map(Into::into), dry_run)?;
        Ok(merge)
    }

    pub fn diff(&self, base_reference: String, target_reference: String) -> Result<Vec<PatchDiff>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let diff = repo.diff(&base_reference, &target_reference)?;
        Ok(diff)
    }

    pub fn status(&self, source_branch: String, target_branch: String) -> Result<bool> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let clean = repo.clean(&source_branch, &target_branch)?;
        Ok(clean)
    }

    pub fn get_blob(&self, commit: String, path: String) -> Result<File> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let blob = repo.blob(&commit, &path)?;
        Ok(File {
            path,
            contents: blob.to_vec(),
            ..Default::default()
        })
    }

    pub fn list_blobs(&self, commit: String) -> Result<Vec<File>> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let blobs = repo
            .all_blobs(&commit)?
            .iter()
            .map(|(path, contents)| File {
                path: path.display().to_string(),
                contents: contents.to_vec(),
                ..Default::default()
            })
            .collect();
        Ok(blobs)
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
            contents: value.contents.clone(),
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

impl From<Merge> for v1::RevertResponse {
    fn from(value: Merge) -> Self {
        let mut resp = v1::RevertResponse {
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

impl From<v1::revert_request::Strategy> for Strategy {
    fn from(value: v1::revert_request::Strategy) -> Self {
        match value {
            v1::revert_request::Strategy::STRATEGY_UNSPECIFIED => Strategy::default(),
            v1::revert_request::Strategy::STRATEGY_KEEP_MAINLINE => Self::KeepMainline,
            v1::revert_request::Strategy::STRATEGY_KEEP_OTHER => Strategy::KeepOther,
        }
    }
}

impl From<&Tag> for v1::Tag {
    fn from(value: &Tag) -> Self {
        Self {
            name: value.name.to_owned(),
            commit: value.commit.to_owned(),
            time: MessageField::some(Timestamp::from_unix_secs(value.time.as_second())),
            author: MessageField::some(CommitAuthor {
                name: value.author.name.to_owned(),
                email: value.author.email.to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}
