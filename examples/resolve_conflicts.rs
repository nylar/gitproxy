use std::{collections::HashMap, iter::zip, path::Path};

use buffa::{MessageField, MessageView};
use connectrpc::client::{ClientConfig, HttpClient};
use console::style;
use gitproxy::{
    connect::gitproxy::v1::GitProxyServiceClient,
    proto::gitproxy::v1::{
        CommitAuthor, CommitRequest, ConflictDiff, CreateBranchRequest, CreateRepositoryRequest,
        DeleteRepositoryRequest, MergeRequest, diff_patch::Operation,
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

    client
        .delete_repository(DeleteRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let resp = client
        .create_repository(CreateRepositoryRequest {
            namespace: NAMESPACE.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let main_dir = Path::new(resp.view().repository.path).join("main");

    std::fs::create_dir_all(&main_dir.join("menus/")).unwrap();
    std::fs::create_dir_all(&main_dir.join("pages/")).unwrap();

    let mut menu_foo = Menu {
        id: "foo".to_owned(),
        title: HashMap::from([("en-US".to_owned(), "Foo".to_owned())]),
        page_ids: vec!["bar".to_owned(), "baz".to_owned()],
    };

    std::fs::write(
        main_dir.join("menus/foo.json"),
        &serde_json::to_vec(&menu_foo).unwrap(),
    )
    .unwrap();

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

    std::fs::write(
        main_dir.join("pages/bar.json"),
        &serde_json::to_vec(&page_bar).unwrap(),
    )
    .unwrap();

    std::fs::write(
        main_dir.join("pages/baz.json"),
        &serde_json::to_vec(&page_baz).unwrap(),
    )
    .unwrap();

    client
        .commit(CommitRequest {
            namespace: NAMESPACE.to_owned(),
            branch: "main".to_owned(),
            message: "Initial comit".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "test".to_owned(),
                email: "test@example.com".to_owned(),
                ..Default::default()
            }),
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

    menu_foo.title.insert("en-US".to_owned(), "Quux".to_owned());

    std::fs::write(
        branch_dir.join("menus/foo.json"),
        &serde_json::to_vec(&menu_foo).unwrap(),
    )
    .unwrap();

    page_bar.title.insert("en-US".to_owned(), "Thud".to_owned());
    page_baz.archived = true;

    std::fs::write(
        branch_dir.join("pages/bar.json"),
        &serde_json::to_vec(&page_bar).unwrap(),
    )
    .unwrap();

    std::fs::write(
        branch_dir.join("pages/baz.json"),
        &serde_json::to_vec(&page_baz).unwrap(),
    )
    .unwrap();

    client
        .commit(CommitRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            message: "Change in my-branch".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "test".to_owned(),
                email: "test@example.com".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    menu_foo
        .title
        .insert("en-US".to_owned(), "Plugh".to_owned());

    std::fs::write(
        main_dir.join("menus/foo.json"),
        &serde_json::to_vec(&menu_foo).unwrap(),
    )
    .unwrap();

    page_bar
        .title
        .insert("en-US".to_owned(), "Blargh".to_owned());

    std::fs::write(
        main_dir.join("pages/bar.json"),
        &serde_json::to_vec(&page_bar).unwrap(),
    )
    .unwrap();

    client
        .commit(CommitRequest {
            namespace: NAMESPACE.to_owned(),
            branch: "main".to_owned(),
            message: "Initial comit".to_owned(),
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
            branch: branch.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    cliclack::clear_screen().unwrap();
    cliclack::intro(style(" Resolve conflicts ").on_magenta().black()).unwrap();

    resolve_conflicts(
        branch_dir,
        &resp.view().to_owned_message().unwrap().conflicts,
    );

    cliclack::outro(style(" Conflicts resolved! ").on_green().black()).unwrap();

    let resp = client
        .commit(CommitRequest {
            namespace: NAMESPACE.to_owned(),
            branch: branch.to_owned(),
            message: "Conflicts resolved".to_owned(),
            author: MessageField::some(CommitAuthor {
                name: "test".to_owned(),
                email: "test@example.com".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    println!("Merge commit: {}", resp.view().commit);
}

fn resolve_conflicts(dir: &Path, conflicts: &[ConflictDiff]) {
    for conflict in conflicts {
        let data = std::fs::read(dir.join(&conflict.path)).unwrap();
        let mut value: Value = serde_json::from_slice(&data).unwrap();

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
        std::fs::write(dir.join(&conflict.path), &data).unwrap();
    }
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
