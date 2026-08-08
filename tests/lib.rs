use std::{net::SocketAddr, path::Path};

use buffa::MessageField;
use connectrpc::client::{ClientConfig, HttpClient};
use gitproxy::{
    config::Config,
    connect::gitproxy::v1::GitProxyServiceClient,
    proto::gitproxy::v1::{
        CommitAuthor, CommitRequest, CreateBranchRequest, CreateRepositoryRequest,
        CreateTagRequest, DeleteBranchRequest, DeleteRepositoryRequest, DeleteTagRequest,
        ListBranchesRequest, ListRepositoriesRequest, ListTagsRequest, MergeRequest,
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

    client
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
            commit: None,
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

    assert_eq!(resp.view().tags.iter().as_slice(), &[tag]);

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

    let resp = client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let branch_dir = Path::new(resp.view().branch.path);
    std::fs::write(branch_dir.join("my_file.txt"), "foo\nbar\nbaz\n".as_bytes()).unwrap();

    client
        .commit(CommitRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            message: "Added my_file".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "test".to_owned(),
                email: "test@example.com".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

async fn start_server(root_dir: &Path) -> SocketAddr {
    let app = gitproxy::app(&config(root_dir));
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
