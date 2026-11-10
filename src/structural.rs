//! Read-only, versioned semantic snapshot. No evaluation or new typing rules.
use crate::{Kind, Node};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Type {
    Number,
    String,
    Bool,
    Unit,
    Array {
        element: Box<Type>,
    },
    Function {
        parameters: Vec<Type>,
        result: Box<Type>,
    },
    Namespace {
        target: String,
    },
    Unresolved,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    pub id: String,
    pub path: String,
    /// Exact UTF-8 source is the collision-free content identity in schema v1.
    pub content: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyntaxNode {
    pub id: String,
    pub kind: String,
    pub parent: Option<String>,
    pub location: Location,
    pub text: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticNode {
    pub id: String,
    pub kind: String,
    pub syntax: Option<String>,
    pub location: Option<Location>,
    pub ty: Option<Type>,
    pub public: Option<bool>,
    /// Only owner nodes are classified; absence is not a purity claim.
    pub effect: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub location: Location,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub schema_version: u32,
    pub producer: String,
    pub valid: bool,
    pub files: Vec<SourceFile>,
    pub syntax: Vec<SyntaxNode>,
    pub nodes: Vec<SemanticNode>,
    pub edges: Vec<Edge>,
    pub diagnostics: Vec<Diagnostic>,
}
/// Checks one entry and its sibling imports. Never executes source. Invalid
/// documents retain source/diagnostics, but expose no consumable typed nodes.
pub fn analyze(path: impl AsRef<Path>) -> Document {
    crate::semantic::check_structural(path.as_ref())
}
impl Document {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
fn escape(s: &str) -> String {
    s.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}
fn label(n: &Node) -> (&'static str, Option<String>) {
    match &n.kind {
        Kind::Name(x) => ("name", Some(x.clone())),
        Kind::Number(x) => ("number", Some(x.clone())),
        Kind::String(x) => ("string", Some(x.clone())),
        Kind::Bool(x) => ("bool", Some(x.to_string())),
        Kind::Unary(x, _) => ("unary", Some(x.clone())),
        Kind::Binary(x, _, _) => (
            if x == "->" { "pipeline" } else { "binary" },
            Some(x.clone()),
        ),
        Kind::Array(_) => ("array", None),
        Kind::Group(_) => ("group", None),
        Kind::Call(..) => ("call", None),
        Kind::Index(..) => ("index", None),
        Kind::Member(..) => ("member", None),
        Kind::Assign(..) => ("assign", None),
        Kind::Return(_) => ("return", None),
        Kind::Function { .. } => ("function", None),
        Kind::Object { .. } => ("object", None),
        Kind::Use(_) => ("use", None),
        Kind::Public(_) => ("public", None),
        Kind::Match { .. } => ("match", None),
        Kind::Arm { .. } => ("arm", None),
        Kind::Wildcard => ("wildcard", None),
    }
}
fn anchor(n: &Node) -> String {
    let n = if let Kind::Public(x) = &n.kind { x } else { n };
    let named = match &n.kind {
        Kind::Function { name, .. } | Kind::Object { name, .. } => Some(name.as_ref()),
        Kind::Assign(left, _) => Some(left.as_ref()),
        Kind::Use(x) => Some(x.as_ref()),
        _ => None,
    };
    if let Some(Node {
        kind: Kind::Name(name),
        ..
    }) = named
    {
        format!("{}-{}", label(n).0, escape(name))
    } else {
        label(n).0.into()
    }
}
fn body(nodes: &[Node], parent: &str, file: &str, out: &mut Vec<SyntaxNode>) {
    let mut counts = BTreeMap::new();
    for n in nodes {
        let key = anchor(n);
        let count = counts.entry(key.clone()).or_insert(0);
        let id = format!("{parent}/{key}~{count}");
        *count += 1;
        walk(n, id, Some(parent.into()), file, out);
    }
}
fn walk(n: &Node, id: String, parent: Option<String>, file: &str, out: &mut Vec<SyntaxNode>) {
    let (kind, text) = label(n);
    out.push(SyntaxNode {
        id: id.clone(),
        kind: kind.into(),
        parent,
        location: Location {
            file: file.into(),
            start: n.span.start,
            end: n.span.end,
        },
        text,
    });
    let mut children: Vec<(&str, &Node)> = Vec::new();
    match &n.kind {
        Kind::Array(xs) => {
            for (i, x) in xs.iter().enumerate() {
                walk(x, format!("{id}/element-{i}"), Some(id.clone()), file, out);
            }
        }
        Kind::Group(x) | Kind::Public(x) | Kind::Use(x) | Kind::Unary(_, x) => {
            children.push(("value", x))
        }
        Kind::Binary(_, a, b) | Kind::Assign(a, b) | Kind::Index(a, b) | Kind::Member(a, b) => {
            children.push(("left", a));
            children.push(("right", b));
        }
        Kind::Call(c, args) => {
            children.push(("callee", c));
            for (i, a) in args.iter().enumerate() {
                walk(a, format!("{id}/argument-{i}"), Some(id.clone()), file, out);
            }
        }
        Kind::Return(Some(x)) => children.push(("value", x)),
        Kind::Function {
            name,
            params,
            body: xs,
        } => {
            children.push(("name", name));
            for (i, p) in params.iter().enumerate() {
                walk(
                    p,
                    format!("{id}/parameter-{i}"),
                    Some(id.clone()),
                    file,
                    out,
                );
            }
            body(xs, &id, file, out);
        }
        Kind::Object { name, body: xs } => {
            children.push(("name", name));
            body(xs, &id, file, out);
        }
        Kind::Match { subject, arms } => {
            children.push(("subject", subject));
            for (i, a) in arms.iter().enumerate() {
                walk(a, format!("{id}/arm-{i}"), Some(id.clone()), file, out);
            }
        }
        Kind::Arm { pattern, value } => {
            children.push(("pattern", pattern));
            children.push(("value", value));
        }
        _ => (),
    }
    for (role, c) in children {
        walk(c, format!("{id}/{role}"), Some(id.clone()), file, out);
    }
}

pub(crate) fn build(
    sources: &BTreeMap<String, String>,
    programs: &BTreeMap<String, Vec<Node>>,
    graph: &Value,
    diagnostics: &[(String, crate::Diagnostic)],
) -> Document {
    let files = sources
        .iter()
        .map(|(path, content)| SourceFile {
            id: format!("file:{path}"),
            path: path.clone(),
            content: content.clone(),
        })
        .collect();
    let diagnostics: Vec<_> = diagnostics
        .iter()
        .map(|(file, d)| Diagnostic {
            code: d.code.into(),
            message: d.message.clone(),
            location: Location {
                file: format!("file:{file}"),
                start: d.span.start,
                end: d.span.end,
            },
        })
        .collect();
    let mut doc = Document {
        schema_version: SCHEMA_VERSION,
        producer: format!("luma/{}", env!("CARGO_PKG_VERSION")),
        valid: diagnostics.is_empty(),
        files,
        syntax: vec![],
        nodes: vec![],
        edges: vec![],
        diagnostics,
    };
    if !doc.valid {
        return doc;
    }
    for (file, nodes) in programs {
        body(
            nodes,
            &format!("file:{file}"),
            &format!("file:{file}"),
            &mut doc.syntax,
        );
    }
    let mut locations: BTreeMap<(String, usize, usize), Vec<&SyntaxNode>> = BTreeMap::new();
    for n in &doc.syntax {
        locations
            .entry((n.location.file.clone(), n.location.start, n.location.end))
            .or_default()
            .push(n);
    }
    let mut ids = BTreeMap::new();
    for n in graph["nodes"].as_array().unwrap() {
        let old = n["id"].as_str().unwrap();
        let kind = n["kind"].as_str().unwrap();
        let location = n["start"].as_u64().map(|start| Location {
            file: format!("file:{}", n["file"].as_str().unwrap()),
            start: start as usize,
            end: n["end"].as_u64().unwrap() as usize,
        });
        let syntax = location
            .as_ref()
            .and_then(|l| locations.get(&(l.file.clone(), l.start, l.end)))
            .and_then(|xs| {
                xs.iter()
                    .find(|x| kind == "expression" || x.kind == "name")
                    .or_else(|| xs.first())
            })
            .map(|n| n.id.clone());
        let id = syntax
            .as_ref()
            .map(|s| format!("{s}@{kind}"))
            .unwrap_or_else(|| old.into());
        ids.insert(old.to_owned(), id.clone());
        doc.nodes.push(SemanticNode {
            id,
            kind: kind.into(),
            syntax,
            location,
            public: n["public"].as_bool(),
            ty: n
                .get("structuralType")
                .map(|v| serde_json::from_value(v.clone()).expect("checker type contract")),
            effect: n["effect"].as_str().map(str::to_owned),
        });
    }
    for n in &mut doc.nodes {
        if let Some(t) = &mut n.ty {
            remap_type(t, &ids);
        }
    }
    let edges: BTreeSet<_> = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| Edge {
            from: ids[e["from"].as_str().unwrap()].clone(),
            to: ids[e["to"].as_str().unwrap()].clone(),
            kind: e["kind"].as_str().unwrap().into(),
        })
        .collect();
    doc.edges = edges.into_iter().collect();
    doc.syntax.sort_by(|a, b| a.id.cmp(&b.id));
    doc.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    doc
}
fn remap_type(t: &mut Type, ids: &BTreeMap<String, String>) {
    match t {
        Type::Namespace { target } => {
            if let Some(id) = ids.get(target) {
                *target = id.clone();
            }
        }
        Type::Array { element } => remap_type(element, ids),
        Type::Function { parameters, result } => {
            for p in parameters {
                remap_type(p, ids);
            }
            remap_type(result, ids);
        }
        _ => (),
    }
}
