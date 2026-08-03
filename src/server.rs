use std::path::PathBuf;

use buffa::MessageField;
use buffa_types::Timestamp;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};

use crate::{
    connect::gitproxy::v1::GitProxyService,
    proto::gitproxy::v1::{
        CommitAuthor, CommitRequest, CommitResponse, CreateBranchRequest, CreateBranchResponse,
        CreateRepositoryRequest, CreateRepositoryResponse, CreateTagRequest, CreateTagResponse,
        DeleteBranchRequest, DeleteBranchResponse, DeleteRepositoryRequest,
        DeleteRepositoryResponse, DeleteTagRequest, DeleteTagResponse, ListBranchesRequest,
        ListBranchesResponse, ListRepositoriesRequest, ListRepositoriesResponse, ListTagsRequest,
        ListTagsResponse, Log, LogRequest, LogResponse, MergeRequest, MergeResponse,
        RevertMergeRequest, RevertMergeResponse,
    },
    repository::{Author, MergePreference, Repository},
};

pub struct Server {
    root_dir: PathBuf,
}

impl Server {
    pub fn new(root_dir: PathBuf) -> Self {
        Self { root_dir }
    }

    fn repo_dir(&self, namespace: &str) -> PathBuf {
        self.root_dir.join(namespace)
    }
}

#[allow(refining_impl_trait)]
impl GitProxyService for Server {
    async fn list_repositories(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListRepositoriesRequest>,
    ) -> ServiceResult<ListRepositoriesResponse> {
        let mut repositories = Vec::new();

        for dir in std::fs::read_dir(&self.root_dir).map_err(internal)? {
            let dir = dir.map_err(internal)?;

            let repo_dir = self.root_dir.join(dir.path());

            let repo = Repository::open(&repo_dir).map_err(internal)?;
            let main = repo.primary_worktree().map_err(internal)?;
            let head = main
                .head()
                .map_err(internal)?
                .peel_to_commit()
                .map_err(internal)?;

            repositories.push(crate::proto::gitproxy::v1::Repository {
                namespace: dir.file_name().into_string().unwrap(),
                head_commit: head.id().to_string(),
                path: repo_dir.to_str().unwrap().to_string(),
                ..Default::default()
            });
        }

        Response::ok(ListRepositoriesResponse {
            repositories,
            ..Default::default()
        })
    }

    async fn create_repository(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CreateRepositoryRequest>,
    ) -> ServiceResult<CreateRepositoryResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        if std::fs::exists(&repo_dir)? {
            return Err(ConnectError::already_exists(format!(
                "repo {} already exists",
                request.namespace,
            )));
        }

        let repo = Repository::open(&repo_dir).map_err(internal)?;
        let main = repo.primary_worktree().map_err(internal)?;
        let head = main
            .head()
            .map_err(internal)?
            .peel_to_commit()
            .map_err(internal)?;

        Response::ok(CreateRepositoryResponse {
            repository: MessageField::some(crate::proto::gitproxy::v1::Repository {
                namespace: request.namespace.to_string(),
                head_commit: head.id().to_string(),
                path: repo_dir.to_str().unwrap().to_string(),
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    async fn delete_repository(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteRepositoryRequest>,
    ) -> ServiceResult<DeleteRepositoryResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;
        repo.remove().map_err(internal)?;

        Response::ok(DeleteRepositoryResponse {
            ..Default::default()
        })
    }

    async fn list_branches(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListBranchesRequest>,
    ) -> ServiceResult<ListBranchesResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;
        let main = repo.primary_worktree().map_err(internal)?;

        let branches = main.list_branches().map_err(internal)?;

        Response::ok(ListBranchesResponse {
            branches,
            ..Default::default()
        })
    }

    async fn create_branch(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CreateBranchRequest>,
    ) -> ServiceResult<CreateBranchResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;
        let main = repo.primary_worktree().map_err(internal)?;

        let branches = main.list_branches().map_err(internal)?;

        if branches.iter().any(|b| b == request.branch) {
            return Err(ConnectError::already_exists("branch {} already exists"));
        }

        main.new(request.branch).map_err(internal)?;

        Response::ok(CreateBranchResponse {
            path: repo_dir.join(request.branch).to_str().unwrap().to_string(),
            ..Default::default()
        })
    }

    async fn delete_branch(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteBranchRequest>,
    ) -> ServiceResult<DeleteBranchResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;
        let main = repo.primary_worktree().map_err(internal)?;
        let branch = repo.worktree(request.branch).map_err(internal)?;
        branch.remove().map_err(internal)?;
        main.delete_branch(request.branch).map_err(internal)?;

        Response::ok(DeleteBranchResponse {
            ..Default::default()
        })
    }

    async fn list_tags(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListTagsRequest>,
    ) -> ServiceResult<ListTagsResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;
        let main = repo.primary_worktree().map_err(internal)?;

        let tags = main.list_tags().map_err(internal)?;

        Response::ok(ListTagsResponse {
            tags,
            ..Default::default()
        })
    }

    async fn create_tag(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CreateTagRequest>,
    ) -> ServiceResult<CreateTagResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let author = Author {
            name: request.author.name.to_owned(),
            email: request.author.email.to_owned(),
        };

        let repo = Repository::open(&repo_dir).map_err(internal)?;
        let main = repo.primary_worktree().map_err(internal)?;

        main.create_tag(
            request.name,
            &author,
            request.message,
            request.commit,
            request.overwrite,
        )
        .map_err(internal)?;

        Response::ok(CreateTagResponse {
            ..Default::default()
        })
    }

    async fn delete_tag(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteTagRequest>,
    ) -> ServiceResult<DeleteTagResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;
        let main = repo.primary_worktree().map_err(internal)?;
        main.delete_tag(request.name).map_err(internal)?;

        Response::ok(DeleteTagResponse {
            ..Default::default()
        })
    }

    async fn commit(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CommitRequest>,
    ) -> ServiceResult<CommitResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;

        let branch = repo.worktree(request.branch).map_err(internal)?;

        let commit = branch
            .commit_all(
                request.message,
                &Author {
                    name: request.author.name.to_owned(),
                    email: request.author.email.to_owned(),
                },
            )
            .map_err(internal)?;

        Response::ok(CommitResponse {
            commit: commit.to_string(),
            ..Default::default()
        })
    }

    async fn merge(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, MergeRequest>,
    ) -> ServiceResult<MergeResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;

        let main = repo.primary_worktree().map_err(internal)?;
        let branch = repo.worktree(request.branch).map_err(internal)?;

        let head = branch.head().map_err(internal)?;
        let commit = main
            .merge(&head, MergePreference::Normal)
            .map_err(internal)?;
        branch.remove().map_err(internal)?;
        main.delete_branch(request.branch).map_err(internal)?;

        Response::ok(MergeResponse {
            commit: commit.to_string(),
            ..Default::default()
        })
    }

    async fn log(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, LogRequest>,
    ) -> ServiceResult<LogResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;

        let branch = match request.branch {
            Some(branch) => repo.worktree(branch).map_err(internal)?,
            None => repo.primary_worktree().map_err(internal)?,
        };

        let entries = branch
            .log()
            .map_err(internal)?
            .map(|entry| Log {
                message: entry.message,
                author: MessageField::some(CommitAuthor {
                    name: entry.author.name,
                    email: entry.author.email,
                    ..Default::default()
                }),
                commit: entry.commit,
                time: MessageField::some(Timestamp::from_unix_secs(entry.time.timestamp())),
                ..Default::default()
            })
            .collect();

        Response::ok(LogResponse {
            logs: entries,
            ..Default::default()
        })
    }

    async fn revert_merge(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, RevertMergeRequest>,
    ) -> ServiceResult<RevertMergeResponse> {
        let repo_dir = self.repo_dir(request.namespace);

        let repo = Repository::open(&repo_dir).map_err(internal)?;

        let main = repo.primary_worktree().map_err(internal)?;
        let oid = main
            .revert(request.commit, MergePreference::Normal)
            .map_err(internal)?;

        Response::ok(RevertMergeResponse {
            commit: oid.to_string(),
            ..Default::default()
        })
    }
}

fn internal<E: std::error::Error>(err: E) -> ConnectError {
    tracing::error!(error = err.to_string());
    ConnectError::internal(err.to_string())
}
