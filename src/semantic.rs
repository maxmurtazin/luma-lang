//! Bounded, whole-import-graph monomorphic constraint checker. No execution.
use crate::{error, parse, position, Diagnostic, Kind, Node, Span};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Var(usize),
    Number,
    String,
    Bool,
    Unit,
    Array(Box<Ty>),
    Function(Vec<Ty>, Box<Ty>),
    Scope(usize),
}
#[derive(Clone)]
struct Binding {
    ty: Ty,
    id: String,
    ready: bool,
    public: bool,
    function: bool,
}
#[derive(Default)]
struct Scope {
    parent: Option<usize>,
    names: BTreeMap<String, Binding>,
    module: bool,
}
pub struct Report {
    pub runtime_gates: Vec<(String, Diagnostic)>,
    pub programs: BTreeMap<PathBuf, Vec<Node>>,
    pub diagnostics: Vec<(String, Diagnostic)>,
    pub graph: Value,
}
#[derive(Default)]
struct Checker {
    structural: bool,
    root: PathBuf,
    sources: BTreeMap<String, String>,
    source_programs: BTreeMap<String, Vec<Node>>,
    observed: BTreeMap<String, Ty>,
    scope_ids: BTreeMap<usize, String>,
    runtime_gates: Vec<(String, Diagnostic)>,
    programs: BTreeMap<PathBuf, Vec<Node>>,
    scopes: Vec<Scope>,
    types: Vec<Option<Ty>>,
    diagnostics: Vec<(String, Diagnostic)>,
    nodes: BTreeMap<String, Value>,
    edges: BTreeSet<(String, String, String)>,
    modules: BTreeMap<PathBuf, usize>,
    active: BTreeSet<PathBuf>,
    file: String,
    source: String,
    owner: String,
    function: Option<Ty>,
    effects: BTreeSet<String>,
    calls: BTreeSet<(String, String)>,
    parallel: Vec<(String, Span, String)>,
    obligations: Vec<(String, Span, Ty)>,
    print: Vec<(String, Span, Ty)>,
    equality: Vec<(String, Span, Ty)>,
    initializing: Option<String>,
    captures: BTreeSet<(String, String)>,
    top_calls: Vec<(String, Span, String, BTreeSet<String>)>,
}
fn name(n: &Node) -> &str {
    if let Kind::Name(s) = &n.kind {
        s
    } else {
        ""
    }
}
fn inner(n: &Node) -> &Node {
    if let Kind::Public(n) = &n.kind {
        n
    } else {
        n
    }
}
impl Checker {
    fn fresh(&mut self) -> Ty {
        let i = self.types.len();
        self.types.push(None);
        Ty::Var(i)
    }
    fn resolve(&self, t: &Ty) -> Ty {
        match t {
            Ty::Var(i) => self.types[*i]
                .as_ref()
                .map(|t| self.resolve(t))
                .unwrap_or(t.clone()),
            Ty::Array(t) => Ty::Array(Box::new(self.resolve(t))),
            Ty::Function(a, r) => Ty::Function(
                a.iter().map(|t| self.resolve(t)).collect(),
                Box::new(self.resolve(r)),
            ),
            _ => t.clone(),
        }
    }
    fn occurs(i: usize, t: &Ty) -> bool {
        match t {
            Ty::Var(j) => i == *j,
            Ty::Array(t) => Self::occurs(i, t),
            Ty::Function(a, r) => a.iter().any(|t| Self::occurs(i, t)) || Self::occurs(i, r),
            _ => false,
        }
    }
    fn fail(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push((self.file.clone(), error(code, span, message)));
    }
    fn unify(&mut self, a: Ty, b: Ty, span: Span) {
        let (a, b) = (self.resolve(&a), self.resolve(&b));
        if a == b {
            return;
        }
        match (a, b) {
            (Ty::Var(i), t) | (t, Ty::Var(i)) if !Self::occurs(i, &t) => self.types[i] = Some(t),
            (Ty::Array(a), Ty::Array(b)) => self.unify(*a, *b, span),
            (Ty::Function(a, r), Ty::Function(b, s)) => {
                if a.len() != b.len() {
                    self.fail("E_ARITY", span, "argument count differs")
                } else {
                    for (a, b) in a.into_iter().zip(b) {
                        self.unify(a, b, span)
                    }
                    self.unify(*r, *s, span)
                }
            }
            (a, b) => self.fail(
                "E_TYPE",
                span,
                format!("incompatible types: {a:?} and {b:?}"),
            ),
        }
    }
    fn scope(&mut self, parent: Option<usize>, module: bool) -> usize {
        let i = self.scopes.len();
        self.scopes.push(Scope {
            parent,
            module,
            ..Scope::default()
        });
        i
    }
    fn id(&mut self, n: &Node, role: &str) -> String {
        let id = format!("{}#{}:{}:{}", self.file, n.span.start, n.span.end, role);
        let (line, column) = position(&self.source, n.span.start);
        self.nodes.entry(id.clone()).or_insert_with(||json!({"id":id,"kind":role,"file":self.file,"start":n.span.start,"end":n.span.end,"line":line,"column":column}));
        id
    }
    fn edge(&mut self, from: String, to: String, kind: &str) {
        self.edges.insert((from, to, kind.into()));
    }
    fn declare(
        &mut self,
        scope: usize,
        n: &Node,
        ty: Ty,
        ready: bool,
        public: bool,
        function: bool,
    ) {
        let id = self.id(n, "definition");
        if self.structural {
            self.nodes.get_mut(&id).unwrap()["public"] = json!(public);
            self.observed.insert(id.clone(), ty.clone());
            if let Ty::Scope(s) = &ty {
                self.scope_ids.entry(*s).or_insert_with(|| id.clone());
            }
        }
        self.edge(self.owner.clone(), id.clone(), "contains");
        if self.scopes[scope].names.contains_key(name(n)) {
            self.fail(
                "E_DUPLICATE",
                n.span,
                "duplicate declaration; overloading is unsupported",
            );
            return;
        }
        self.obligations
            .push((self.file.clone(), n.span, ty.clone()));
        self.scopes[scope].names.insert(
            name(n).into(),
            Binding {
                ty,
                id,
                ready,
                public,
                function,
            },
        );
    }
    fn predeclare(&mut self, body: &[Node], scope: usize) {
        for outer in body {
            let n = inner(outer);
            let public = matches!(outer.kind, Kind::Public(_));
            match &n.kind {
                Kind::Function { name, params, .. } => {
                    let args = params.iter().map(|_| self.fresh()).collect();
                    let ret = self.fresh();
                    self.declare(
                        scope,
                        name,
                        Ty::Function(args, Box::new(ret)),
                        true,
                        public,
                        true,
                    )
                }
                Kind::Object { name, .. } => {
                    let child = self.scope(Some(scope), false);
                    self.declare(scope, name, Ty::Scope(child), false, public, false)
                }
                Kind::Assign(left, _) if matches!(left.kind, Kind::Name(_)) => {
                    if !self.scopes[scope].names.contains_key(name(left)) {
                        let t = self.fresh();
                        self.declare(scope, left, t, false, public, false)
                    } else if public {
                        self.fail("E_DUPLICATE", left.span, "public binding redeclaration")
                    }
                }
                _ => (),
            }
        }
    }
    fn lookup(&mut self, scope: usize, n: &Node) -> Option<(usize, Binding)> {
        let mut at = Some(scope);
        while let Some(s) = at {
            if let Some(b) = self.scopes[s].names.get(name(n)).cloned() {
                // Captures are conservatively effectful; initialization through forward calls is bounded out.
                if !b.ready {
                    self.fail(
                        "E_INITIALIZATION",
                        n.span,
                        "binding read before initialization (forward captures are unsupported)",
                    )
                }
                if self.function.is_some()
                    && s != scope
                    && !b.function
                    && !matches!(b.ty, Ty::Scope(_))
                {
                    self.effects.insert(self.owner.clone());
                    self.captures.insert((self.owner.clone(), b.id.clone()));
                }
                let id = self.id(n, "reference");
                if self.structural {
                    self.observed.insert(id.clone(), b.ty.clone());
                }
                self.edge(self.owner.clone(), id.clone(), "reads");
                self.edge(id, b.id.clone(), "resolves_to");
                return Some((s, b));
            }
            at = self.scopes[s].parent;
        }
        self.fail("E_NAME", n.span, format!("unknown name {}", name(n)));
        None
    }
    fn binding(&mut self, scope: usize, n: &Node) -> Option<Binding> {
        match &n.kind {
            Kind::Name(_) => self.lookup(scope, n).map(|(_, b)| b),
            Kind::Member(base, member) => {
                let t = self.expr(scope, base);
                if let Ty::Scope(s) = self.resolve(&t) {
                    if let Some(b) = self.scopes[s].names.get(name(member)).cloned() {
                        if self.scopes[s].module && !b.public {
                            self.fail("E_PRIVATE", member.span, "module member is private")
                        }
                        if !b.ready {
                            self.fail("E_INITIALIZATION", member.span, "member is not initialized")
                        }
                        if self.function.is_some() && !b.function {
                            self.effects.insert(self.owner.clone());
                            self.captures.insert((self.owner.clone(), b.id.clone()));
                        }
                        let id = self.id(member, "reference");
                        if self.structural {
                            self.observed.insert(id.clone(), b.ty.clone());
                        }
                        self.edge(self.owner.clone(), id.clone(), "reads");
                        self.edge(id, b.id.clone(), "resolves_to");
                        Some(b)
                    } else {
                        self.fail("E_NAME", member.span, "unknown member");
                        None
                    }
                } else {
                    self.fail(
                        "E_TYPE",
                        base.span,
                        "member access requires a structural scope",
                    );
                    None
                }
            }
            _ => None,
        }
    }
    fn invoke(&mut self, scope: usize, callee: &Node, args: Vec<Ty>, span: Span) -> Ty {
        let parameters = args.clone();
        let result = self.invoke_inner(scope, callee, args, span);
        if self.structural {
            let id = self.id(callee, "expression");
            self.edge(self.owner.clone(), id.clone(), "contains");
            self.observed
                .insert(id, Ty::Function(parameters, Box::new(result.clone())));
        }
        result
    }
    fn invoke_inner(&mut self, scope: usize, callee: &Node, args: Vec<Ty>, span: Span) -> Ty {
        // Library contracts exist only at direct calls; library values are deliberately unsupported.
        if let Kind::Name(s) = &callee.kind {
            let mut at = Some(scope);
            let mut shadow = false;
            while let Some(i) = at {
                shadow |= self.scopes[i].names.contains_key(s);
                at = self.scopes[i].parent;
            }
            if !shadow
                && matches!(
                    s.as_str(),
                    "print" | "sum" | "map" | "filter" | "reduce" | "parallel" | "repeat"
                )
            {
                let expected = match s.as_str() {
                    "print" | "sum" => 1,
                    "repeat" | "reduce" => 3,
                    _ => 2,
                };
                if args.len() != expected {
                    self.fail("E_ARITY", span, "incorrect library argument count");
                    return self.fresh();
                }
                let lib = format!("builtin:{s}");
                self.nodes
                    .insert(lib.clone(), json!({"id":lib,"kind":"builtin"}));
                if self.structural {
                    let reference = self.id(callee, "reference");
                    self.edge(reference, lib.clone(), "resolves_to");
                }
                self.edge(self.owner.clone(), lib, "calls");
                match s.as_str() {
                    "print" => {
                        self.effects.insert(self.owner.clone());
                        self.print.push((self.file.clone(), span, args[0].clone()));
                        return Ty::Unit;
                    }
                    "sum" => {
                        self.unify(args[0].clone(), Ty::Array(Box::new(Ty::Number)), span);
                        return Ty::Number;
                    }
                    "reduce" => {
                        let a = self.fresh();
                        self.unify(args[0].clone(), Ty::Array(Box::new(a.clone())), span);
                        self.unify(
                            args[2].clone(),
                            Ty::Function(vec![args[1].clone(), a], Box::new(args[1].clone())),
                            span,
                        );
                        self.effects.insert(self.owner.clone());
                        return args[1].clone();
                    }
                    "filter" => {
                        let a = self.fresh();
                        self.unify(args[0].clone(), Ty::Array(Box::new(a.clone())), span);
                        self.unify(
                            args[1].clone(),
                            Ty::Function(vec![a], Box::new(Ty::Bool)),
                            span,
                        );
                        self.effects.insert(self.owner.clone());
                        return args[0].clone();
                    }
                    "map" | "parallel" => {
                        let a = self.fresh();
                        let b = self.fresh();
                        self.unify(args[0].clone(), Ty::Array(Box::new(a.clone())), span);
                        self.unify(
                            args[1].clone(),
                            Ty::Function(vec![a], Box::new(b.clone())),
                            span,
                        );
                        // Callback identity is checked at the expression site below; unknown effects reject.
                        self.effects.insert(self.owner.clone());
                        return Ty::Array(Box::new(b));
                    }
                    "repeat" => {
                        self.unify(
                            args[1].clone(),
                            Ty::Function(vec![args[0].clone()], Box::new(args[0].clone())),
                            span,
                        );
                        self.unify(
                            args[2].clone(),
                            Ty::Function(vec![args[0].clone()], Box::new(Ty::Bool)),
                            span,
                        );
                        self.effects.insert(self.owner.clone());
                        return args[0].clone();
                    }
                    _ => unreachable!(),
                }
            }
        }
        let b = self.binding(scope, callee);
        let t = if let Some(b) = b {
            if b.function {
                if self.function.is_none() {
                    let ready = self
                        .scopes
                        .iter()
                        .flat_map(|s| s.names.values())
                        .filter(|b| b.ready)
                        .map(|b| b.id.clone())
                        .collect();
                    self.top_calls
                        .push((self.file.clone(), span, b.id.clone(), ready));
                }
                self.calls.insert((self.owner.clone(), b.id.clone()));
                self.edge(self.owner.clone(), b.id, "calls");
            } else {
                self.effects.insert(self.owner.clone());
                self.fail(
                    "E_UNSUPPORTED",
                    span,
                    "indirect user calls have no proven capture/effect contract",
                );
            }
            b.ty
        } else {
            self.expr_nonbinding(scope, callee)
        };
        let ret = self.fresh();
        self.unify(t, Ty::Function(args, Box::new(ret.clone())), span);
        ret
    }
    fn call(
        &mut self,
        scope: usize,
        callee: &Node,
        args: &[Node],
        first: Option<Ty>,
        span: Span,
    ) -> Ty {
        let mut types = Vec::new();
        if let Some(t) = first {
            types.push(t)
        }
        for a in args {
            types.push(self.expr(scope, a));
            if matches!(a.kind, Kind::Name(_) | Kind::Member(..)) {
                if let Some(b) = self.binding(scope, a).filter(|b| b.function) {
                    self.calls.insert((self.owner.clone(), b.id.clone()));
                    self.edge(self.owner.clone(), b.id.clone(), "callback");
                    if self.function.is_none() {
                        let ready = self
                            .scopes
                            .iter()
                            .flat_map(|s| s.names.values())
                            .filter(|b| b.ready)
                            .map(|b| b.id.clone())
                            .collect();
                        self.top_calls.push((self.file.clone(), span, b.id, ready));
                    }
                }
            }
        }
        if name(callee) == "parallel" && types.len() == 2 {
            let callback = args.last().and_then(|n| self.binding(scope, n));
            if let Some(b) = callback.filter(|b| b.function) {
                self.parallel.push((self.file.clone(), span, b.id));
            } else {
                self.fail(
                    "E_EFFECT",
                    span,
                    "parallel requires a statically named function with proven effects",
                )
            }
        }
        self.invoke(scope, callee, types, span)
    }
    fn expr_nonbinding(&mut self, scope: usize, n: &Node) -> Ty {
        match &n.kind {
            Kind::Number(_) => Ty::Number,
            Kind::String(_) => Ty::String,
            Kind::Bool(_) => Ty::Bool,
            Kind::Group(x) => self.expr(scope, x),
            Kind::Array(xs) => {
                let t = self.fresh();
                for x in xs {
                    let a = self.expr(scope, x);
                    self.unify(t.clone(), a, x.span)
                }
                Ty::Array(Box::new(t))
            }
            Kind::Index(a, i) => {
                let a = self.expr(scope, a);
                let i_ty = self.expr(scope, i);
                self.unify(i_ty, Ty::Number, i.span);
                let t = self.fresh();
                self.unify(a, Ty::Array(Box::new(t.clone())), n.span);
                t
            }
            Kind::Unary(op, x) => {
                let t = self.expr(scope, x);
                let expected = if op == "not" { Ty::Bool } else { Ty::Number };
                self.unify(t, expected.clone(), n.span);
                expected
            }
            Kind::Binary(op, a, b) if op == "->" => {
                let t = self.expr(scope, a);
                if let Kind::Call(c, args) = &b.kind {
                    let result = self.call(scope, c, args, Some(t), n.span);
                    if self.structural {
                        let id = self.id(b, "expression");
                        self.observed.insert(id.clone(), result.clone());
                        self.edge(self.owner.clone(), id, "contains");
                    }
                    result
                } else {
                    self.call(scope, b, &[], Some(t), n.span)
                }
            }
            Kind::Binary(op, a, b) => {
                let a = self.expr(scope, a);
                let b = self.expr(scope, b);
                self.unify(a.clone(), b, n.span);
                match op.as_str() {
                    "==" | "!=" => {
                        self.equality.push((self.file.clone(), n.span, a));
                        Ty::Bool
                    }
                    "and" | "or" => {
                        self.unify(a, Ty::Bool, n.span);
                        Ty::Bool
                    }
                    _ => {
                        self.unify(a, Ty::Number, n.span);
                        if matches!(op.as_str(), "<" | ">" | "<=" | ">=") {
                            Ty::Bool
                        } else {
                            Ty::Number
                        }
                    }
                }
            }
            Kind::Call(c, args) => self.call(scope, c, args, None, n.span),
            Kind::Match { subject, arms } => {
                let subject = self.expr(scope, subject);
                let result = self.fresh();
                let mut fallback = 0;
                for (i, arm) in arms.iter().enumerate() {
                    if let Kind::Arm { pattern, value } = &arm.kind {
                        if matches!(pattern.kind, Kind::Wildcard) {
                            fallback += 1;
                            if i + 1 != arms.len() {
                                self.fail(
                                    "E_MATCH_FALLBACK",
                                    pattern.span,
                                    "wildcard must be final",
                                )
                            }
                        } else {
                            let p = match &pattern.kind {
                                Kind::Unary(op, x) if op != "-" => {
                                    let p = self.expr(scope, x);
                                    if !matches!(op.as_str(), "==" | "!=") {
                                        self.unify(p.clone(), Ty::Number, pattern.span)
                                    }
                                    p
                                }
                                _ => self.expr(scope, pattern),
                            };
                            if self.structural {
                                let id = self.id(pattern, "pattern");
                                self.observed.insert(id, p.clone());
                            }
                            self.unify(subject.clone(), p, pattern.span)
                        }
                        let v = self.expr(scope, value);
                        self.unify(result.clone(), v, value.span);
                    }
                }
                if fallback != 1 {
                    self.fail(
                        "E_MATCH_FALLBACK",
                        n.span,
                        "match requires exactly one final wildcard",
                    )
                }
                result
            }
            _ => {
                self.fail("E_UNSUPPORTED", n.span, "unsupported value expression");
                self.fresh()
            }
        }
    }
    fn expr(&mut self, scope: usize, n: &Node) -> Ty {
        let id = self.id(n, "expression");
        self.edge(self.owner.clone(), id.clone(), "contains");
        let t = if matches!(n.kind, Kind::Name(_) | Kind::Member(..)) {
            self.binding(scope, n)
                .map(|b| b.ty)
                .unwrap_or_else(|| self.fresh())
        } else {
            self.expr_nonbinding(scope, n)
        };
        if self.structural {
            self.observed.insert(id, t.clone());
        }
        self.obligations
            .push((self.file.clone(), n.span, t.clone()));
        t
    }
    fn body(&mut self, body: &[Node], scope: usize) {
        let mut returned = false;
        for outer in body {
            let n = inner(outer);
            match &n.kind {
                Kind::Use(_) => (),
                Kind::Assign(left, right) => {
                    self.initializing = self.scopes[scope]
                        .names
                        .get(name(left))
                        .map(|b| b.id.clone());
                    let r = self.expr(scope, right);
                    if matches!(self.resolve(&r), Ty::Scope(_)) {
                        self.fail(
                            "E_UNSUPPORTED",
                            right.span,
                            "structural namespaces cannot be stored as first-class values",
                        );
                    }
                    self.initializing = None;
                    if let Kind::Name(s) = &left.kind {
                        if let Some(b) = self.scopes[scope].names.get(s).cloned() {
                            if b.function || matches!(b.ty, Ty::Scope(_)) {
                                self.fail("E_DUPLICATE", left.span, "cannot reassign a declaration")
                            }
                            self.unify(b.ty, r, n.span);
                            self.scopes[scope].names.get_mut(s).unwrap().ready = true;
                        }
                    } else {
                        let mut root = left.as_ref();
                        while let Kind::Index(base, _) = &root.kind {
                            root = base;
                        }
                        if !self.scopes[scope].names.contains_key(name(root)) {
                            self.fail(
                                "E_EFFECT",
                                left.span,
                                "indexed writes require a local binding or by-value parameter",
                            );
                        }
                        let l = self.expr(scope, left);
                        self.unify(l, r, n.span)
                    }
                }
                Kind::Function { name, params, body } => {
                    let b = self.scopes[scope].names[crate_name(name)].clone();
                    let Ty::Function(args, ret) = b.ty else {
                        continue;
                    };
                    let child = self.scope(Some(scope), false);
                    let previous = (self.owner.clone(), self.function.clone());
                    self.owner = b.id;
                    self.function = Some(*ret);
                    for (p, t) in params.iter().zip(args) {
                        self.declare(child, p, t, true, false, false)
                    }
                    self.predeclare(body, child);
                    self.body(body, child);
                    self.owner = previous.0;
                    self.function = previous.1;
                }
                Kind::Object { name, body } => {
                    let b = self.scopes[scope].names[crate_name(name)].clone();
                    if let Ty::Scope(child) = b.ty {
                        let owner = self.owner.clone();
                        self.owner = b.id;
                        self.predeclare(body, child);
                        self.body(body, child);
                        self.owner = owner;
                        self.scopes[scope]
                            .names
                            .get_mut(crate_name(name))
                            .unwrap()
                            .ready = true;
                    }
                }
                Kind::Return(value) => {
                    let t = value
                        .as_ref()
                        .map(|n| self.expr(scope, n))
                        .unwrap_or(Ty::Unit);
                    if let Some(ret) = self.function.clone() {
                        if matches!(self.resolve(&t), Ty::Scope(_)) {
                            self.fail(
                                "E_UNSUPPORTED",
                                n.span,
                                "structural namespaces cannot be returned as values",
                            );
                        }
                        self.unify(ret, t, n.span)
                    } else {
                        self.fail("E_RETURN", n.span, "return outside function")
                    }
                    returned = true
                }
                _ => {
                    self.expr(scope, n);
                }
            }
        }
        if !returned {
            if let Some(ret) = self.function.clone() {
                self.unify(
                    ret,
                    Ty::Unit,
                    body.last()
                        .map(|n| n.span)
                        .unwrap_or(Span { start: 0, end: 0 }),
                )
            }
        }
    }
    fn load(&mut self, path: &Path) -> Option<usize> {
        let path = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                self.fail("E_IMPORT", Span { start: 0, end: 0 }, e.to_string());
                return None;
            }
        };
        if self.active.contains(&path) {
            self.fail(
                "E_IMPORT_CYCLE",
                Span { start: 0, end: 0 },
                "cyclic file import",
            );
            return None;
        }
        if let Some(s) = self.modules.get(&path) {
            return Some(*s);
        }
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                self.fail("E_IMPORT", Span { start: 0, end: 0 }, e.to_string());
                return None;
            }
        };
        let previous = (self.file.clone(), self.source.clone(), self.owner.clone());
        self.file = if self.structural {
            path.strip_prefix(&self.root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/")
        } else {
            path.file_name().unwrap().to_string_lossy().into()
        };
        if self.structural {
            self.sources.insert(self.file.clone(), source.clone());
        }
        self.source = source.clone();
        self.owner = format!("file:{}", self.file);
        self.nodes.insert(
            self.owner.clone(),
            json!({"id":self.owner,"kind":"module","file":self.file}),
        );
        let body = match parse(&source) {
            Ok(b) => b,
            Err(d) => {
                self.diagnostics.push((self.file.clone(), d));
                self.file = previous.0;
                self.source = previous.1;
                self.owner = previous.2;
                return None;
            }
        };
        let scope = self.scope(None, true);
        self.scope_ids.insert(scope, self.owner.clone());
        if self.structural {
            self.source_programs.insert(self.file.clone(), body.clone());
        }
        self.programs.insert(path.clone(), body.clone());
        self.active.insert(path.clone());
        self.modules.insert(path.clone(), scope);
        for n in &body {
            if let Kind::Use(module) = &n.kind {
                let import = path.with_file_name(format!("{}.luma", name(module)));
                if let Some(s) = self.load(&import) {
                    self.declare(scope, module, Ty::Scope(s), true, false, false);
                    self.edge(self.owner.clone(), self.scope_ids[&s].clone(), "imports")
                }
            }
        }
        self.predeclare(&body, scope);
        self.body(&body, scope);
        self.active.remove(&path);
        self.file = previous.0;
        self.source = previous.1;
        self.owner = previous.2;
        Some(scope)
    }
}
fn crate_name(n: &Node) -> &str {
    name(n)
}
pub fn check(path: impl AsRef<Path>) -> Report {
    check_impl(path.as_ref(), false).0
}
pub(crate) fn check_structural(path: &Path) -> crate::structural::Document {
    check_impl(path, true).1.unwrap()
}
fn check_impl(path: &Path, structural: bool) -> (Report, Option<crate::structural::Document>) {
    let mut c = Checker {
        structural,
        root: path
            .canonicalize()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default(),
        file: path.to_string_lossy().into(),
        ..Checker::default()
    };
    c.load(path);
    loop {
        let before = (c.effects.len(), c.captures.len());
        for (a, b) in &c.calls {
            if c.effects.contains(b) {
                c.effects.insert(a.clone());
            }
        }
        for (a, b) in c.calls.clone() {
            for (owner, value) in c.captures.clone() {
                if owner == b {
                    c.captures.insert((a.clone(), value));
                }
            }
        }
        if before == (c.effects.len(), c.captures.len()) {
            break;
        }
    }
    for (file, span, id, ready) in c.top_calls.clone() {
        if c.captures
            .iter()
            .any(|(owner, value)| owner == &id && !ready.contains(value))
        {
            c.file = file;
            c.fail(
                "E_INITIALIZATION",
                span,
                "call may read an uninitialized capture",
            )
        }
    }
    for (file, span, id) in c.parallel.clone() {
        if c.effects.contains(&id) {
            c.file = file;
            c.fail(
                "E_EFFECT",
                span,
                "parallel callback has I/O, captures, indirect calls or unproven callback effects",
            )
        }
    }
    fn printable(t: &Ty) -> bool {
        match t {
            Ty::Number | Ty::String | Ty::Bool | Ty::Var(_) => true,
            Ty::Array(element) => printable(element),
            _ => false,
        }
    }
    for (file, span, t) in c.print.clone() {
        if !printable(&c.resolve(&t)) {
            c.file = file;
            c.fail(
                "E_TYPE",
                span,
                "print requires a scalar or arrays of scalars",
            )
        }
    }
    for (file, span, t) in c.equality.clone() {
        if !matches!(
            c.resolve(&t),
            Ty::Number | Ty::String | Ty::Bool | Ty::Var(_)
        ) {
            c.file = file;
            c.fail("E_TYPE", span, "only scalar equality is supported")
        }
    }
    for (file, span, t) in c.obligations.clone() {
        let t = c.resolve(&t);
        if unresolved(&t) {
            c.file = file;
            c.fail(
                "E_INFERENCE",
                span,
                "type remains unresolved; user polymorphism is unsupported",
            )
        }
    }
    // Types are attached to every expression/definition with the same source span.
    for (file, span, t) in &c.obligations {
        for role in ["expression", "definition"] {
            let id = format!("{}#{}:{}:{}", file, span.start, span.end, role);
            if let Some(n) = c.nodes.get_mut(&id) {
                n["type"] = json!(format!("{:?}", resolve_types(&c.types, t)));
            }
        }
    }
    c.diagnostics
        .sort_by_key(|(f, d)| (f.clone(), d.span.start, d.code));
    c.diagnostics.dedup();
    if structural {
        for (owner, value) in &c.captures {
            c.edges
                .insert((owner.clone(), value.clone(), "captures".into()));
        }
    }
    let edges: Vec<_> = c
        .edges
        .iter()
        .map(|(from, to, kind)| json!({"from":from,"to":to,"kind":kind}))
        .collect();
    if structural {
        for (id, t) in &c.observed {
            if let Some(n) = c.nodes.get_mut(id) {
                n["structuralType"] = serde_json::to_value(structural_type(
                    &resolve_types(&c.types, t),
                    &c.scope_ids,
                ))
                .unwrap();
            }
        }
        for (id, n) in &mut c.nodes {
            if n["kind"] == "module"
                || c.scopes
                    .iter()
                    .any(|s| s.names.values().any(|b| b.function && &b.id == id))
            {
                n["effect"] = json!(if c.effects.contains(id) {
                    "conservative_effectful"
                } else {
                    "not_flagged"
                });
            }
        }
    }
    let graph = json!({"schemaVersion":1,"producer":"luma-0.1C","valid":c.diagnostics.is_empty(),"nodes":c.nodes.values().collect::<Vec<_>>(),"edges":edges});
    let document = structural
        .then(|| crate::structural::build(&c.sources, &c.source_programs, &graph, &c.diagnostics));
    (
        Report {
            runtime_gates: c.runtime_gates,
            programs: c.programs,
            graph,
            diagnostics: c.diagnostics,
        },
        document,
    )
}
fn resolve_types(types: &[Option<Ty>], t: &Ty) -> Ty {
    match t {
        Ty::Var(i) => types[*i]
            .as_ref()
            .map(|t| resolve_types(types, t))
            .unwrap_or(t.clone()),
        Ty::Array(t) => Ty::Array(Box::new(resolve_types(types, t))),
        Ty::Function(a, r) => Ty::Function(
            a.iter().map(|t| resolve_types(types, t)).collect(),
            Box::new(resolve_types(types, r)),
        ),
        _ => t.clone(),
    }
}
fn unresolved(t: &Ty) -> bool {
    match t {
        Ty::Var(_) => true,
        Ty::Array(t) => unresolved(t),
        Ty::Function(a, r) => a.iter().any(unresolved) || unresolved(r),
        _ => false,
    }
}

fn structural_type(t: &Ty, scopes: &BTreeMap<usize, String>) -> crate::structural::Type {
    use crate::structural::Type;
    match t {
        Ty::Var(_) => Type::Unresolved,
        Ty::Number => Type::Number,
        Ty::String => Type::String,
        Ty::Bool => Type::Bool,
        Ty::Unit => Type::Unit,
        Ty::Array(t) => Type::Array {
            element: Box::new(structural_type(t, scopes)),
        },
        Ty::Function(a, r) => Type::Function {
            parameters: a.iter().map(|t| structural_type(t, scopes)).collect(),
            result: Box::new(structural_type(r, scopes)),
        },
        Ty::Scope(s) => Type::Namespace {
            target: scopes[s].clone(),
        },
    }
}
