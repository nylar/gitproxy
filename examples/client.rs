use std::path::{Path, PathBuf};

use buffa::{MessageField, MessageFieldView};
use connectrpc::client::{ClientConfig, HttpClient};
use gitproxy::{
    connect::gitproxy::v1::GitProxyServiceClient,
    proto::gitproxy::v1::{
        CommitAuthor, CommitRequest, CreateBranchRequest, CreateRepositoryRequest,
        CreateTagRequest, DeleteRepositoryRequest, DiffRequest, DiffView, File,
        ListRepositoriesRequest, LogRequest, LogView, MaintenanceRequest, MergeRequest,
        RevertRequest, StatusRequest,
        commit_request::{Files, Metadata, Payload},
        diff_patch::OperationView,
    },
};
use jiff::Timestamp;
use yansi::Paint;

const NAMESPACE: &str = "example";

#[tokio::main]
async fn main() {
    let base_uri = "http://0.0.0.0:3000".parse().unwrap();
    let config = ClientConfig::new(base_uri);
    let client = GitProxyServiceClient::new(HttpClient::plaintext(), config);

    ensure_repo(&client).await;

    let branch1 = "branch-1";
    client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch1.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch created: {}", branch1);

    let requests = commit_request(
        NAMESPACE,
        branch1,
        "Bob",
        "bob@example.com",
        "change A",
        vec![
            (
                Path::new("my-dir/foo.json").to_path_buf(),
                serde_json::to_vec(&serde_json::json!({
                    "a": "foo"
                }))
                .unwrap(),
            ),
            (
                Path::new("my-dir/bar.json").to_path_buf(),
                serde_json::to_vec(&serde_json::json!({
                    "b": "bar"
                }))
                .unwrap(),
            ),
        ],
    );

    let resp = client.commit(requests).await.unwrap();
    println!("Commit A: {}", resp.view().commit);

    let diff = client
        .diff(DiffRequest {
            namespace: NAMESPACE.to_owned(),
            base_reference: branch1.to_owned(),
            target_reference: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    print_diff(&diff.view().diff);

    let requests = commit_request(
        NAMESPACE,
        branch1,
        "Bob",
        "bob@example.com",
        "change B",
        vec![(
            Path::new("my-dir/foo.json").to_path_buf(),
            serde_json::to_vec(&serde_json::json!({
                "a": "baz"
            }))
            .unwrap(),
        )],
    );

    let resp = client.commit(requests).await.unwrap();
    println!("Commit B: {}", resp.view().commit);

    let diff = client
        .diff(DiffRequest {
            namespace: NAMESPACE.to_owned(),
            base_reference: "main".to_owned(),
            target_reference: branch1.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    print_diff(&diff.view().diff);

    let resp = client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch1.to_owned(),
            target_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch {} merged: {}", branch1, resp.view().commit.unwrap());

    let branch2 = "branch-2";
    client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch2.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch created: {}", branch2);

    println!("Dirty: {}", dirty(&client, NAMESPACE, branch2).await);

    let requests = commit_request(
        NAMESPACE,
        branch2,
        "Bob",
        "bob@example.com",
        "change C",
        vec![(
            Path::new("my-dir/foo.json").to_path_buf(),
            serde_json::to_vec(&serde_json::json!({
                "a": "quux"
            }))
            .unwrap(),
        )],
    );

    let resp = client.commit(requests).await.unwrap();
    println!("Commit C: {}", resp.view().commit);

    println!("Dirty: {}", dirty(&client, NAMESPACE, branch2).await);

    let diff = client
        .diff(DiffRequest {
            namespace: NAMESPACE.to_owned(),
            base_reference: "main".to_owned(),
            target_reference: branch2.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    print_diff(&diff.view().diff);

    let resp = client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch2.to_owned(),
            target_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch {} merged: {}", branch2, resp.view().commit.unwrap());

    let head = resp.view().commit.unwrap().to_owned();

    let resp = client
        .log(LogRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("\n--- Changes --- \n");
    for log in &resp.view().logs {
        print_log(log);
    }

    let resp = client
        .revert(RevertRequest {
            namespace: NAMESPACE.to_owned(),
            target_branch: "main".to_owned(),
            commit: head.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Commit {} reverted: {}", head, resp.view().commit.unwrap());

    let last_commit = resp.view().commit.unwrap().to_owned();

    let resp = client
        .log(LogRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("\n--- Changes --- \n");
    for log in &resp.view().logs {
        print_log(log);
    }

    let tag = "my-tag";
    client
        .create_tag(CreateTagRequest {
            namespace: NAMESPACE.to_owned(),
            name: tag.to_owned(),
            commit: last_commit,
            message: "Tagging my-tag".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "Bob".to_owned(),
                email: "bob@example.com".to_owned(),
                ..Default::default()
            }),
            overwrite: false,
            ..Default::default()
        })
        .await
        .unwrap();

    client
        .maintenance(MaintenanceRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
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

fn print_log(log: &LogView<'_>) {
    let time = Timestamp::from_second(log.time.seconds).unwrap_or_default();
    println!(
        "commit {}\nAuthor: {} <{}>\nDate: {}\n\n{}\n",
        log.commit, log.author.name, log.author.email, time, log.message
    );
}

fn print_diff(diff: &MessageFieldView<DiffView<'_>>) {
    println!("--- DIFF ---");
    for delta in &diff.files {
        println!(
            "{}",
            delta.old_path.or(delta.new_path).unwrap_or_default().bold()
        );
        for patch in &delta.patches {
            if let Some(operation) = &patch.operation {
                match operation {
                    OperationView::Add(patch) => {
                        println!(
                            "{}",
                            format!("ADD path={} value={}", patch.path, patch.value,).green()
                        )
                    }
                    OperationView::Remove(patch) => {
                        println!("{}", format!("REMOVE path={}", patch.path,).red())
                    }
                    OperationView::Replace(patch) => {
                        println!(
                            "{}",
                            format!("REPLACE path={} value={}", patch.path, patch.value,).yellow()
                        )
                    }
                    OperationView::Move(patch) => {
                        println!("MOVE from={} path={}", patch.from, patch.path)
                    }
                    OperationView::Copy(patch) => {
                        println!("COPY from={} path={}", patch.from, patch.path)
                    }
                    OperationView::Test(patch) => {
                        println!("TEST path={} value={}", patch.path, patch.value,)
                    }
                }
            }
        }
    }
    println!("--- DIFF ---");
}

async fn dirty(client: &GitProxyServiceClient<HttpClient>, namespace: &str, branch: &str) -> bool {
    let resp = client
        .status(StatusRequest {
            namespace: namespace.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: "main".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    resp.view().dirty
}

async fn ensure_repo(client: &GitProxyServiceClient<HttpClient>) {
    let resp = client
        .list_repositories(ListRepositoriesRequest {
            ..Default::default()
        })
        .await
        .unwrap();

    for repo in &resp.view().repositories {
        if repo.namespace == NAMESPACE {
            client
                .delete_repository(DeleteRepositoryRequest {
                    namespace: NAMESPACE.to_owned(),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
    }

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}
