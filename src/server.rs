use std::path::PathBuf;

use buffa::MessageField;
use buffa_types::Timestamp;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use tokio::task::spawn_blocking;

use crate::{
    config::Config,
    connect::gitproxy::v1::GitProxyService,
    error::Error,
    proto::gitproxy::v1::{
        Branch, CheckoutTagRequest, CheckoutTagResponse, CommitAuthor, CommitRequest,
        CommitResponse, CreateBranchRequest, CreateBranchResponse, CreateRepositoryRequest,
        CreateRepositoryResponse, CreateTagRequest, CreateTagResponse, DeleteBranchRequest,
        DeleteBranchResponse, DeleteRepositoryRequest, DeleteRepositoryResponse, DeleteTagRequest,
        DeleteTagResponse, Diff, DiffRequest, DiffResponse, GetBranchRequest, GetBranchResponse,
        GetRepositoryRequest, GetRepositoryResponse, ListBranchesRequest, ListBranchesResponse,
        ListRepositoriesRequest, ListRepositoriesResponse, ListTagsRequest, ListTagsResponse, Log,
        LogRequest, LogResponse, MergeRequest, MergeResponse, Repository, RevertCommitRequest,
        RevertCommitResponse, RevertMergeRequest, RevertMergeResponse, StatusRequest,
        StatusResponse, log_request::Order,
    },
    repository::{Author, LogOrder},
    service::Service,
};

pub struct Server {
    root_dir: PathBuf,
    default_author: Author,
}

impl Server {
    pub fn new(config: &Config) -> Self {
        Self {
            root_dir: config.root_dir.to_owned(),
            default_author: Author {
                name: config.git_user_name.to_owned(),
                email: config.git_user_email.to_owned(),
            },
        }
    }

    fn service(&self, namespace: &str) -> Service {
        let repo_dir = self.root_dir.join(namespace);
        Service::new(repo_dir, self.default_author.clone())
    }

    fn service_with_repo_dir(&self, repo_dir: PathBuf) -> Service {
        Service::new(repo_dir, self.default_author.clone())
    }

    fn repo_dir(&self, namespace: &str) -> PathBuf {
        self.root_dir.join(namespace)
    }
}

#[allow(refining_impl_trait)]
#[protovalidate_buffa::connect_impl]
impl GitProxyService for Server {
    async fn list_repositories(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListRepositoriesRequest>,
    ) -> ServiceResult<ListRepositoriesResponse> {
        let mut repositories = Vec::new();

        for dir in std::fs::read_dir(&self.root_dir)? {
            let dir = dir?;
            let service = self.service_with_repo_dir(self.root_dir.join(dir.path()));

            let head = spawn_blocking(move || service.fetch_repository_head_commit())
                .await
                .map_err(Error::TokioTask)??;

            repositories.push(crate::proto::gitproxy::v1::Repository {
                namespace: dir.file_name().into_string().unwrap(),
                head_commit: head.to_string(),
                path: self.root_dir.join(dir.path()).to_str().unwrap().to_string(),
                ..Default::default()
            });
        }

        Response::ok(ListRepositoriesResponse {
            repositories,
            ..Default::default()
        })
    }

    async fn get_repository(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetRepositoryRequest>,
    ) -> ServiceResult<GetRepositoryResponse> {
        let service = self.service(request.namespace);

        let head = spawn_blocking(move || service.fetch_repository_head_commit())
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(GetRepositoryResponse {
            repository: MessageField::some(Repository {
                namespace: request.namespace.to_string(),
                head_commit: head.to_string(),
                path: self.repo_dir(request.namespace).display().to_string(),
                ..Default::default()
            }),
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
        let service = self.service_with_repo_dir(repo_dir);

        let head = spawn_blocking(move || service.fetch_repository_head_commit())
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(CreateRepositoryResponse {
            repository: MessageField::some(crate::proto::gitproxy::v1::Repository {
                namespace: request.namespace.to_string(),
                head_commit: head.to_string(),
                path: self
                    .repo_dir(request.namespace)
                    .to_str()
                    .unwrap()
                    .to_string(),
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
        let service = self.service(request.namespace);

        spawn_blocking(move || service.remove_repository())
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(DeleteRepositoryResponse {
            ..Default::default()
        })
    }

    async fn list_branches(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListBranchesRequest>,
    ) -> ServiceResult<ListBranchesResponse> {
        let service = self.service(request.namespace);

        let branches = spawn_blocking(move || service.list_branches())
            .await
            .map_err(Error::TokioTask)??;

        let branches = branches
            .iter()
            .map(|b| Branch {
                name: b.to_owned(),
                path: self
                    .repo_dir(request.namespace)
                    .join(b)
                    .to_str()
                    .unwrap()
                    .to_string(),
                ..Default::default()
            })
            .collect();

        Response::ok(ListBranchesResponse {
            branches,
            ..Default::default()
        })
    }

    async fn get_branch(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetBranchRequest>,
    ) -> ServiceResult<GetBranchResponse> {
        let service = self.service(request.namespace);

        let branches = spawn_blocking(move || service.list_branches())
            .await
            .map_err(Error::TokioTask)??;

        if !branches.iter().any(|b| b == request.branch) {
            return Err(ConnectError::not_found(format!(
                "{} was not found",
                request.branch
            )));
        }

        Response::ok(GetBranchResponse {
            branch: MessageField::some(Branch {
                name: request.branch.to_owned(),
                path: self
                    .repo_dir(request.namespace)
                    .join(request.branch)
                    .to_str()
                    .unwrap()
                    .to_string(),
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    async fn create_branch(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CreateBranchRequest>,
    ) -> ServiceResult<CreateBranchResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        spawn_blocking(move || service.create_branch(req.branch))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(CreateBranchResponse {
            branch: MessageField::some(Branch {
                name: request.branch.to_owned(),
                path: self
                    .repo_dir(request.namespace)
                    .join(request.branch)
                    .to_str()
                    .unwrap()
                    .to_string(),
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    async fn delete_branch(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteBranchRequest>,
    ) -> ServiceResult<DeleteBranchResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        spawn_blocking(move || service.delete_branch(req.branch))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(DeleteBranchResponse {
            ..Default::default()
        })
    }

    async fn list_tags(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListTagsRequest>,
    ) -> ServiceResult<ListTagsResponse> {
        let service = self.service(request.namespace);

        let tags = spawn_blocking(move || service.list_tags())
            .await
            .map_err(Error::TokioTask)??;

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
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        spawn_blocking(move || {
            service.create_tag(
                req.name,
                req.message,
                req.commit,
                Author {
                    name: req.author.name.to_owned(),
                    email: req.author.email.to_owned(),
                },
                req.overwrite,
            )
        })
        .await
        .map_err(Error::TokioTask)??;

        Response::ok(CreateTagResponse {
            ..Default::default()
        })
    }

    async fn delete_tag(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteTagRequest>,
    ) -> ServiceResult<DeleteTagResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        spawn_blocking(move || service.delete_tag(req.name))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(DeleteTagResponse {
            ..Default::default()
        })
    }

    async fn checkout_tag(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CheckoutTagRequest>,
    ) -> ServiceResult<CheckoutTagResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        let path = spawn_blocking(move || service.checkout_tag(req.name))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(CheckoutTagResponse {
            path: path.to_str().unwrap().to_string(),
            ..Default::default()
        })
    }

    async fn commit(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CommitRequest>,
    ) -> ServiceResult<CommitResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        let commit = spawn_blocking(move || {
            service.commit(
                req.branch,
                req.message,
                Author {
                    name: req.author.name.to_owned(),
                    email: req.author.email.to_owned(),
                },
            )
        })
        .await
        .map_err(Error::TokioTask)??;

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
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        let merge = spawn_blocking(move || service.merge(req.branch, req.dry_run))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(merge.into())
    }

    async fn log(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, LogRequest>,
    ) -> ServiceResult<LogResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        let order = match request.order.as_known() {
            Some(Order::ORDER_REVERSE) => LogOrder::Reverse,
            _ => LogOrder::Normal,
        };

        let entries = spawn_blocking(move || service.log(req.branch, order, req.parent_branch))
            .await
            .map_err(Error::TokioTask)??;

        let entries = entries
            .into_iter()
            .map(|entry| Log {
                message: entry.message,
                author: MessageField::some(CommitAuthor {
                    name: entry.author.name,
                    email: entry.author.email,
                    ..Default::default()
                }),
                commit: entry.commit,
                time: MessageField::some(Timestamp::from_unix_secs(entry.time.as_second())),
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
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        let commit = spawn_blocking(move || service.revert_merge(req.commit))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(RevertMergeResponse {
            commit: commit.to_string(),
            ..Default::default()
        })
    }

    async fn diff(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DiffRequest>,
    ) -> ServiceResult<DiffResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        let diff = spawn_blocking(move || service.diff(req.base_reference, req.target_reference))
            .await
            .map_err(Error::TokioTask)??;

        let files = diff.iter().map(Into::into).collect();

        Response::ok(DiffResponse {
            diff: MessageField::some(Diff {
                files,
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    async fn status(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, StatusRequest>,
    ) -> ServiceResult<StatusResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        let clean = spawn_blocking(move || service.status(req.branch))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(StatusResponse {
            dirty: !clean,
            ..Default::default()
        })
    }

    async fn revert_commit(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, RevertCommitRequest>,
    ) -> ServiceResult<RevertCommitResponse> {
        let req = request.to_owned_message();
        let service = self.service(request.namespace);

        let commit = spawn_blocking(move || service.revert_commit(req.branch, req.commit))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(RevertCommitResponse {
            commit: commit.to_string(),
            ..Default::default()
        })
    }
}
