use std::path::PathBuf;

use git2::Oid;

use crate::{
    error::{Error, Result},
    repository::{Author, Diff, LogEntry, Repository},
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

    pub fn merge(&self, branch: String) -> Result<Oid> {
        let repo = Repository::open(&self.repo_dir, &self.author)?;
        let main = repo.primary_worktree()?;
        let worktree = repo.worktree(&branch)?;

        let head = worktree.head()?;
        let commit = main.merge(&head)?;
        worktree.remove()?;
        main.delete_branch(&branch)?;
        Ok(commit)
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

    pub fn diff(&self, base_reference: String, target_reference: String) -> Result<Diff> {
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
}
