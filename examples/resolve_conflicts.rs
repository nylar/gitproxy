use std::{
    collections::HashMap,
    iter::zip,
    path::{Path, PathBuf},
};

use buffa::{MessageField, MessageView};
use connectrpc::client::{ClientConfig, HttpClient};
use console::style;
use gitproxy::{
    connect::gitproxy::v1::GitProxyServiceClient,
    proto::gitproxy::v1::{
        CommitAuthor, CommitRequest, ConflictDiff, CreateBranchRequest, CreateRepositoryRequest,
        DeleteRepositoryRequest, File, GraphStatusRequest, ListBlobsRequest,
        ListRepositoriesRequest, MergeRequest, ResolveConflictsRequest,
        commit_request::{Files, Metadata, Payload},
        diff_patch::Operation,
    },
};
use json_patch::{PatchOperation, jsonptr::PointerBuf};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const NAMESPACE: &str = "example";

#[tokio::main]
async fn main() {
    let base_uri = "http://0.0.0.0:3000".parse().unwrap();
    let config = ClientConfig::new(base_uri);
    let client = GitProxyServiceClient::new(HttpClient::plaintext(), config);

    ensure_repo(&client).await;

    let mut menu_foo = Menu {
        id: "foo".to_owned(),
        title: HashMap::from([("en-US".to_owned(), "Foo".to_owned())]),
        page_ids: vec!["bar".to_owned(), "baz".to_owned()],
    };

    let mut page_bar = Page {
        id: "bar".to_owned(),
        title: HashMap::from([("en-US".to_owned(), "Bar".to_owned())]),
        archived: false,
        menu_id: "foo".to_owned(),
    };

    let mut page_baz = Page {
        id: "baz".to_owned(),
        title: HashMap::from([("en-US".to_owned(), "Baz".to_owned())]),
        archived: false,
        menu_id: "foo".to_owned(),
    };

    let requests = commit_request(
        NAMESPACE,
        "main",
        "test",
        "test@example.com",
        "Initial main commit",
        vec![
            (
                Path::new("menus/foo.json").to_path_buf(),
                serde_json::to_vec(&menu_foo).unwrap(),
            ),
            (
                Path::new("pages/bar.json").to_path_buf(),
                serde_json::to_vec(&page_bar).unwrap(),
            ),
            (
                Path::new("pages/baz.json").to_path_buf(),
                serde_json::to_vec(&page_baz).unwrap(),
            ),
        ],
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

    menu_foo.title.insert("en-US".to_owned(), "Quux".to_owned());
    page_bar.title.insert("en-US".to_owned(), "Thud".to_owned());
    page_baz.archived = true;

    let requests = commit_request(
        NAMESPACE,
        branch,
        "test",
        "test@example.com",
        "Change in my-branch",
        vec![
            (
                Path::new("menus/foo.json").to_path_buf(),
                serde_json::to_vec(&menu_foo).unwrap(),
            ),
            (
                Path::new("pages/bar.json").to_path_buf(),
                serde_json::to_vec(&page_bar).unwrap(),
            ),
            (
                Path::new("pages/baz.json").to_path_buf(),
                serde_json::to_vec(&page_baz).unwrap(),
            ),
        ],
    );

    client.commit(requests).await.unwrap();

    menu_foo
        .title
        .insert("en-US".to_owned(), "Plugh".to_owned());

    page_bar
        .title
        .insert("en-US".to_owned(), "Blargh".to_owned());

    let requests = commit_request(
        NAMESPACE,
        "main",
        "test",
        "test@example.com",
        "Change in main",
        vec![
            (
                Path::new("menus/foo.json").to_path_buf(),
                serde_json::to_vec(&menu_foo).unwrap(),
            ),
            (
                Path::new("pages/bar.json").to_path_buf(),
                serde_json::to_vec(&page_bar).unwrap(),
            ),
        ],
    );

    client.commit(requests).await.unwrap();

    let graph_status = client
        .graph_status(GraphStatusRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: "main".to_owned(),
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

    cliclack::clear_screen().unwrap();
    cliclack::intro(
        style(format!(
            " Common ancestor: {} ",
            graph_status.view().common_ancestor_commit
        ))
        .on_blue()
        .black(),
    )
    .unwrap();
    cliclack::intro(
        style(format!(
            " Commits ahead: {} ",
            graph_status.view().commits_ahead
        ))
        .on_green()
        .black(),
    )
    .unwrap();
    cliclack::intro(
        style(format!(
            " Commits behind: {} ",
            graph_status.view().commits_behind
        ))
        .on_red()
        .black(),
    )
    .unwrap();
    cliclack::intro(style(" Resolve conflicts ").on_magenta().black()).unwrap();

    let files = resolve_conflicts(&resp.view().to_owned_message().unwrap().conflicts);

    let resp = client
        .resolve_conflicts(ResolveConflictsRequest {
            namespace: NAMESPACE.to_owned(),
            source_branch: branch.to_owned(),
            target_branch: "main".to_owned(),
            files: files
                .into_iter()
                .map(|f| File {
                    path: f.0.display().to_string(),
                    contents: f.1,
                    ..Default::default()
                })
                .collect(),
            author: MessageField::some(CommitAuthor {
                name: "test".to_owned(),
                email: "test@example.com".to_owned(),
                ..Default::default()
            }),
            message: format!("Resolved conflicts for {}", branch),
            ..Default::default()
        })
        .await
        .unwrap();

    cliclack::outro(
        style(format!(
            " Conflicts resolved! {} ",
            resp.view().commit.unwrap()
        ))
        .on_green()
        .black(),
    )
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

    println!("Merge commit: {}", resp.view().commit.unwrap());

    let resp = client
        .list_blobs(ListBlobsRequest {
            namespace: NAMESPACE.to_owned(),
            commit: resp.view().commit.unwrap().to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    for file in &resp.view().files {
        println!("{} - {}", file.path, String::from_utf8_lossy(file.contents));
    }
}

fn resolve_conflicts(conflicts: &[ConflictDiff]) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();

    for conflict in conflicts {
        let mut value: Value = serde_json::from_slice(&conflict.contents).unwrap();

        cliclack::note(
            format!("Before: {}", conflict.path.to_owned()),
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let mut ops = Vec::new();

        for (ours, theirs) in zip(&conflict.ours, &conflict.theirs) {
            ops.push(prompt_conflict(
                ours.operation.as_ref().unwrap(),
                theirs.operation.as_ref().unwrap(),
            ));
        }

        json_patch::patch(&mut value, &ops).unwrap();

        cliclack::note(
            format!("After: {}", conflict.path.to_owned()),
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let data = serde_json::to_vec(&value).unwrap();

        files.push((Path::new(&conflict.path).to_path_buf(), data));
    }
    files
}

fn title(ours: &PatchOperation) -> String {
    match ours {
        PatchOperation::Add(add_operation) => add_operation.path.to_string(),
        PatchOperation::Remove(remove_operation) => remove_operation.path.to_string(),
        PatchOperation::Replace(replace_operation) => replace_operation.path.to_string(),
        PatchOperation::Move(move_operation) => move_operation.path.to_string(),
        PatchOperation::Copy(copy_operation) => copy_operation.path.to_string(),
        PatchOperation::Test(test_operation) => test_operation.path.to_string(),
    }
}

fn operation(op: &PatchOperation) -> String {
    match op {
        PatchOperation::Add(add_operation) => format!("{}", add_operation.value),
        PatchOperation::Remove(remove_operation) => format!("Remove {}", remove_operation.path),
        PatchOperation::Replace(replace_operation) => format!("{}", replace_operation.value),
        PatchOperation::Move(move_operation) => format!("Move {}", move_operation.from),
        PatchOperation::Copy(copy_operation) => format!("Copy {}", copy_operation.from),
        PatchOperation::Test(test_operation) => format!("{}", test_operation.value),
    }
}

fn prompt_conflict(ours: &Operation, theirs: &Operation) -> PatchOperation {
    let ours = into_op(ours);
    let theirs = into_op(theirs);

    let our_op = operation(&ours);
    let theirs_op = operation(&theirs);

    cliclack::select(title(&ours))
        .item(ours.clone(), our_op, "Ours")
        .item(theirs.clone(), theirs_op, "Theirs")
        .interact()
        .unwrap()
}

#[derive(Debug, Deserialize, Serialize)]
struct Menu {
    id: String,
    title: HashMap<String, String>,
    page_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Page {
    id: String,
    title: HashMap<String, String>,
    archived: bool,
    menu_id: String,
}

fn into_op(op: &Operation) -> PatchOperation {
    match op {
        Operation::Add(add) => PatchOperation::Add(json_patch::AddOperation {
            path: PointerBuf::parse(&add.path).unwrap(),
            value: serde_json::from_str(&add.value).unwrap(),
        }),
        Operation::Remove(remove) => PatchOperation::Remove(json_patch::RemoveOperation {
            path: PointerBuf::parse(&remove.path).unwrap(),
        }),
        Operation::Replace(replace) => PatchOperation::Replace(json_patch::ReplaceOperation {
            path: PointerBuf::parse(&replace.path).unwrap(),
            value: serde_json::from_str(&replace.value).unwrap(),
        }),
        Operation::Move(mov) => PatchOperation::Move(json_patch::MoveOperation {
            path: PointerBuf::parse(&mov.path).unwrap(),
            from: PointerBuf::parse(&mov.from).unwrap(),
        }),
        Operation::Copy(copy) => PatchOperation::Copy(json_patch::CopyOperation {
            path: PointerBuf::parse(&copy.path).unwrap(),
            from: PointerBuf::parse(&copy.from).unwrap(),
        }),
        Operation::Test(test) => PatchOperation::Test(json_patch::TestOperation {
            path: PointerBuf::parse(&test.path).unwrap(),
            value: serde_json::from_str(&test.value).unwrap(),
        }),
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
