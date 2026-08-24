use std::path::{Path, PathBuf};

use git2::{
    Commit, Delta, DiffFindOptions, DiffOptions, Index, IndexEntry, IndexTime, MergeOptions,
    ObjectType, Oid, Repository as GitRepository, Signature, Sort, Tree, TreeWalkResult,
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
            Self::init_repo(&repo, default_author)?;
            repo
        };

        Ok(Self {
            repo,
            repo_dir: repo_dir.to_path_buf(),
            default_author: default_author.clone(),
        })
    }

    pub fn head(&self, branch: &str) -> Result<Oid> {
        let commit = self.branch_commit(branch)?;
        Ok(commit.id())
    }

    pub fn branch_commit(&self, branch: &str) -> Result<Commit<'_>> {
        let commit = match self.repo.find_branch(branch, git2::BranchType::Local) {
            Ok(existing) => existing.into_reference().peel_to_commit()?,
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                let main_branch = self
                    .repo
                    .find_branch(DEFAULT_PRIMARY_BRANCH, git2::BranchType::Local)?;
                let main_commit = main_branch.into_reference().peel_to_commit()?;

                let new_branch = self.repo.branch(branch, &main_commit, false)?;
                new_branch.into_reference().peel_to_commit()?
            }
            Err(e) => return Err(Error::Git(e)),
        };
        Ok(commit)
    }

    pub fn delete_branch(&self, branch: &str) -> Result<()> {
        self.repo
            .find_branch(branch, git2::BranchType::Local)?
            .delete()?;
        Ok(())
    }

    pub fn list_branches(&self) -> Result<impl Iterator<Item = String>> {
        Ok(self
            .repo
            .branches(Some(git2::BranchType::Local))?
            .flatten()
            .map(|(branch, _)| branch.name().unwrap().unwrap().to_owned())
            .filter(|w| w != DEFAULT_PRIMARY_BRANCH))
    }

    fn init_repo(repo: &GitRepository, author: &Author) -> Result<()> {
        let sig = Signature::now(&author.name, &author.email)?;
        let tree_id = {
            let mut index = repo.index()?;
            index.write_tree()?
        };
        let tree = repo.find_tree(tree_id)?;
        repo.commit(
            Some(&format!("refs/heads/{}", DEFAULT_PRIMARY_BRANCH)),
            &sig,
            &sig,
            "Initial commit",
            &tree,
            &[],
        )?;

        repo.set_head(&format!("refs/heads/{}", DEFAULT_PRIMARY_BRANCH))?;

        Ok(())
    }

    pub fn remove(&self) -> Result<()> {
        std::fs::remove_dir_all(&self.repo_dir)?;
        Ok(())
    }

    pub fn list_tags(&self) -> Result<Vec<Tag>> {
        let mut tags = Vec::new();
        let references = self.repo.references()?;

        for reference in references {
            let reference = reference?;

            if reference.is_tag() {
                let target = reference
                    .target()
                    .ok_or_else(|| Error::Git(git2::Error::from_str("invalid ref target")))?;

                let object = self.repo.find_object(target, None)?;
                let commit = reference.peel_to_commit()?;

                let (name, email, time) = if let Ok(annotated_tag) = object.into_tag()
                    && let Some(tagger) = annotated_tag.tagger()
                {
                    let author_name = tagger.name()?.to_owned();
                    let author_email = tagger.email()?.to_owned();
                    let time = Timestamp::from_second(tagger.when().seconds())?;
                    (author_name, author_email, time)
                } else {
                    let author = commit.author();
                    let author_name = author.name()?.to_owned();
                    let author_email = author.email()?.to_owned();
                    let time = Timestamp::from_second(commit.time().seconds())?;
                    (author_name, author_email, time)
                };

                if let Ok(ref_name) = reference.name() {
                    tags.push(Tag {
                        name: ref_name
                            .strip_prefix("refs/tags/")
                            .unwrap_or(ref_name)
                            .to_owned(),
                        commit: commit.id().to_string(),
                        time,
                        author: Author { name, email },
                    });
                }
            }
        }

        tags.sort_by(|a, b| a.time.cmp(&b.time).reverse());

        Ok(tags)
    }

    pub fn create_tag(
        &self,
        name: &str,
        author: &Author,
        message: &str,
        commit: &str,
        force: bool,
    ) -> Result<Tag> {
        let signature = Signature::now(&author.name, &author.email)?;
        let target = self.repo.find_commit(Oid::from_str(commit)?)?;

        let commit = self
            .repo
            .tag(name, &target.into_object(), &signature, message, force)?;

        Ok(Tag {
            name: name.to_owned(),
            commit: commit.to_string(),
            time: Timestamp::from_second(signature.when().seconds())?,
            author: Author {
                name: author.name.to_owned(),
                email: author.email.to_owned(),
            },
        })
    }

    pub fn delete_tag(&self, name: &str) -> Result<()> {
        self.repo.tag_delete(name)?;
        Ok(())
    }

    pub fn commit_files(
        &self,
        branch: &str,
        message: &str,
        author: &Author,
        files: Vec<(PathBuf, Vec<u8>)>,
    ) -> Result<Oid> {
        let (parent_commits, parent_tree) =
            match self.repo.find_branch(branch, git2::BranchType::Local) {
                Ok(branch) => {
                    let commit = branch.into_reference().peel_to_commit()?;
                    let tree = commit.tree()?;
                    (vec![commit], Some(tree))
                }
                Err(e) if e.code() == git2::ErrorCode::NotFound => (vec![], None),
                Err(e) => return Err(Error::Git(e)),
            };

        let signature = Signature::now(&author.name, &author.email)?;

        let mut index = self.repo.index()?;
        index.clear()?;

        if let Some(tree) = &parent_tree {
            index.read_tree(tree)?;
        }

        for (path, contents) in files {
            let blob_id = self.repo.blob(&contents)?;

            let entry = new_index_entry(blob_id, &path, &contents);
            index.add(&entry)?;
        }
        let tree_id = index.write_tree_to(&self.repo)?;

        // Return current id if we didn't mutate the tree
        if let Some(old_tree) = &parent_tree
            && old_tree.id() == tree_id
        {
            return Ok(parent_commits[0].id());
        }

        let tree = self.repo.find_tree(tree_id)?;
        let oid = self.repo.commit(
            Some(&format!("refs/heads/{}", branch)),
            &signature,
            &signature,
            message,
            &tree,
            &parent_commits.iter().collect::<Vec<&git2::Commit>>(),
        )?;

        Ok(oid)
    }

    pub fn merge(&self, source_branch: &str, target_branch: &str, dry_run: bool) -> Result<Merge> {
        let source_commit = self
            .repo
            .find_branch(source_branch, git2::BranchType::Local)?
            .into_reference()
            .peel_to_commit()?;
        let source_tree = source_commit.tree()?;

        let target_commit = self
            .repo
            .find_branch(target_branch, git2::BranchType::Local)?
            .into_reference()
            .peel_to_commit()?;
        let target_tree = target_commit.tree()?;

        let msg = self.build_squash_merge(&source_commit, &target_commit)?;

        let ancestor = self
            .repo
            .merge_base(target_commit.id(), source_commit.id())?;
        let ancestor_commit = self.repo.find_commit(ancestor)?;
        let ancestor_tree = ancestor_commit.tree()?;

        let opts = MergeOptions::new();

        let mut idx =
            self.repo
                .merge_trees(&ancestor_tree, &target_tree, &source_tree, Some(&opts))?;

        if idx.has_conflicts() {
            let conflicts = conflict_diffs(&self.repo, &idx)?;
            return Ok(Merge::Conflicts(conflicts));
        }

        if dry_run {
            return Ok(Merge::Ok(None));
        }

        let squashed_tree_commit = idx.write_tree_to(&self.repo)?;
        let tree = self.repo.find_tree(squashed_tree_commit)?;

        let sig = Signature::now(&self.default_author.name, &self.default_author.email)?;
        let merge_commit = self.repo.commit(
            Some(&format!("refs/heads/{}", target_branch)),
            &sig,
            &sig,
            &msg,
            &tree,
            &[&target_commit],
        )?;

        Ok(Merge::Ok(Some(merge_commit)))
    }

    pub fn resolve_conflicts(
        &self,
        source_branch: &str,
        target_branch: &str,
        files: impl Iterator<Item = (PathBuf, Vec<u8>)>,
        author: &Author,
        message: &str,
    ) -> Result<Merge> {
        let source_commit = self
            .repo
            .find_branch(source_branch, git2::BranchType::Local)?
            .into_reference()
            .peel_to_commit()?;
        let target_commit = self
            .repo
            .find_branch(target_branch, git2::BranchType::Local)?
            .into_reference()
            .peel_to_commit()?;

        let mut opts = MergeOptions::new();
        opts.fail_on_conflict(false);

        let mut idx = self
            .repo
            .merge_commits(&source_commit, &target_commit, Some(&opts))?;

        for (path, contents) in files {
            let blob_id = self.repo.blob(&contents)?;
            idx.conflict_remove(&path)?;

            let entry = new_index_entry(blob_id, &path, &contents);
            idx.add(&entry)?;
        }

        if idx.has_conflicts() {
            let conflicts = conflict_diffs(&self.repo, &idx)?;
            return Ok(Merge::Conflicts(conflicts));
        }

        let tree_id = idx.write_tree_to(&self.repo)?;
        let tree = self.repo.find_tree(tree_id)?;

        let sig = Signature::now(&author.name, &author.email)?;

        let commit = self.repo.commit(
            None,
            &sig,
            &sig,
            message,
            &tree,
            &[&source_commit, &target_commit],
        )?;

        self.repo.reference(
            &format!("refs/heads/{}", source_branch),
            commit,
            true,
            message,
        )?;

        Ok(Merge::Ok(Some(commit)))
    }

    pub fn log(
        &self,
        sort_order: LogOrder,
        source_branch: &str,
        target_branch: Option<&str>,
    ) -> Result<impl Iterator<Item = LogEntry>> {
        let branch = self.branch_commit(source_branch)?;

        let mut revwalk = self.repo.revwalk()?;
        revwalk.push(branch.id())?;

        if let Some(branch) = target_branch {
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

    pub fn revert(
        &self,
        target_branch: &str,
        commit: &str,
        strategy: Option<Strategy>,
        dry_run: bool,
    ) -> Result<Merge> {
        let target = self
            .repo
            .find_branch(target_branch, git2::BranchType::Local)?;
        let head_commit = target.into_reference().peel_to_commit()?;

        let merge_commit = self.repo.find_commit(Oid::from_str(commit)?)?;

        let mainline = match merge_commit.parent_count() {
            0 => {
                return Err(Error::Git(git2::Error::from_str(
                    "Initial root commit cannot be reverted",
                )));
            }
            1 => 0,
            _ => match strategy {
                Some(Strategy::KeepOther) => 1,
                _ => 0,
            },
        };

        let opts = MergeOptions::new();

        let mut idx =
            self.repo
                .revert_commit(&merge_commit, &head_commit, mainline, Some(&opts))?;

        if idx.has_conflicts() {
            let conflicts = conflict_diffs(&self.repo, &idx)?;
            return Ok(Merge::Conflicts(conflicts));
        }

        if dry_run {
            return Ok(Merge::Ok(None));
        }

        let tree_id = idx.write_tree_to(&self.repo)?;
        let tree = self.repo.find_tree(tree_id)?;

        let sig = Signature::now(&self.default_author.name, &self.default_author.email)?;

        let commit = self.repo.commit(
            None,
            &sig,
            &sig,
            &format!("Reverted {}", commit),
            &tree,
            &[&head_commit],
        )?;

        self.repo.reference(
            &format!("refs/heads/{}", target_branch),
            commit,
            true,
            "revert: update branch head",
        )?;

        Ok(Merge::Ok(Some(commit)))
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

    pub fn clean(&self, source_branch: &str, target_branch: &str) -> Result<bool> {
        let source_tree = self.branch_commit(source_branch)?.tree()?;
        let target_tree = self
            .repo
            .find_branch(target_branch, git2::BranchType::Local)?
            .into_reference()
            .peel_to_commit()?
            .tree()?;

        let mut opts = DiffOptions::new();
        let diff =
            self.repo
                .diff_tree_to_tree(Some(&target_tree), Some(&source_tree), Some(&mut opts))?;

        Ok(diff.deltas().len() == 0)
    }

    pub fn blob(&self, commit: &str, path: &str) -> Result<Vec<u8>> {
        let commit = self.repo.find_commit(Oid::from_str(commit)?)?;
        let tree = commit.tree()?;
        let tree_entry = tree.get_path(Path::new(path))?;

        let blob = tree_entry
            .to_object(&self.repo)?
            .into_blob()
            .map_err(|_| Error::Git(git2::Error::from_str("Not a valid blob file")))?;
        Ok(blob.content().to_vec())
    }

    pub fn all_blobs(&self, commit: &str) -> Result<Vec<(PathBuf, Vec<u8>)>> {
        let commit = self.repo.find_commit(Oid::from_str(commit)?)?;
        let tree = commit.tree()?;

        let mut blobs = Vec::new();

        tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if let Some(ObjectType::Blob) = entry.kind()
                && let Ok(name) = entry.name()
            {
                let mut path = PathBuf::from(root);
                path.push(name);

                let blob = match self.repo.find_blob(entry.id()) {
                    Ok(blob) => blob,
                    Err(_) => return TreeWalkResult::Abort,
                };

                blobs.push((path, blob.content().to_vec()));
            }
            TreeWalkResult::Ok
        })?;
        Ok(blobs)
    }

    pub fn graph_status(&self, source_branch: &str, target_branch: &str) -> Result<GraphStatus> {
        let source_commit = self
            .repo
            .resolve_reference_from_short_name(source_branch)?
            .peel_to_commit()?;
        let target_commit = self
            .repo
            .resolve_reference_from_short_name(target_branch)?
            .peel_to_commit()?;

        let common_ancesotr = self
            .repo
            .merge_base(source_commit.id(), target_commit.id())?;

        let (ahead, behind) = self
            .repo
            .graph_ahead_behind(source_commit.id(), target_commit.id())?;

        Ok(GraphStatus {
            common_ancestor_commit: common_ancesotr.to_string(),
            commits_ahead: ahead as i32,
            commits_behind: behind as i32,
        })
    }

    fn build_squash_merge(
        &self,
        source: &git2::Commit<'_>,
        target: &git2::Commit<'_>,
    ) -> Result<String> {
        let mut revwalk = self.repo.revwalk()?;
        revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
        revwalk.push(source.id())?;
        revwalk.hide(target.id())?;

        let mut message = String::from("Merged branch\n");
        for oid in revwalk {
            let oid = oid?;
            if let Ok(commit) = self.repo.find_commit(oid)
                && let Some(summary) = commit.summary()?
            {
                message.push_str(&format!("- {}\n", summary));
            }
        }

        Ok(message)
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
            .our
            .as_ref()
            .or(conflict.their.as_ref())
            .or(conflict.ancestor.as_ref());

        if let Some(entry) = entry {
            let path = bytes2path(&entry.path);

            let contents = match conflict.our.as_ref().or(conflict.their.as_ref()) {
                Some(entry) => repo.find_blob(entry.id)?.content().to_vec(),
                None => Vec::new(),
            };

            let ancestor = object_to_json(repo, &conflict.ancestor)?;
            let ours = object_to_json(repo, &conflict.our)?;
            let theirs = object_to_json(repo, &conflict.their)?;

            diffs.push(ConflictDiff {
                path: path.to_path_buf(),
                contents,
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
    pub contents: Vec<u8>,
    pub ours: json_patch::Patch,
    pub theirs: json_patch::Patch,
}

#[derive(Clone, Copy, Default)]
pub enum LogOrder {
    #[default]
    Normal,
    Reverse,
}

#[derive(Clone, Copy, Default)]
pub enum Strategy {
    #[default]
    KeepMainline,
    KeepOther,
}

#[derive(Debug)]
pub struct Tag {
    pub name: String,
    pub commit: String,
    pub time: Timestamp,
    pub author: Author,
}

#[derive(Debug)]
pub struct GraphStatus {
    pub common_ancestor_commit: String,
    pub commits_ahead: i32,
    pub commits_behind: i32,
}

fn new_index_entry(blob_id: Oid, path: &Path, contents: &[u8]) -> IndexEntry {
    IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o100644,
        uid: 0,
        gid: 0,
        file_size: contents.len() as u32,
        id: blob_id,
        flags: 0,
        flags_extended: 0,
        path: path.as_os_str().as_encoded_bytes().to_vec(),
    }
}
