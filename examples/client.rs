use buffa::MessageField;
use chrono::DateTime;
use connectrpc::client::{ClientConfig, HttpClient};
use gitproxy::{
    connect::gitproxy::v1::GitProxyServiceClient,
    proto::gitproxy::v1::{
        CommitAuthor, CommitFile, CommitFilesRequest, CreateBranchRequest, CreateRepositoryRequest,
        DeleteRepositoryRequest, LogRequest, LogView, MergeRequest, RevertMergeRequest,
    },
};

const NAMESPACE: &str = "example";

#[tokio::main]
async fn main() {
    let base_uri = "http://0.0.0.0:3000".parse().unwrap();
    let config = ClientConfig::new(base_uri);
    let client = GitProxyServiceClient::new(HttpClient::plaintext(), config);

    client
        .delete_repository(DeleteRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Repo created: {}", NAMESPACE);

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

    let resp = client
        .commit_files(CommitFilesRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch1.to_owned(),
            message: "change A".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "Bob".to_owned(),
                email: "bob@example.com".to_owned(),
                ..Default::default()
            }),
            files: vec![
                CommitFile {
                    path: "my-dir/foo.txt".to_owned(),
                    contents: "foo".as_bytes().to_vec(),
                    ..Default::default()
                },
                CommitFile {
                    path: "my-dir/bar.txt".to_owned(),
                    contents: "bar".as_bytes().to_vec(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Commit A: {}", resp.view().commit);

    let resp = client
        .commit_files(CommitFilesRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch1.to_owned(),
            message: "change B".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "Bob".to_owned(),
                email: "bob@example.com".to_owned(),
                ..Default::default()
            }),
            files: vec![CommitFile {
                path: "my-dir/foo.txt".to_owned(),
                contents: "foo2".as_bytes().to_vec(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Commit B: {}", resp.view().commit);

    let resp = client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch1.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch {} merged: {}", branch1, resp.view().commit);

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

    let resp = client
        .commit_files(CommitFilesRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch2.to_owned(),
            message: "change C".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "Bob".to_owned(),
                email: "bob@example.com".to_owned(),
                ..Default::default()
            }),
            files: vec![CommitFile {
                path: "my-dir/foo.txt".to_owned(),
                contents: "foo3".as_bytes().to_vec(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Commit C: {}", resp.view().commit);

    let resp = client
        .merge(MergeRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch2.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch {} merged: {}", branch2, resp.view().commit);

    let head = resp.view().commit.to_owned();

    let resp = client
        .log(LogRequest {
            namespace: NAMESPACE.to_owned(),
            branch: None, // main
            ..Default::default()
        })
        .await
        .unwrap();
    println!("\n--- Changes --- \n");
    for log in &resp.view().logs {
        print_log(log);
    }

    let resp = client
        .revert_merge(RevertMergeRequest {
            namespace: NAMESPACE.to_owned(),
            commit: head.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Commit {} reverted: {}", head, resp.view().commit);

    let resp = client
        .log(LogRequest {
            namespace: NAMESPACE.to_owned(),
            branch: None, // main
            ..Default::default()
        })
        .await
        .unwrap();
    println!("\n--- Changes --- \n");
    for log in &resp.view().logs {
        print_log(log);
    }
}

fn print_log(log: &LogView<'_>) {
    let time = DateTime::from_timestamp_secs(log.time.seconds).unwrap();
    println!(
        "commit {}\nAuthor: {} <{}>\nDate: {}\n\n\t{}\n",
        log.commit, log.author.name, log.author.email, time, log.message
    );
}
