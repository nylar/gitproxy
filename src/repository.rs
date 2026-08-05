use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use buffa::{EnumValue, MessageField};
use chrono::{DateTime, Utc};
use git2::{
    DiffFindOptions, DiffOptions, Oid, Repository as GitRepository, Signature, WorktreeAddOptions,
    WorktreePruneOptions, build::CheckoutBuilder,
};

use crate::error::Result;

const DEFAULT_PRIMARY_BRANCH: &str = "main";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MergePreference {
    #[default]
    Normal,
    FastForward,
}

#[derive(Debug)]
pub struct Author {
    pub name: String,
    pub email: String,
}

pub struct Repository {
    repo: GitRepository,
    repo_dir: PathBuf,
}

impl Repository {
    pub fn open(repo_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(repo_dir)?;

        let git_dir = repo_dir.join(".git");

        let repo = if std::fs::exists(&git_dir)? {
            GitRepository::open_bare(git_dir)?
        } else {
            std::fs::create_dir_all(&git_dir)?;
            let repo = GitRepository::init_bare(&git_dir)?;
            Self::init_repo(&repo, repo_dir)?;
            repo
        };

        Ok(Self {
            repo,
            repo_dir: repo_dir.to_path_buf(),
        })
    }

    fn init_repo(repo: &GitRepository, repo_dir: &Path) -> Result<()> {
        let sig = repo.signature()?;
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
        Ok(Worktree::open(repo, name, path))
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
}

impl Worktree {
    fn open(repo: GitRepository, name: &str, path: PathBuf) -> Self {
        Self {
            repo,
            name: name.to_owned(),
            path,
        }
    }

    pub fn new(&self, name: &str) -> Result<Self> {
        let path = self.path.parent().unwrap().join(name);

        let worktree = self
            .repo
            .worktree(name, &path, Some(&WorktreeAddOptions::new()))?;

        let repo = GitRepository::open_from_worktree(&worktree)?;
        Ok(Self::open(repo, name, path))
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

    pub fn revert(&self, reference: &str, preference: MergePreference) -> Result<Oid> {
        match preference {
            MergePreference::FastForward => todo!("Handle fast-forward revert"),
            MergePreference::Normal => {
                let oid = Oid::from_str(reference)?;
                let commit = self.repo.find_commit(oid)?;

                let mut checkout = CheckoutBuilder::new();
                checkout.allow_conflicts(true).conflict_style_merge(true);

                let mut opts = git2::RevertOptions::new();
                opts.mainline(1);
                opts.checkout_builder(checkout);
                self.repo.revert(&commit, Some(&mut opts))?;

                let sig = self.repo.signature()?;
                let oid = self.commit(
                    &format!("Reverted {}", reference),
                    &Author {
                        name: sig.name()?.to_owned(),
                        email: sig.email()?.to_owned(),
                    },
                )?;

                Ok(oid)
            }
        }
    }

    pub fn merge(
        &self,
        reference: &git2::Reference<'_>,
        preference: MergePreference,
    ) -> Result<Oid> {
        let commit = self
            .repo
            .find_annotated_commit(reference.peel_to_commit()?.id())?;

        let oid = self.do_merge(&commit, preference)?;
        Ok(oid)
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

    pub fn log(&self) -> Result<impl Iterator<Item = LogEntry>> {
        let mut revwalk = self.repo.revwalk()?;
        revwalk.push_head()?;

        Ok(revwalk
            .flatten()
            .flat_map(|id| self.repo.find_commit(id))
            .map(|commit| {
                let author = commit.author();

                let time = DateTime::from_timestamp_secs(commit.time().seconds()).unwrap();

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

    pub fn diff(&self, from_branch: Option<&str>, to_branch: &str) -> Result<Diff> {
        let mut opts = DiffOptions::new();
        opts.force_text(true);

        let from_ref = match from_branch {
            Some(from_ref) => from_ref,
            None => DEFAULT_PRIMARY_BRANCH,
        };

        let from_ref = self.repo.find_branch(from_ref, git2::BranchType::Local)?;
        let to_ref = self.repo.find_branch(to_branch, git2::BranchType::Local)?;

        let from_tree = from_ref.get().peel_to_tree()?;
        let to_tree = to_ref.get().peel_to_tree()?;

        let mut diff =
            self.repo
                .diff_tree_to_tree(Some(&from_tree), Some(&to_tree), Some(&mut opts))?;

        let mut opts = DiffFindOptions::new();
        opts.renames(true);

        diff.find_similar(Some(&mut opts))?;
        let stats = diff.stats()?;

        let diff_report = RefCell::new(DiffReport {
            diff: Diff {
                files_changes: stats.files_changed(),
                insertions: stats.insertions(),
                deletions: stats.deletions(),
                deltas: vec![],
            },
            delta_index: -1,
        });

        diff.foreach(
            &mut |diff_delta, _| {
                let new_file = diff_delta.new_file();
                let old_file = diff_delta.old_file();

                let mut diff_report = diff_report.borrow_mut();

                diff_report.diff.deltas.push(Delta {
                    status: DeltaStatus::from(diff_delta.status()),
                    old_file: DeltaFile {
                        path: old_file.path().map(|p| p.to_path_buf()),
                        size: old_file.size() as usize,
                    },
                    new_file: DeltaFile {
                        path: new_file.path().map(|p| p.to_path_buf()),
                        size: new_file.size() as usize,
                    },
                    lines: Vec::new(),
                });
                diff_report.delta_index += 1;

                true
            },
            None,
            None,
            Some(&mut |_, _, diff_line| {
                let mut diff_report = diff_report.borrow_mut();
                if diff_report.delta_index < 0 {
                    return false;
                }
                let index = diff_report.delta_index as usize;

                if let Some(delta) = diff_report.diff.deltas.get_mut(index) {
                    delta.lines.push(DeltaLine {
                        new_line_number: diff_line.new_lineno(),
                        old_line_number: diff_line.old_lineno(),
                        number_lines: diff_line.num_lines(),
                        content_offset: diff_line.content_offset(),
                        content: diff_line.content().to_vec(),
                        origin: DeltaOrigin::from(diff_line.origin_value()),
                    })
                }

                true
            }),
        )?;

        Ok(diff_report.into_inner().diff)
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

    fn fast_forward(&self, lb: &mut git2::Reference, rc: &git2::AnnotatedCommit) -> Result<Oid> {
        let name = match lb.name() {
            Ok(s) => s.to_string(),
            Err(_) => String::from_utf8_lossy(lb.name_bytes()).to_string(),
        };
        let msg = format!("Fast-Forward: Setting {} to id: {}", name, rc.id());
        lb.set_target(rc.id(), &msg)?;
        self.repo.set_head(&name)?;
        self.repo
            .checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
        Ok(rc.id())
    }

    fn normal_merge(
        &self,
        local: &git2::AnnotatedCommit,
        remote: &git2::AnnotatedCommit,
    ) -> Result<Oid> {
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
            self.repo.checkout_index(Some(&mut idx), None)?;
            todo!("Merge conflicts detected...");
        }
        let result_tree = self.repo.find_tree(idx.write_tree_to(&self.repo)?)?;
        let msg = format!("Merge: {} into {}", remote.id(), local.id());
        let sig = self.repo.signature()?;
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

        Ok(merge_commit)
    }

    fn do_merge(
        &self,
        fetch_commit: &git2::AnnotatedCommit<'_>,
        preference: MergePreference,
    ) -> Result<Oid> {
        let analysis = self.repo.merge_analysis(&[fetch_commit])?;

        match analysis {
            (analysis, _)
                if analysis.is_fast_forward() && preference == MergePreference::FastForward =>
            {
                let refname = format!("refs/heads/{}", self.name);
                match self.repo.find_reference(&refname) {
                    Ok(mut r) => Ok(self.fast_forward(&mut r, fetch_commit)?),
                    Err(_) => {
                        self.repo.reference(
                            &refname,
                            fetch_commit.id(),
                            true,
                            &format!("Setting {} to {}", self.name, fetch_commit.id()),
                        )?;
                        self.repo.set_head(&refname)?;
                        self.repo.checkout_head(Some(
                            git2::build::CheckoutBuilder::default()
                                .allow_conflicts(true)
                                .conflict_style_merge(true)
                                .force(),
                        ))?;
                        Ok(fetch_commit.id())
                    }
                }
            }
            (analysis, _) if analysis.is_normal() && preference == MergePreference::Normal => {
                let head_commit = self
                    .repo
                    .reference_to_annotated_commit(&self.repo.head()?)?;
                Ok(self.normal_merge(&head_commit, fetch_commit)?)
            }
            _ => panic!("Unhandled merge"),
        }
    }
}

#[derive(Debug)]
pub struct LogEntry {
    pub message: String,
    pub author: Author,
    pub commit: String,
    pub time: DateTime<Utc>,
}

#[derive(Debug)]
struct DiffReport {
    diff: Diff,
    delta_index: isize,
}

#[derive(Debug)]
pub struct Diff {
    pub files_changes: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub deltas: Vec<Delta>,
}

impl From<Diff> for crate::proto::gitproxy::v1::Diff {
    fn from(value: Diff) -> Self {
        Self {
            files_changes: value.files_changes as u64,
            insertions: value.insertions as u64,
            deletions: value.deletions as u64,
            deltas: value.deltas.into_iter().map(From::from).collect(),
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub struct Delta {
    pub status: DeltaStatus,
    pub old_file: DeltaFile,
    pub new_file: DeltaFile,
    pub lines: Vec<DeltaLine>,
}

impl From<Delta> for crate::proto::gitproxy::v1::DiffDelta {
    fn from(value: Delta) -> Self {
        Self {
            status: EnumValue::Known(value.status.into()),
            old_file: MessageField::some(value.old_file.into()),
            new_file: MessageField::some(value.new_file.into()),
            lines: value.lines.into_iter().map(From::from).collect(),
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub enum DeltaStatus {
    Unmodified,
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    Ignored,
    Untracked,
    Typechange,
    Unreadable,
    Conflicted,
}

impl From<DeltaStatus> for crate::proto::gitproxy::v1::diff_delta::Status {
    fn from(value: DeltaStatus) -> Self {
        match value {
            DeltaStatus::Unmodified => Self::Unmodified,
            DeltaStatus::Added => Self::Added,
            DeltaStatus::Deleted => Self::Deleted,
            DeltaStatus::Modified => Self::Modified,
            DeltaStatus::Renamed => Self::Renamed,
            DeltaStatus::Copied => Self::Copied,
            DeltaStatus::Ignored => Self::Ignored,
            DeltaStatus::Untracked => Self::Untracked,
            DeltaStatus::Typechange => Self::Typechange,
            DeltaStatus::Unreadable => Self::Unreadable,
            DeltaStatus::Conflicted => Self::Conflicted,
        }
    }
}

impl From<git2::Delta> for DeltaStatus {
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

#[derive(Debug)]
pub struct DeltaFile {
    pub path: Option<PathBuf>,
    pub size: usize,
}

impl From<DeltaFile> for crate::proto::gitproxy::v1::diff_delta::File {
    fn from(value: DeltaFile) -> Self {
        Self {
            path: value
                .path
                .map(|p| p.as_os_str().to_str().unwrap().to_owned()),
            size: value.size as u64,
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub struct DeltaLine {
    pub old_line_number: Option<u32>,
    pub new_line_number: Option<u32>,
    pub number_lines: u32,
    pub content_offset: i64,
    pub content: Vec<u8>,
    pub origin: DeltaOrigin,
}

impl From<DeltaLine> for crate::proto::gitproxy::v1::diff_delta::Line {
    fn from(value: DeltaLine) -> Self {
        Self {
            old_line_number: value.old_line_number,
            new_line_number: value.new_line_number,
            number_lines: value.number_lines,
            content_offset: value.content_offset,
            content: value.content,
            origin: EnumValue::Known(value.origin.into()),
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub enum DeltaOrigin {
    Context,
    Addition,
    Deletion,
    ContextEOFNL,
    AddEOFNL,
    DeleteEOFNL,
    FileHeader,
    HunkHeader,
    Binary,
}

impl From<git2::DiffLineType> for DeltaOrigin {
    fn from(value: git2::DiffLineType) -> Self {
        match value {
            git2::DiffLineType::Context => Self::Context,
            git2::DiffLineType::Addition => Self::Addition,
            git2::DiffLineType::Deletion => Self::Deletion,
            git2::DiffLineType::ContextEOFNL => Self::ContextEOFNL,
            git2::DiffLineType::AddEOFNL => Self::AddEOFNL,
            git2::DiffLineType::DeleteEOFNL => Self::DeleteEOFNL,
            git2::DiffLineType::FileHeader => Self::FileHeader,
            git2::DiffLineType::HunkHeader => Self::HunkHeader,
            git2::DiffLineType::Binary => Self::Binary,
        }
    }
}

impl From<DeltaOrigin> for crate::proto::gitproxy::v1::diff_delta::line::Origin {
    fn from(value: DeltaOrigin) -> Self {
        match value {
            DeltaOrigin::Context => Self::Context,
            DeltaOrigin::Addition => Self::Addition,
            DeltaOrigin::Deletion => Self::Deletion,
            DeltaOrigin::ContextEOFNL => Self::Contexteofnl,
            DeltaOrigin::AddEOFNL => Self::Addeofnl,
            DeltaOrigin::DeleteEOFNL => Self::Deleteeofnl,
            DeltaOrigin::FileHeader => Self::Fileheader,
            DeltaOrigin::HunkHeader => Self::Hunkheader,
            DeltaOrigin::Binary => Self::Binary,
        }
    }
}
