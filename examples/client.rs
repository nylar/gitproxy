use std::path::Path;

use buffa::{MessageField, MessageFieldView};
use connectrpc::client::{ClientConfig, HttpClient};
use gitproxy::{
    connect::gitproxy::v1::GitProxyServiceClient,
    error::Result,
    proto::gitproxy::v1::{
        CheckoutTagRequest, CommitAuthor, CommitRequest, CreateBranchRequest,
        CreateRepositoryRequest, CreateTagRequest, DeleteBranchRequest, DeleteRepositoryRequest,
        DiffRequest, DiffView, LogRequest, LogView, MergeRequest, RevertMergeRequest,
        StatusRequest, diff_patch::OperationView,
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
    let branch_path = client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch1.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch created: {}", branch1);

    write_files(
        branch_path.view().branch.path,
        vec![
            (
                &Path::new("my-dir/foo.json"),
                &serde_json::to_vec(&serde_json::json!({
                    "a": "foo"
                }))
                .unwrap(),
            ),
            (
                &Path::new("my-dir/bar.json"),
                &serde_json::to_vec(&serde_json::json!({
                    "b": "bar"
                }))
                .unwrap(),
            ),
        ],
    )
    .unwrap();

    let resp = client
        .commit(CommitRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch1.to_owned(),
            message: "change A".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "Bob".to_owned(),
                email: "bob@example.com".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Commit A: {}", resp.view().commit);

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

    write_files(
        branch_path.view().branch.path,
        vec![(
            &Path::new("my-dir/foo.json"),
            &serde_json::to_vec(&serde_json::json!({
                "a": "baz"
            }))
            .unwrap(),
        )],
    )
    .unwrap();

    let resp = client
        .commit(CommitRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch1.to_owned(),
            message: "change B".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "Bob".to_owned(),
                email: "bob@example.com".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
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
            branch: branch1.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch {} merged: {}", branch1, resp.view().commit.unwrap());

    let branch2 = "branch-2";
    let branch_path = client
        .create_branch(CreateBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch2.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch created: {}", branch2);

    write_files(
        branch_path.view().branch.path,
        vec![(
            &Path::new("my-dir/foo.json"),
            &serde_json::to_vec(&serde_json::json!({
                "a": "quux"
            }))
            .unwrap(),
        )],
    )
    .unwrap();

    println!("Dirty: {}", dirty(&client, NAMESPACE, Some(branch2)).await);

    let resp = client
        .commit(CommitRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch2.to_owned(),
            message: "change C".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "Bob".to_owned(),
                email: "bob@example.com".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Commit C: {}", resp.view().commit);

    println!("Dirty: {}", dirty(&client, NAMESPACE, Some(branch2)).await);

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
            branch: branch2.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("Branch {} merged: {}", branch2, resp.view().commit.unwrap());

    let head = resp.view().commit.unwrap().to_owned();

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

    let tag = "my-tag";
    client
        .create_tag(CreateTagRequest {
            namespace: NAMESPACE.to_owned(),
            name: tag.to_owned(),
            commit: None,
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

    let resp = client
        .checkout_tag(CheckoutTagRequest {
            namespace: NAMESPACE.to_owned(),
            name: tag.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    println!("Checked out tag {} to {}", tag, resp.view().path);

    client
        .delete_branch(DeleteBranchRequest {
            namespace: NAMESPACE.to_owned(),
            branch: tag.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

fn print_log(log: &LogView<'_>) {
    let time = Timestamp::from_second(log.time.seconds).unwrap_or_default();
    println!(
        "commit {}\nAuthor: {} <{}>\nDate: {}\n\n{}\n",
        log.commit, log.author.name, log.author.email, time, log.message
    );
}

fn write_files(root_path: &str, files: Vec<(&Path, &[u8])>) -> Result<()> {
    let root_path = Path::new(root_path);

    for (path, contents) in files {
        let path = root_path.join(path);
        println!("Writing file to {}", path.to_string_lossy());
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, contents)?;
    }

    Ok(())
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

async fn dirty(
    client: &GitProxyServiceClient<HttpClient>,
    namespace: &str,
    branch: Option<&str>,
) -> bool {
    let resp = client
        .status(StatusRequest {
            namespace: namespace.to_owned(),
            branch: branch.map(|b| b.to_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    resp.view().dirty
}
