use std::{path::PathBuf, sync::Arc};

use buffa::MessageField;
use buffa_types::Timestamp;
use connectrpc::{
    ConnectError, InboundStream, RequestContext, Response, ServiceRequest, ServiceResult,
};
use dashmap::DashMap;
use futures::StreamExt;
use protovalidate_buffa::Validate;
use tokio::{
    sync::{RwLock, mpsc},
    task::spawn_blocking,
};

use crate::{
    config::Config,
    connect::gitproxy::v1::GitProxyService,
    error::{Error, Result},
    proto::gitproxy::v1::{
        BlameRequest, BlameResponse, Branch, CommitAuthor, CommitRequest, CommitResponse,
        CreateBranchRequest, CreateBranchResponse, CreateRepositoryRequest,
        CreateRepositoryResponse, CreateTagRequest, CreateTagResponse, DeleteBranchRequest,
        DeleteBranchResponse, DeleteRepositoryRequest, DeleteRepositoryResponse, DeleteTagRequest,
        DeleteTagResponse, Diff, DiffRequest, DiffResponse, File, GetBlobRequest, GetBlobResponse,
        GetBranchRequest, GetBranchResponse, GetRepositoryRequest, GetRepositoryResponse,
        GraphStatusRequest, GraphStatusResponse, ListBlobsRequest, ListBlobsResponse,
        ListBranchesRequest, ListBranchesResponse, ListRepositoriesRequest,
        ListRepositoriesResponse, ListTagsRequest, ListTagsResponse, Log, LogRequest, LogResponse,
        MaintenanceRequest, MaintenanceResponse, MergeRequest, MergeResponse, Repository,
        ResolveConflictsRequest, ResolveConflictsResponse, RevertRequest, RevertResponse,
        StatusRequest, StatusResponse,
        commit_request::{Metadata, Payload},
        log_request::Order,
    },
    repository::{Author, LogOrder},
    service::{ReadService, WriteService},
};

pub struct Server {
    root_dir: PathBuf,
    default_author: Author,
    repo_locks: Arc<DashMap<String, Arc<RwLock<()>>>>,
}

impl Server {
    pub async fn new(config: &Config) -> Result<Self> {
        let repo_locks = Arc::new(DashMap::new());

        let mut entries = tokio::fs::read_dir(&config.root_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir()
                && let Some(repo_id) = path.file_name().and_then(|s| s.to_str())
                && path.join(".git/HEAD").exists()
            {
                repo_locks.insert(repo_id.to_string(), Arc::new(RwLock::new(())));
            }
        }

        Ok(Self {
            root_dir: config.root_dir.to_owned(),
            default_author: Author {
                name: config.git_user_name.to_owned(),
                email: config.git_user_email.to_owned(),
            },
            repo_locks,
        })
    }

    async fn write_service(&self, namespace: &str) -> WriteService {
        let lock_ref = self.repo_locks.get(namespace).unwrap();
        let lock_arc = lock_ref.value().clone();
        let guard = lock_arc.write_owned().await;

        WriteService::new(self.repo_dir(namespace), self.default_author.clone(), guard)
    }

    async fn read_service(&self, namespace: &str) -> ReadService {
        let lock_ref = self.repo_locks.get(namespace).unwrap();
        let lock_arc = lock_ref.value().clone();
        let guard = lock_arc.read_owned().await;

        ReadService::new(self.repo_dir(namespace), self.default_author.clone(), guard)
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

            if let Some(namespace) = dir.path().file_name() {
                let service = self.read_service(&namespace.display().to_string()).await;

                let head = spawn_blocking(move || service.fetch_repository_head_commit())
                    .await
                    .map_err(Error::TokioTask)??;

                repositories.push(crate::proto::gitproxy::v1::Repository {
                    namespace: dir.file_name().into_string().unwrap(),
                    head_commit: head.to_string(),
                    ..Default::default()
                });
            }
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
        let service = self.read_service(request.namespace).await;

        let head = spawn_blocking(move || service.fetch_repository_head_commit())
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(GetRepositoryResponse {
            repository: MessageField::some(Repository {
                namespace: request.namespace.to_string(),
                head_commit: head.to_string(),
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
        self.repo_locks
            .insert(request.namespace.to_owned(), Arc::new(RwLock::new(())));

        let service = self.read_service(request.namespace).await;

        let head = spawn_blocking(move || service.fetch_repository_head_commit())
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(CreateRepositoryResponse {
            repository: MessageField::some(crate::proto::gitproxy::v1::Repository {
                namespace: request.namespace.to_string(),
                head_commit: head.to_string(),
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
        let service = self.write_service(request.namespace).await;

        spawn_blocking(move || service.remove_repository())
            .await
            .map_err(Error::TokioTask)??;

        self.repo_locks.remove(request.namespace);

        Response::ok(DeleteRepositoryResponse {
            ..Default::default()
        })
    }

    async fn list_branches(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListBranchesRequest>,
    ) -> ServiceResult<ListBranchesResponse> {
        let service = self.read_service(request.namespace).await;

        let branches = spawn_blocking(move || service.list_branches())
            .await
            .map_err(Error::TokioTask)??;

        let branches = branches
            .iter()
            .map(|b| Branch {
                name: b.to_owned(),
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
        let service = self.read_service(request.namespace).await;

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
        let service = self.write_service(request.namespace).await;

        spawn_blocking(move || service.create_branch(req.branch))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(CreateBranchResponse {
            branch: MessageField::some(Branch {
                name: request.branch.to_owned(),
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
        let service = self.write_service(request.namespace).await;

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
        let service = self.read_service(request.namespace).await;

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
        let service = self.write_service(request.namespace).await;

        let tag = spawn_blocking(move || {
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
            tag: MessageField::some(tag),
            ..Default::default()
        })
    }

    async fn delete_tag(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteTagRequest>,
    ) -> ServiceResult<DeleteTagResponse> {
        let req = request.to_owned_message();
        let service = self.write_service(request.namespace).await;

        spawn_blocking(move || service.delete_tag(req.name))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(DeleteTagResponse {
            ..Default::default()
        })
    }

    async fn commit(
        &self,
        _ctx: RequestContext,
        mut requests: InboundStream<CommitRequest>,
    ) -> ServiceResult<CommitResponse> {
        let (tx, mut rx) = mpsc::channel::<Vec<File>>(32);

        let mut metadata = Metadata::default();

        if let Some(Ok(first)) = requests.next().await {
            let payload = first.to_owned_message().payload;

            if let Some(Payload::Metadata(m)) = payload {
                metadata = *m.to_owned();
            }
        }

        metadata.validate().map_err(Error::Validation)?;

        let service = self.write_service(&metadata.namespace).await;

        let worker = spawn_blocking(move || service.commit(metadata, &mut rx));

        while let Some(Ok(request)) = requests.next().await {
            let payload = request.to_owned_message().payload;

            if let Some(Payload::Files(files)) = payload
                && !files.files.is_empty()
                && tx.send(files.files).await.is_err()
            {
                break;
            }
        }

        drop(tx);

        let commit = worker.await.map_err(Error::TokioTask)??;

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
        let service = self.write_service(request.namespace).await;

        let merge = spawn_blocking(move || {
            service.merge(req.source_branch, req.target_branch, req.dry_run)
        })
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
        let service = self.read_service(request.namespace).await;

        let order = match request.order.as_known() {
            Some(Order::ORDER_REVERSE) => LogOrder::Reverse,
            _ => LogOrder::Normal,
        };

        let entries =
            spawn_blocking(move || service.log(req.source_branch, order, req.target_branch))
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

    async fn revert(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, RevertRequest>,
    ) -> ServiceResult<RevertResponse> {
        let req = request.to_owned_message();
        let service = self.write_service(request.namespace).await;

        let merge = spawn_blocking(move || {
            let strategy = match req.strategy {
                Some(s) => s.as_known(),
                _ => Some(crate::proto::gitproxy::v1::revert_request::Strategy::Unspecified),
            };

            service.revert_merge(req.target_branch, req.commit, strategy, req.dry_run)
        })
        .await
        .map_err(Error::TokioTask)??;

        Response::ok(merge.into())
    }

    async fn diff(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DiffRequest>,
    ) -> ServiceResult<DiffResponse> {
        let req = request.to_owned_message();
        let service = self.read_service(request.namespace).await;

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
        let service = self.read_service(request.namespace).await;

        let clean = spawn_blocking(move || service.status(req.source_branch, req.target_branch))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(StatusResponse {
            dirty: !clean,
            ..Default::default()
        })
    }

    async fn get_blob(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetBlobRequest>,
    ) -> ServiceResult<GetBlobResponse> {
        let req = request.to_owned_message();
        let service = self.read_service(request.namespace).await;

        let blob = spawn_blocking(move || service.get_blob(req.commit, req.path))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(GetBlobResponse {
            file: MessageField::some(blob),
            ..Default::default()
        })
    }

    async fn list_blobs(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListBlobsRequest>,
    ) -> ServiceResult<ListBlobsResponse> {
        let req = request.to_owned_message();
        let service = self.read_service(request.namespace).await;

        let blobs = spawn_blocking(move || service.list_blobs(req.commit))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(ListBlobsResponse {
            files: blobs,
            ..Default::default()
        })
    }

    async fn resolve_conflicts(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ResolveConflictsRequest>,
    ) -> ServiceResult<ResolveConflictsResponse> {
        let req = request.to_owned_message();
        let service = self.write_service(request.namespace).await;

        let merge = spawn_blocking(move || {
            service.resolve_conflicts(
                req.source_branch,
                req.target_branch,
                req.files,
                Author {
                    name: req.author.name.to_owned(),
                    email: req.author.email.to_owned(),
                },
                req.message,
            )
        })
        .await
        .map_err(Error::TokioTask)??;

        Response::ok(merge.into())
    }

    async fn maintenance(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, MaintenanceRequest>,
    ) -> ServiceResult<MaintenanceResponse> {
        let service = self.write_service(request.namespace).await;

        spawn_blocking(move || service.maintenance())
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(MaintenanceResponse {
            ..Default::default()
        })
    }

    async fn graph_status(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GraphStatusRequest>,
    ) -> ServiceResult<GraphStatusResponse> {
        let req = request.to_owned_message();
        let service = self.read_service(request.namespace).await;

        let graph_status =
            spawn_blocking(move || service.graph_status(req.source_branch, req.target_branch))
                .await
                .map_err(Error::TokioTask)??;

        Response::ok(GraphStatusResponse {
            common_ancestor_commit: graph_status.common_ancestor_commit,
            commits_ahead: graph_status.commits_ahead,
            commits_behind: graph_status.commits_behind,
            ..Default::default()
        })
    }

    async fn blame(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, BlameRequest>,
    ) -> ServiceResult<BlameResponse> {
        let req = request.to_owned_message();
        let service = self.read_service(request.namespace).await;

        let hunks = spawn_blocking(move || service.blame(req.path, req.reference))
            .await
            .map_err(Error::TokioTask)??;

        Response::ok(BlameResponse {
            hunks: hunks.into_iter().map(Into::into).collect(),
            ..Default::default()
        })
    }
}
