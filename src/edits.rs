//! Validated transactions on a closed, caller-supplied source workspace.
use crate::{lex, parse, semantic, structural, Kind, Node, Span};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[path = "edits_disk.rs"]
mod disk;
pub use disk::DiskWorkspace;

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: usize = 128 * 1024;
const MAX_FILES: usize = 16;
const MAX_SOURCE_BYTES: usize = 32 * 1024;
const MAX_TOKENS: usize = 2048;
const MAX_OPERATIONS: usize = 32;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operand {
    Number { value: i32 },
    Bool { value: bool },
    Name { name: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    CreateFunction {
        file: String,
        name: String,
    },
    AddParameter {
        function: String,
        name: String,
    },
    CreateValue {
        file: String,
        name: String,
        value: Operand,
    },
    InsertCall {
        file: String,
        callee: Vec<String>,
        arguments: Vec<Operand>,
    },
    Connect {
        call: String,
        argument: usize,
        value: Operand,
    },
    RemoveNode {
        node: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub schema_version: u32,
    pub expected_revision: String,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    pub operation: Option<usize>,
    pub actual_revision: Option<String>,
    pub diagnostics: Vec<structural::Diagnostic>,
}

impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            operation: None,
            actual_revision: None,
            diagnostics: vec![],
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub revision: String,
    pub ir: structural::Document,
    pub graph: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct SourceEdit {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub base_revision: String,
    pub candidate: Snapshot,
    pub edits: Vec<SourceEdit>,
}

/// Publish snapshots under an exclusive Rust borrow. Share through a Mutex
/// when several threads need to operate on the same workspace.
pub struct Workspace {
    entry: String,
    files: BTreeMap<String, String>,
    generation: u64,
    snapshot: Snapshot,
}

impl Workspace {
    pub fn open(entry: String, files: BTreeMap<String, String>) -> Result<Self, Failure> {
        if !files.contains_key(&entry) {
            return Err(Failure::new(
                "missing_entry",
                "entry must be supplied in files",
            ));
        }
        let snapshot = analyze(&entry, &files, 0)?;
        Ok(Self {
            entry,
            files,
            generation: 0,
            snapshot,
        })
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn preview(&self, proposal: &Proposal) -> Result<Preview, Failure> {
        self.prepare(proposal).map(|(preview, _, _)| preview)
    }

    pub fn commit(&mut self, proposal: &Proposal) -> Result<&Snapshot, Failure> {
        let (preview, files, generation) = self.prepare(proposal)?;
        self.files = files;
        self.generation = generation;
        self.snapshot = preview.candidate;
        Ok(&self.snapshot)
    }

    fn prepare(
        &self,
        proposal: &Proposal,
    ) -> Result<(Preview, BTreeMap<String, String>, u64), Failure> {
        if proposal.schema_version != SCHEMA_VERSION {
            return Err(Failure::new(
                "unsupported_version",
                "expected schema_version 1",
            ));
        }
        if proposal.expected_revision != self.snapshot.revision {
            let mut failure = Failure::new("revision_conflict", "workspace changed since proposal");
            failure.actual_revision = Some(self.snapshot.revision.clone());
            return Err(failure);
        }
        if !(1..=MAX_OPERATIONS).contains(&proposal.operations.len()) {
            return Err(Failure::new("limit", "expected 1 to 32 operations"));
        }
        let mut edits = Vec::new();
        let mut created = BTreeSet::new();
        for (index, operation) in proposal.operations.iter().enumerate() {
            let result = self.plan(operation, &mut created);
            edits.push(result.map_err(|mut error| {
                error.operation = Some(index);
                error
            })?);
        }
        // Stable sorting keeps same-offset appends in proposal order.
        edits.sort_by(|a, b| (&a.file, a.start, a.end).cmp(&(&b.file, b.start, b.end)));
        for pair in edits.windows(2) {
            let [a, b] = pair else { unreachable!() };
            if a.file == b.file && a.end > b.start {
                return Err(Failure::new(
                    "overlap",
                    "operations overlap in the base source",
                ));
            }
        }
        let mut files = self.files.clone();
        for edit in edits.iter().rev() {
            files
                .get_mut(&edit.file)
                .unwrap()
                .replace_range(edit.start..edit.end, &edit.replacement);
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Failure::new("limit", "revision sequence exhausted"))?;
        let candidate = analyze(&self.entry, &files, generation)?;
        Ok((
            Preview {
                base_revision: self.snapshot.revision.clone(),
                candidate,
                edits,
            },
            files,
            generation,
        ))
    }

    fn node(&self, id: &str, kind: &str) -> Result<&structural::SyntaxNode, Failure> {
        self.snapshot
            .ir
            .syntax
            .iter()
            .find(|node| node.id == id && (kind.is_empty() || node.kind == kind))
            .ok_or_else(|| Failure::new("invalid_target", "syntax ID absent or of the wrong kind"))
    }

    fn source(&self, file: &str) -> Result<&str, Failure> {
        self.snapshot
            .ir
            .files
            .iter()
            .find(|source| source.path == file)
            .map(|source| source.content.as_str())
            .ok_or_else(|| {
                Failure::new("invalid_target", "file is outside the checked import graph")
            })
    }

    fn plan(
        &self,
        operation: &Operation,
        created: &mut BTreeSet<(String, String)>,
    ) -> Result<SourceEdit, Failure> {
        match operation {
            Operation::CreateFunction { file, name }
            | Operation::CreateValue { file, name, .. } => {
                identifier(name)?;
                let source = self.source(file)?;
                if lex(source)
                    .unwrap()
                    .iter()
                    .any(|token| token.value.is_none() && token.text == *name)
                    || !created.insert((file.clone(), name.clone()))
                {
                    return Err(Failure::new(
                        "name_in_use",
                        "new definition requires a fresh name in its file",
                    ));
                }
                let nl = newline(source);
                let statement = match operation {
                    Operation::CreateValue { value, .. } => format!("{name} = {}", operand(value)?),
                    _ => format!("fn {name}(){nl}end"),
                };
                Ok(append(file, source, &statement))
            }
            Operation::InsertCall {
                file,
                callee,
                arguments,
            } => {
                if !(1..=2).contains(&callee.len()) || arguments.len() > 16 {
                    return Err(Failure::new(
                        "limit",
                        "expected 1 or 2 callee components and at most 16 arguments",
                    ));
                }
                for name in callee {
                    identifier(name)?;
                }
                let args = arguments
                    .iter()
                    .map(operand)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(append(
                    file,
                    self.source(file)?,
                    &format!("{}({})", callee.join("."), args.join(", ")),
                ))
            }
            Operation::AddParameter { function, name } => {
                identifier(name)?;
                let node = self.node(function, "function")?;
                let (file, span) = location(node);
                let source = self.source(file)?;
                let tokens = lex(&source[span.start..span.end]).unwrap();
                if tokens
                    .iter()
                    .any(|token| token.value.is_none() && token.text == *name)
                    || !created.insert((function.clone(), name.clone()))
                {
                    return Err(Failure::new(
                        "name_in_use",
                        "parameter must be fresh in the function",
                    ));
                }
                let close = tokens.iter().position(|token| token.text == ")").unwrap();
                let start = span.start + tokens[close].span.start;
                // All targets are from the base revision. Only one parameter
                // can be added to a given function in one transaction.
                if !created.insert((function.clone(), String::new())) {
                    return Err(Failure::new(
                        "overlap",
                        "only one parameter addition per function per proposal",
                    ));
                }
                let prefix = if close == 3 { "" } else { ", " };
                Ok(edit(file, start, start, format!("{prefix}{name}")))
            }
            Operation::Connect {
                call,
                argument,
                value,
            } => {
                self.node(call, "call")?;
                let target = self.node(&format!("{call}/argument-{argument}"), "")?;
                if !matches!(target.kind.as_str(), "name" | "number" | "bool" | "string") {
                    return Err(Failure::new(
                        "invalid_target",
                        "connect only replaces simple argument expressions",
                    ));
                }
                let (file, span) = location(target);
                Ok(edit(file, span.start, span.end, operand(value)?))
            }
            Operation::RemoveNode { node } => self.remove(node),
        }
    }

    fn remove(&self, id: &str) -> Result<SourceEdit, Failure> {
        let node = self.node(id, "")?;
        let (file, span) = location(node);
        let source = self.source(file)?;
        let program = parse(source).unwrap();
        let Some(statement) = program.iter().find(|node| node.span == span) else {
            return Err(Failure::new(
                "invalid_target",
                "removal is limited to whole top-level statements",
            ));
        };
        let definition = match &statement.kind {
            Kind::Function { name, .. } => name,
            Kind::Assign(left, right)
                if matches!(left.kind, Kind::Name(_))
                    && matches!(
                        right.kind,
                        Kind::Number(_) | Kind::Bool(_) | Kind::String(_)
                    ) =>
            {
                left
            }
            _ => {
                return Err(Failure::new(
                    "invalid_target",
                    "remove supports private top-level functions and bindings",
                ))
            }
        };
        let Kind::Name(name) = &definition.kind else {
            unreachable!()
        };
        // The checker models reads and calls but not all writes. Reject every
        // same-file mention outside the removal to prevent silent rebinding.
        if lex(source).unwrap().iter().any(|token| {
            token.value.is_none()
                && token.text == *name
                && (token.span.start < span.start || token.span.end > span.end)
        }) {
            return Err(Failure::new(
                "dependency",
                "definition has a possible dependent outside the removed statement",
            ));
        }
        Ok(edit(file, span.start, span.end, String::new()))
    }
}

fn location(node: &structural::SyntaxNode) -> (&str, Span) {
    let loc = &node.location;
    (
        loc.file.strip_prefix("file:").unwrap(),
        Span {
            start: loc.start,
            end: loc.end,
        },
    )
}

fn edit(file: &str, start: usize, end: usize, replacement: String) -> SourceEdit {
    SourceEdit {
        file: file.into(),
        start,
        end,
        replacement,
    }
}

fn newline(source: &str) -> &str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn append(file: &str, source: &str, statement: &str) -> SourceEdit {
    let nl = newline(source);
    let prefix = if source.is_empty() || source.ends_with('\n') {
        ""
    } else {
        nl
    };
    edit(
        file,
        source.len(),
        source.len(),
        format!("{prefix}{statement}{nl}"),
    )
}

fn identifier(name: &str) -> Result<(), Failure> {
    if name.len() <= 64
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && matches!(
            parse(&format!("fn {name}()\nend")).as_deref(),
            Ok([Node {
                kind: Kind::Function { .. },
                ..
            }])
        )
    {
        Ok(())
    } else {
        Err(Failure::new(
            "invalid_identifier",
            "expected a Luma identifier of at most 64 ASCII bytes",
        ))
    }
}

fn operand(value: &Operand) -> Result<String, Failure> {
    Ok(match value {
        Operand::Number { value } => value.to_string(),
        Operand::Bool { value } => value.to_string(),
        Operand::Name { name } => {
            identifier(name)?;
            name.clone()
        }
    })
}

fn valid_file(file: &str) -> bool {
    file.strip_suffix(".luma")
        .is_some_and(|stem| identifier(stem).is_ok())
        && !file.bytes().any(|b| b.is_ascii_uppercase())
}

fn analyze(
    entry: &str,
    files: &BTreeMap<String, String>,
    generation: u64,
) -> Result<Snapshot, Failure> {
    if files.len() > MAX_FILES || files.values().map(String::len).sum::<usize>() > MAX_SOURCE_BYTES
    {
        return Err(Failure::new(
            "limit",
            "workspace exceeds 16 files or 32 KiB of UTF-8 source",
        ));
    }
    let mut token_count = 0;
    for (file, source) in files {
        if !valid_file(file) {
            return Err(Failure::new(
                "invalid_file",
                "expected a lowercase sibling .luma filename",
            ));
        }
        let tokens = lex(source).map_err(|d| {
            invalid(vec![structural::Diagnostic {
                code: d.code.into(),
                message: d.message,
                location: structural::Location {
                    file: format!("file:{file}"),
                    start: d.span.start,
                    end: d.span.end,
                },
            }])
        })?;
        token_count += tokens.len();
    }
    if token_count > MAX_TOKENS {
        return Err(Failure::new("limit", "workspace exceeds 2048 tokens"));
    }
    let (report, ir) = semantic::check_snapshot(entry, files);
    if !ir.valid {
        return Err(invalid(ir.diagnostics));
    }
    let bytes = serde_json::to_vec(&(SCHEMA_VERSION, generation, entry, files)).unwrap();
    let revision = format!("s2v1:{:x}", Sha256::digest(bytes));
    Ok(Snapshot {
        revision,
        ir,
        graph: report.graph,
    })
}

fn invalid(diagnostics: Vec<structural::Diagnostic>) -> Failure {
    let mut failure = Failure::new("validation_failed", "source failed Luma validation");
    failure.diagnostics = diagnostics;
    failure
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Open {
        entry: String,
        files: BTreeMap<String, String>,
    },
    OpenDisk {
        entry: String,
        modules: Vec<String>,
    },
    Snapshot,
    Preview {
        proposal: Proposal,
    },
    Commit {
        proposal: Proposal,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema_version: u32,
    request: Request,
}

enum Session {
    Memory(Workspace),
    Disk(DiskWorkspace),
}

fn dispatch(
    workspace: &mut Option<Session>,
    root: Option<&Path>,
    bytes: &[u8],
) -> Result<serde_json::Value, Failure> {
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|e| Failure::new("invalid_request", e.to_string()))?;
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(Failure::new(
            "unsupported_version",
            "expected schema_version 1",
        ));
    }
    if matches!(
        &envelope.request,
        Request::Open { .. } | Request::OpenDisk { .. }
    ) {
        if workspace.is_some() {
            return Err(Failure::new(
                "already_open",
                "start a new session to open another workspace",
            ));
        }
        let session = match envelope.request {
            Request::Open { entry, files } => Session::Memory(Workspace::open(entry, files)?),
            Request::OpenDisk { entry, modules } => {
                let root = root.ok_or_else(|| {
                    Failure::new(
                        "disk_disabled",
                        "start luma edit --workspace to allow disk access",
                    )
                })?;
                Session::Disk(DiskWorkspace::open(root, entry, modules)?)
            }
            _ => unreachable!(),
        };
        let snapshot = match &session {
            Session::Memory(workspace) => serde_json::json!(workspace.snapshot()),
            Session::Disk(workspace) => serde_json::json!(workspace.snapshot()),
        };
        *workspace = Some(session);
        return Ok(snapshot);
    }
    let workspace = workspace
        .as_mut()
        .ok_or_else(|| Failure::new("not_open", "open a workspace first"))?;
    Ok(match (workspace, envelope.request) {
        (Session::Memory(w), Request::Snapshot) => serde_json::json!(w.snapshot()),
        (Session::Disk(w), Request::Snapshot) => serde_json::json!(w.snapshot()),
        (Session::Memory(w), Request::Preview { proposal }) => {
            serde_json::json!(w.preview(&proposal)?)
        }
        (Session::Disk(w), Request::Preview { proposal }) => {
            serde_json::json!(w.preview(&proposal)?)
        }
        (Session::Memory(w), Request::Commit { proposal }) => {
            serde_json::json!(w.commit(&proposal)?)
        }
        (Session::Disk(w), Request::Commit { proposal }) => serde_json::json!(w.commit(&proposal)?),
        (_, Request::Open { .. } | Request::OpenDisk { .. }) => unreachable!(),
    })
}

/// Run a JSON Lines session. An oversized frame returns a structured error
/// and closes the session without reading the rest of that frame.
pub fn serve(reader: impl std::io::BufRead, writer: impl std::io::Write) -> std::io::Result<()> {
    serve_workspace(reader, writer, None)
}

/// Enable disk operations only within the directory chosen by the host.
pub fn serve_workspace(
    mut reader: impl std::io::BufRead,
    mut writer: impl std::io::Write,
    root: Option<&Path>,
) -> std::io::Result<()> {
    use std::io::{BufRead, Read};
    let mut workspace = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = (&mut reader)
            .take((MAX_REQUEST_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            return Ok(());
        }
        let too_large = count > MAX_REQUEST_BYTES;
        let result = if too_large {
            Err(Failure::new("limit", "request frame exceeds 128 KiB"))
        } else {
            dispatch(&mut workspace, root, &line)
        };
        let reply = match result {
            Ok(result) => serde_json::json!({ "schema_version": SCHEMA_VERSION, "result": result }),
            Err(error) => serde_json::json!({ "schema_version": SCHEMA_VERSION, "error": error }),
        };
        serde_json::to_writer(&mut writer, &reply)?;
        writeln!(writer)?;
        writer.flush()?;
        if too_large {
            return Ok(());
        }
    }
}
