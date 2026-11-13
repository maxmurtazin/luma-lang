use luma::edits::{
    serve, Operand, Operation, Proposal, Workspace, MAX_REQUEST_BYTES, SCHEMA_VERSION,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier, Mutex},
    thread,
};

fn open(source: &str) -> Workspace {
    Workspace::open(
        "main.luma".into(),
        BTreeMap::from([("main.luma".into(), source.into())]),
    )
    .unwrap()
}

fn propose(workspace: &Workspace, operations: Vec<Operation>) -> Proposal {
    Proposal {
        schema_version: SCHEMA_VERSION,
        expected_revision: workspace.snapshot().revision.clone(),
        operations,
    }
}

fn id(workspace: &Workspace, kind: &str) -> String {
    workspace
        .snapshot()
        .ir
        .syntax
        .iter()
        .find(|node| node.kind == kind)
        .unwrap()
        .id
        .clone()
}

fn create(name: &str) -> Operation {
    Operation::CreateValue {
        file: "main.luma".into(),
        name: name.into(),
        value: Operand::Number { value: 42 },
    }
}

#[test]
fn all_operations_and_stable_ids() {
    let original = "# source comment\r\n";
    let mut workspace = open(original);
    let proposal = propose(
        &workspace,
        vec![
            Operation::CreateFunction {
                file: "main.luma".into(),
                name: "f".into(),
            },
            create("input"),
        ],
    );
    let preview = workspace.preview(&proposal).unwrap();
    assert_eq!(workspace.snapshot().ir.files[0].content, original);
    assert_eq!(preview.edits.len(), 2);
    assert_eq!(
        serde_json::to_value(&preview).unwrap(),
        serde_json::to_value(workspace.preview(&proposal).unwrap()).unwrap()
    );
    assert_eq!(
        workspace.commit(&proposal).unwrap().revision,
        preview.candidate.revision
    );
    let function = id(&workspace, "function");
    let proposal = propose(
        &workspace,
        vec![
            Operation::AddParameter {
                function: function.clone(),
                name: "arg".into(),
            },
            Operation::InsertCall {
                file: "main.luma".into(),
                callee: vec!["f".into()],
                arguments: vec![Operand::Number { value: 1 }],
            },
        ],
    );
    workspace.commit(&proposal).unwrap();
    assert_eq!(id(&workspace, "function"), function);
    let call = id(&workspace, "call");
    let proposal = propose(
        &workspace,
        vec![Operation::Connect {
            call,
            argument: 0,
            value: Operand::Name {
                name: "input".into(),
            },
        }],
    );
    workspace.commit(&proposal).unwrap();
    let binding = id(&workspace, "assign");
    let proposal = propose(&workspace, vec![Operation::RemoveNode { node: binding }]);
    assert_eq!(workspace.commit(&proposal).unwrap_err().code, "dependency");
    let proposal = propose(&workspace, vec![create("unused")]);
    workspace.commit(&proposal).unwrap();
    let unused = workspace
        .snapshot()
        .ir
        .syntax
        .iter()
        .find(|node| node.text.as_deref() == Some("unused"))
        .unwrap()
        .parent
        .clone()
        .unwrap();
    let proposal = propose(&workspace, vec![Operation::RemoveNode { node: unused }]);
    workspace.commit(&proposal).unwrap();
    let source = &workspace.snapshot().ir.files[0].content;
    assert!(source.starts_with(original));
    assert!(!source.contains("unused"));
    assert!(!source.replace("\r\n", "").contains('\n'));
}

#[test]
fn invalid_multi_module_transaction_rolls_back_and_graph_matches_disk_checker() {
    let files = BTreeMap::from([
        ("main.luma".into(), "use lib\nlib.inc(1)\n".into()),
        (
            "lib.luma".into(),
            "pub fn inc(x)\nreturn x + 1\nend\n".into(),
        ),
    ]);
    let mut workspace = Workspace::open("main.luma".into(), files).unwrap();
    let before = serde_json::to_value(workspace.snapshot()).unwrap();
    let call = id(&workspace, "call");
    let add = Operation::CreateValue {
        file: "lib.luma".into(),
        name: "extra".into(),
        value: Operand::Bool { value: true },
    };
    let proposal = propose(
        &workspace,
        vec![
            add.clone(),
            Operation::Connect {
                call,
                argument: 0,
                value: Operand::Bool { value: true },
            },
        ],
    );
    let failure = workspace.commit(&proposal).unwrap_err();
    assert_eq!(failure.code, "validation_failed");
    assert!(failure.diagnostics.iter().any(|d| d.code == "E_TYPE"));
    assert_eq!(serde_json::to_value(workspace.snapshot()).unwrap(), before);
    let proposal = propose(&workspace, vec![add, create("more")]);
    let committed = workspace.commit(&proposal).unwrap();
    let root = std::env::temp_dir().join(format!("luma edit test {}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for file in &committed.ir.files {
        std::fs::write(root.join(&file.path), &file.content).unwrap();
    }
    let entry = root.join("main.luma");
    assert_eq!(committed.ir, luma::structural::analyze(&entry));
    assert_eq!(committed.graph, luma::semantic::check(&entry).graph);
    assert!(committed
        .ir
        .edges
        .iter()
        .any(|edge| edge.kind == "resolves_to"
            && edge.from.starts_with("file:main.luma/")
            && edge.to.starts_with("file:lib.luma/")));
    for file in &committed.ir.files {
        std::fs::remove_file(root.join(&file.path)).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn one_concurrent_commit_wins() {
    let workspace = Arc::new(Mutex::new(open("")));
    let barrier = Arc::new(Barrier::new(2));
    let workers = ["first", "second"].map(|name| {
        let workspace = workspace.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            let proposal = propose(&workspace.lock().unwrap(), vec![create(name)]);
            barrier.wait();
            workspace
                .lock()
                .unwrap()
                .commit(&proposal)
                .map(|_| ())
                .map_err(|error| error.code)
        })
    });
    let mut outcomes = workers.map(|worker| worker.join().unwrap());
    outcomes.sort();
    assert_eq!(outcomes, [Ok(()), Err("revision_conflict")]);
}

#[test]
fn json_round_trip_and_limits() {
    let workspace = open("");
    let proposal = propose(&workspace, vec![create("value")]);
    let envelope = |request: Value| json!({"schema_version":1,"request":request});
    let input = [
        envelope(json!({"action":"open","entry":"main.luma","files":{"main.luma":""}})),
        envelope(json!({"action":"preview","proposal":proposal})),
        envelope(json!({"action":"commit","proposal":proposal})),
        envelope(json!({"action":"commit","proposal":proposal})),
    ]
    .map(|value| value.to_string())
    .join("\n");
    let mut output = Vec::new();
    serve(input.as_bytes(), &mut output).unwrap();
    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies[1]["result"]["candidate"], replies[2]["result"]);
    assert_eq!(replies[3]["error"]["code"], "revision_conflict");
    let mut output = Vec::new();
    serve(vec![b' '; MAX_REQUEST_BYTES + 1].as_slice(), &mut output).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&output).unwrap()["error"]["code"],
        "limit"
    );
    let path = "..\\outside.luma".to_owned();
    assert_eq!(
        Workspace::open(path.clone(), BTreeMap::from([(path, String::new())]))
            .err()
            .unwrap()
            .code,
        "invalid_file"
    );
}

#[test]
fn rejects_renames_injection_and_duplicate_replacements() {
    let mut workspace = open("x = 1\nprint(x)\n");
    let original = workspace.snapshot().revision.clone();
    let proposal = propose(&workspace, vec![create("x\nprint(99)")]);
    assert_eq!(
        workspace.commit(&proposal).unwrap_err().code,
        "invalid_identifier"
    );
    let proposal = propose(
        &workspace,
        vec![
            create("y"),
            Operation::RemoveNode {
                node: id(&workspace, "assign"),
            },
        ],
    );
    assert_eq!(workspace.commit(&proposal).unwrap_err().code, "dependency");
    let replacement = Operation::Connect {
        call: id(&workspace, "call"),
        argument: 0,
        value: Operand::Number { value: 2 },
    };
    let proposal = propose(&workspace, vec![replacement.clone(), replacement]);
    assert_eq!(workspace.commit(&proposal).unwrap_err().code, "overlap");
    assert_eq!(workspace.snapshot().revision, original);
}
