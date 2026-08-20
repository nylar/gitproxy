use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use buffa::MessageField;
use connectrpc::client::{ClientConfig, HttpClient};
use gitproxy::{
    config::Config,
    connect::gitproxy::v1::GitProxyServiceClient,
    proto::gitproxy::v1::{
        CommitAuthor, CommitRequest, ConflictDiff, CreateBranchRequest, CreateRepositoryRequest,
        CreateTagRequest, DeleteBranchRequest, DeleteRepositoryRequest, DeleteTagRequest,
        DiffPatch, File, GetBlobRequest, GetBranchRequest, ListBlobsRequest, ListBranchesRequest,
        ListRepositoriesRequest, ListTagsRequest, LogRequest, MergeRequest,
        ResolveConflictsRequest, RevertRequest,
        commit_request::{Files, Metadata, Payload},
        diff_patch::{Operation, Replace},
    },
};

const NAMESPACE: &str = "test";

#[tokio::test]
async fn test_repositories() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    let resp = client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().repository.namespace, NAMESPACE);

    let resp = client
        .list_repositories(ListRepositoriesRequest {
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().repositories.len(), 1);

    client
        .delete_repository(DeleteRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "namespace: value is required")]
async fn test_create_repository_empty_namespace() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: "".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "namespace: value is required")]
async fn test_delete_repository_empty_namespace() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    client
        .delete_repository(DeleteRepositoryRequest {
            namespace: "".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_branches() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let branch = "my-branch";

    let resp = client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().branch.name, branch);

    let resp = client
        .get_branch(GetBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().branch.name, branch);

    let resp = client
        .list_branches(ListBranchesRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().branches.len(), 1);

    client
        .delete_branch(DeleteBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_tags() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    let resp = client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let tag = "v1.0.0";

    client
        .create_tag(CreateTagRequest {
            namespace: NAMESPACE.to_owned(),
            name: tag.to_owned(),
            commit: resp.view().repository.head_commit.to_owned(),
            message: "My first tag".to_owned(),
            overwrite: false,
            author: MessageField::some(CommitAuthor {
                name: "bob".to_owned(),
                email: "bob@example.com".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let resp = client
        .list_tags(ListTagsRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        resp.view().tags.iter().map(|t| t.name).collect::<Vec<_>>(),
        &[tag]
    );

    client
        .delete_tag(DeleteTagRequest {
            namespace: NAMESPACE.to_owned(),
            name: tag.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_branch_merges_successfully() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let branch: &str = "my-branch";

    client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let requests = commit_request(
        NAMESPACE,
        branch,
        "test",
        "test@example.com",
        "Added my_file",
        vec![(
            Path::new("my_file.txt").to_path_buf(),
            "foo\nbar\nbaz\n".as_bytes().to_vec(),
        )],
    );

    client.commit(requests).await.unwrap();

    let resp = client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_ne!(resp.view().commit.unwrap(), "");

    let log = client
        .log(LogRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let most_recent_commit = log.view().logs.first().unwrap();

    assert_eq!(
        most_recent_commit.message,
        format!("Merged branch\n- Added my_file\n")
    );
}

#[tokio::test]
async fn test_branch_merges_yields_conflicts() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let requests = commit_request(
        NAMESPACE,
        "main",
        "test",
        "test@example.com",
        "Initial commit",
        vec![(
            Path::new("my_file.json").to_path_buf(),
            serde_json::to_vec(&serde_json::json!({
                "a": "Initial content"
            }))
            .unwrap(),
        )],
    );

    client.commit(requests).await.unwrap();

    let branch: &str = "my-branch";

    client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let requests = commit_request(
        NAMESPACE,
        branch,
        "test",
        "test@example.com",
        "Change in my-branch",
        vec![(
            Path::new("my_file.json").to_path_buf(),
            serde_json::to_vec(&serde_json::json!({
                "a": "Change from my-branch"
            }))
            .unwrap(),
        )],
    );

    client.commit(requests).await.unwrap();

    let requests = commit_request(
        NAMESPACE,
        "main",
        "test",
        "test@example.com",
        "Change in main",
        vec![(
            Path::new("my_file.json").to_path_buf(),
            serde_json::to_vec(&serde_json::json!({
                "a": "Change from main branch"
            }))
            .unwrap(),
        )],
    );

    client.commit(requests).await.unwrap();

    let resp = client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let expected_diff = vec![ConflictDiff {
        path: "my_file.json".to_owned(),
        contents: serde_json::to_vec(&serde_json::json!({
            "a": "Change from main branch"
        }))
        .unwrap(),
        ours: vec![DiffPatch {
            operation: Some(Operation::Replace(Box::new(Replace {
                path: "/a".to_owned(),
                value: "\"Change from main branch\"".to_owned(),
                ..Default::default()
            }))),
            ..Default::default()
        }],
        theirs: vec![DiffPatch {
            operation: Some(Operation::Replace(Box::new(Replace {
                path: "/a".to_owned(),
                value: "\"Change from my-branch\"".to_owned(),
                ..Default::default()
            }))),
            ..Default::default()
        }],
        ..Default::default()
    }];

    assert_eq!(
        resp.into_view().to_owned_message().unwrap().conflicts,
        expected_diff
    );

    client
        .resolve_conflicts(ResolveConflictsRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: "main".to_owned(),
            files: vec![File {
                path: "my_file.json".to_owned(),
                contents: serde_json::to_vec(&serde_json::json!({
                    "a": "Change from my-branch"
                }))
                .unwrap(),
                ..Default::default()
            }],
            message: "Resolved conflicts".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "test".to_owned(),
                email: "test@example.com".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let resp = client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    resp.view().commit.unwrap();
}

#[tokio::test]
async fn test_merge_reverts_successfully() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let branch: &str = "my-branch";

    client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let requests = commit_request(
        NAMESPACE,
        branch,
        "test",
        "test@example.com",
        "Added my_file",
        vec![(
            Path::new("my_file.txt").to_path_buf(),
            "foo\nbar\nbaz\n".as_bytes().to_vec(),
        )],
    );

    client.commit(requests).await.unwrap();

    let resp = client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let merge_commit = resp.view().commit.unwrap();

    client
        .revert(RevertRequest {
            namespace: NAMESPACE.to_owned(),
            target_branch: "main".to_owned(),
            commit: merge_commit.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let log = client
        .log(LogRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let most_recent_commit = log.view().logs.first().unwrap();

    assert_eq!(
        most_recent_commit.message,
        &format!("Reverted {}", merge_commit)
    );
}

#[tokio::test]
async fn test_commit_reverts_successfully() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let branch: &str = "my-branch";

    client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let requests = commit_request(
        NAMESPACE,
        branch,
        "test",
        "test@example.com",
        "Added my_file",
        vec![(
            Path::new("my_file.txt").to_path_buf(),
            "before".as_bytes().to_vec(),
        )],
    );

    client.commit(requests).await.unwrap();

    let requests = commit_request(
        NAMESPACE,
        branch,
        "test",
        "test@example.com",
        "Updated my_file",
        vec![(
            Path::new("my_file.txt").to_path_buf(),
            "after".as_bytes().to_vec(),
        )],
    );

    let resp = client.commit(requests).await.unwrap();

    let bad_commit = resp.view().commit;

    client
        .revert(RevertRequest {
            namespace: NAMESPACE.to_owned(),
            commit: bad_commit.to_owned(),
            target_branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let log = client
        .log(LogRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let most_recent_commit = log.view().logs.first().unwrap();

    assert_eq!(
        most_recent_commit.message,
        &format!("Reverted {}", bad_commit)
    );

    let resp = client
        .get_blob(GetBlobRequest {
            namespace: NAMESPACE.to_owned(),
            path: "my_file.txt".to_owned(),
            commit: most_recent_commit.commit.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().file.contents, "before".as_bytes())
}

#[tokio::test]
async fn test_log_with_parent_includes_only_branch_changes() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let branch: &str = "my-branch";

    client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let requests = commit_request(
        NAMESPACE,
        branch,
        "test",
        "test@example.com",
        "Added my_file",
        vec![(
            Path::new("my_file.txt").to_path_buf(),
            "before".as_bytes().to_vec(),
        )],
    );

    client.commit(requests).await.unwrap();

    let log = client
        .log(LogRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: Some("main".to_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        &log.view()
            .logs
            .iter()
            .map(|e| e.message.to_owned())
            .collect::<Vec<_>>(),
        &["Added my_file".to_owned()]
    );
}

#[tokio::test]
async fn test_blobs() {
    let root_dir = tempfile::tempdir().unwrap();
    let addr = start_server(root_dir.path()).await;
    let client = make_client(&addr);

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let requests = commit_request(
        NAMESPACE,
        "main",
        "test",
        "test@example.com",
        "Added some files",
        vec![
            (
                Path::new("foo.txt").to_path_buf(),
                "foo".as_bytes().to_vec(),
            ),
            (
                Path::new("bar.txt").to_path_buf(),
                "bar".as_bytes().to_vec(),
            ),
            (
                Path::new("baz/quux.txt").to_path_buf(),
                "quux".as_bytes().to_vec(),
            ),
        ],
    );

    let resp = client.commit(requests).await.unwrap();

    let commit = resp.view().commit.to_owned();

    let resp = client
        .get_blob(GetBlobRequest {
            namespace: NAMESPACE.to_owned(),
            path: "bar.txt".to_owned(),
            commit: commit.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().file.contents, "bar".as_bytes());

    let resp = client
        .get_blob(GetBlobRequest {
            namespace: NAMESPACE.to_owned(),
            path: "baz/quux.txt".to_owned(),
            commit: commit.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().file.contents, "quux".as_bytes());

    let resp = client
        .list_blobs(ListBlobsRequest {
            namespace: NAMESPACE.to_owned(),
            commit: commit.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(resp.view().files.len(), 3);
}

async fn start_server(root_dir: &Path) -> SocketAddr {
    let app = gitproxy::app(&config(root_dir)).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn make_client(addr: &SocketAddr) -> GitProxyServiceClient<HttpClient> {
    let config = ClientConfig::new(format!("http://{addr}").parse().unwrap());
    GitProxyServiceClient::new(HttpClient::plaintext(), config)
}

fn config(root_dir: &Path) -> Config {
    Config {
        port: 0,
        root_dir: root_dir.to_path_buf(),
        git_user_name: "test".to_owned(),
        git_user_email: "test@example.com".to_owned(),
    }
}

fn commit_request(
    namespace: &str,
    branch: &str,
    author_name: &str,
    author_email: &str,
    message: &str,
    files: Vec<(PathBuf, Vec<u8>)>,
) -> impl IntoIterator<Item = CommitRequest> {
    let mut requests = vec![CommitRequest {
        payload: Some(Payload::Metadata(Box::new(Metadata {
            namespace: namespace.to_owned(),
            branch: branch.to_owned(),
            message: message.to_owned(),
            author: MessageField::some(CommitAuthor {
                name: author_name.to_owned(),
                email: author_email.to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        }))),
        ..Default::default()
    }];

    for (path, contents) in files {
        requests.push(CommitRequest {
            payload: Some(Payload::Files(Box::new(Files {
                files: vec![File {
                    path: path.display().to_string(),
                    contents: contents.to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            }))),
            ..Default::default()
        })
    }

    requests.into_iter()
}
