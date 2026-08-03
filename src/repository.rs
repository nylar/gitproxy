use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use git2::{
    Oid, Repository as GitRepository, Signature, WorktreeAddOptions, WorktreePruneOptions,
    build::CheckoutBuilder,
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
