//! Deterministic execution of the checker's validated source snapshot.
use crate::{error, semantic, Diagnostic, Kind, Node};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Number(f64),
    String(String),
    Bool(bool),
    Array(Vec<Value>),
    Unit,
    Scope(usize),
    Function(usize, PathBuf, Vec<Node>, Vec<Node>),
}
#[derive(Clone, Default)]
struct Scope {
    parent: Option<usize>,
    values: BTreeMap<String, Value>,
}
#[derive(Debug)]
pub struct Failure {
    pub diagnostics: Vec<(String, Diagnostic)>,
    pub runtime: bool,
}
#[derive(Clone)]
struct Engine {
    programs: BTreeMap<PathBuf, Vec<Node>>,
    modules: BTreeMap<PathBuf, usize>,
    scopes: Vec<Scope>,
    output: String,
    fuel: usize,
    depth: usize,
    error_file: Option<PathBuf>,
}
type Result<T> = std::result::Result<T, Diagnostic>;
fn name(n: &Node) -> &str {
    if let Kind::Name(s) = &n.kind {
        s
    } else {
        ""
    }
}
fn inner(n: &Node) -> &Node {
    if let Kind::Public(x) = &n.kind {
        x
    } else {
        n
    }
}
impl Engine {
    fn scope(&mut self, parent: Option<usize>) -> usize {
        let id = self.scopes.len();
        self.scopes.push(Scope {
            parent,
            ..Scope::default()
        });
        id
    }
    fn lookup(&self, mut s: usize, key: &str) -> Option<Value> {
        loop {
            if let Some(v) = self.scopes[s].values.get(key) {
                return Some(v.clone());
            }
            s = self.scopes[s].parent?;
        }
    }
    fn fail(&self, n: &Node, message: &str) -> Diagnostic {
        let code = if message.contains("index") || message.contains("bounds") {
            "E_INDEX"
        } else if message.contains("by zero") {
            "E_DIV_ZERO"
        } else {
            "E_RUNTIME"
        };
        error(code, n.span, message)
    }
    fn prepare(&mut self, body: &[Node], s: usize, path: &Path) {
        for n in body {
            if let Kind::Function {
                name: key,
                params,
                body,
            } = &inner(n).kind
            {
                self.scopes[s].values.insert(
                    name(key).into(),
                    Value::Function(s, path.into(), params.clone(), body.clone()),
                );
            }
        }
    }
    fn load(&mut self, path: &Path) -> Result<usize> {
        if let Some(s) = self.modules.get(path) {
            return Ok(*s);
        }
        let body = self.programs[path].clone();
        let s = self.scope(None);
        self.modules.insert(path.into(), s);
        for n in &body {
            if let Kind::Use(m) = &n.kind {
                let p = path
                    .with_file_name(format!("{}.luma", name(m)))
                    .canonicalize()
                    .map_err(|e| self.fail(n, &e.to_string()))?;
                let child = self.load(&p)?;
                self.scopes[s]
                    .values
                    .insert(name(m).into(), Value::Scope(child));
            }
        }
        self.prepare(&body, s, path);
        self.body(&body, s, path)?;
        Ok(s)
    }
    fn body(&mut self, body: &[Node], s: usize, path: &Path) -> Result<Option<Value>> {
        let result = self.body_inner(body, s, path);
        if result.is_err() && self.error_file.is_none() {
            self.error_file = Some(path.into());
        }
        result
    }
    fn body_inner(&mut self, body: &[Node], s: usize, path: &Path) -> Result<Option<Value>> {
        for outer in body {
            let n = inner(outer);
            match &n.kind {
                Kind::Use(_) | Kind::Function { .. } => (),
                Kind::Return(x) => {
                    return Ok(Some(if let Some(x) = x {
                        self.eval(x, s, path)?
                    } else {
                        Value::Unit
                    }))
                }
                Kind::Assign(left, right) => {
                    let v = self.eval(right, s, path)?;
                    self.assign(left, v, s, path)?;
                }
                Kind::Object { name: key, body } => {
                    let child = self.scope(Some(s));
                    self.prepare(body, child, path);
                    self.body(body, child, path)?;
                    self.scopes[s]
                        .values
                        .insert(name(key).into(), Value::Scope(child));
                }
                _ => {
                    self.eval(n, s, path)?;
                }
            }
        }
        Ok(None)
    }
    fn assign(&mut self, n: &Node, v: Value, s: usize, path: &Path) -> Result<()> {
        fn replace(slot: &mut Value, indices: &[usize], v: Value) -> bool {
            if indices.is_empty() {
                *slot = v;
                true
            } else if let Value::Array(xs) = slot {
                xs.get_mut(indices[0])
                    .is_some_and(|x| replace(x, &indices[1..], v))
            } else {
                false
            }
        }
        fn parts<'a>(n: &'a Node, out: &mut Vec<&'a Node>) -> &'a Node {
            if let Kind::Index(a, i) = &n.kind {
                let root = parts(a, out);
                out.push(i);
                root
            } else {
                n
            }
        }
        let mut nodes = Vec::new();
        let root = parts(n, &mut nodes);
        let mut indices = Vec::new();
        for i in nodes {
            let value = self.eval(i, s, path)?;
            indices.push(self.index(value, i)?);
        }
        if indices.is_empty() {
            self.scopes[s].values.insert(name(root).into(), v);
            return Ok(());
        }
        if self.scopes[s]
            .values
            .get_mut(name(root))
            .is_some_and(|x| replace(x, &indices, v))
        {
            Ok(())
        } else {
            Err(self.fail(n, "array write out of bounds"))
        }
    }
    fn index(&self, v: Value, n: &Node) -> Result<usize> {
        if let Value::Number(x) = v {
            if x >= 0.0 && x.fract() == 0.0 && x < usize::MAX as f64 {
                return Ok(x as usize);
            }
        }
        Err(self.fail(n, "index must be a nonnegative integer within array bounds"))
    }
    fn call(
        &mut self,
        c: &Node,
        args: &[Node],
        first: Option<Value>,
        s: usize,
        path: &Path,
        n: &Node,
    ) -> Result<Value> {
        let callee = self.lookup(s, name(c));
        let mut values = first.into_iter().collect::<Vec<_>>();
        for a in args {
            values.push(self.eval(a, s, path)?);
        }
        if name(c) == "print" && callee.is_none() {
            fn format_value(value: &Value, nested: bool) -> Option<String> {
                Some(match value {
                    Value::Number(x) => x.to_string(),
                    Value::String(x) if nested => serde_json::to_string(x).ok()?,
                    Value::String(x) => x.clone(),
                    Value::Bool(x) => x.to_string(),
                    Value::Array(xs) => format!(
                        "[{}]",
                        xs.iter()
                            .map(|x| format_value(x, true))
                            .collect::<Option<Vec<_>>>()?
                            .join(", ")
                    ),
                    _ => return None,
                })
            }
            let text = format_value(&values[0], false)
                .ok_or_else(|| self.fail(n, "print requires a scalar or arrays of scalars"))?;
            self.output.push_str(&text);
            self.output.push('\n');
            return Ok(Value::Unit);
        }
        if callee.is_none()
            && matches!(
                name(c),
                "sum" | "map" | "filter" | "reduce" | "repeat" | "parallel"
            )
        {
            return self.library(name(c), values, n);
        }
        let f = self.eval(c, s, path)?;
        self.apply(f, values, n)
    }
    fn apply(&mut self, f: Value, values: Vec<Value>, n: &Node) -> Result<Value> {
        if let Value::Function(parent, file, params, body) = f {
            if self.depth >= 32 {
                return Err(self.fail(n, "call depth limit (32) exceeded"));
            }
            let local = self.scope(Some(parent));
            for (p, v) in params.iter().zip(values) {
                self.scopes[local].values.insert(name(p).into(), v);
            }
            self.depth += 1;
            let result = self.body(&body, local, &file);
            self.depth -= 1;
            result.map(|v| v.unwrap_or(Value::Unit))
        } else {
            Err(self.fail(n, "value is not callable"))
        }
    }
    fn library(&mut self, op: &str, mut values: Vec<Value>, n: &Node) -> Result<Value> {
        let input = values.remove(0);
        if op == "repeat" {
            let mut state = input;
            for _ in 0..10000 {
                if self.apply(values[1].clone(), vec![state.clone()], n)? == Value::Bool(true) {
                    return Ok(state);
                }
                state = self.apply(values[0].clone(), vec![state], n)?;
            }
            if self.apply(values[1].clone(), vec![state.clone()], n)? == Value::Bool(true) {
                return Ok(state);
            }
            return Err(self.fail(n, "repeat limit (10000 transitions) exceeded"));
        }
        let Value::Array(xs) = input else {
            return Err(self.fail(n, "library requires array"));
        };
        if op == "sum" {
            let mut total = Value::Number(0.0);
            for x in xs {
                total = self.binary("+", total, x, n)?;
            }
            return Ok(total);
        }
        if op == "reduce" {
            let mut acc = values.remove(0);
            let f = values.remove(0);
            for x in xs {
                acc = self.apply(f.clone(), vec![acc, x], n)?;
            }
            return Ok(acc);
        }
        let f = values.remove(0);
        if op == "parallel" {
            if xs.len() > 256 {
                return Err(self.fail(n, "parallel input limit (256) exceeded"));
            }
            // Each task owns a complete snapshot; no mutable state crosses workers.
            // Fixed indexed chunks and ordered joins make failures deterministic.
            let workers = xs.len().clamp(1, 8);
            let size = xs.len().div_ceil(workers).max(1);
            let snapshot = &*self;
            let result = std::thread::scope(|scope| {
                let mut handles = Vec::new();
                for chunk in xs.chunks(size) {
                    let f = &f;
                    handles.push(scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|x| {
                                let mut worker = snapshot.clone();
                                #[cfg(test)]
                                WORKER_THREADS
                                    .lock()
                                    .unwrap()
                                    .push(std::thread::current().id());
                                worker.fuel = 100000;
                                worker.apply(f.clone(), vec![x.clone()], n)
                            })
                            .collect::<Result<Vec<_>>>()
                    }));
                }
                let mut out = Vec::new();
                let mut failure = None;
                for handle in handles {
                    match handle.join() {
                        Ok(Ok(xs)) => out.extend(xs),
                        Ok(Err(e)) => {
                            if failure.is_none() {
                                failure = Some(e);
                            }
                        }
                        Err(_) => {
                            if failure.is_none() {
                                failure = Some(snapshot.fail(n, "parallel worker panicked"));
                            }
                        }
                    }
                }
                failure.map_or(Ok(out), Err)
            });
            return result.map(Value::Array);
        }
        let mut out = Vec::new();
        for x in xs {
            let v = self.apply(f.clone(), vec![x.clone()], n)?;
            if op == "filter" {
                if v == Value::Bool(true) {
                    out.push(x);
                }
            } else {
                out.push(v);
            }
        }
        Ok(Value::Array(out))
    }
    fn binary(&self, op: &str, a: Value, b: Value, n: &Node) -> Result<Value> {
        if op == "==" {
            return Ok(Value::Bool(a == b));
        }
        if op == "!=" {
            return Ok(Value::Bool(a != b));
        }
        match (a, b) {
            (Value::Bool(a), Value::Bool(b)) => match op {
                "and" => Ok(Value::Bool(a && b)),
                "or" => Ok(Value::Bool(a || b)),
                _ => Err(self.fail(n, "invalid boolean operation")),
            },
            (Value::Number(a), Value::Number(b)) => {
                let x = match op {
                    "<" => return Ok(Value::Bool(a < b)),
                    ">" => return Ok(Value::Bool(a > b)),
                    "<=" => return Ok(Value::Bool(a <= b)),
                    ">=" => return Ok(Value::Bool(a >= b)),
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" | "%" if b == 0.0 => {
                        return Err(self.fail(n, "division or remainder by zero"))
                    }
                    "/" => a / b,
                    "%" => a % b,
                    "^" => a.powf(b),
                    _ => return Err(self.fail(n, "invalid numeric operation")),
                };
                if x.is_finite() {
                    Ok(Value::Number(x))
                } else {
                    Err(self.fail(n, "numeric result is not finite"))
                }
            }
            _ => Err(self.fail(n, "invalid operand types")),
        }
    }
    fn eval(&mut self, n: &Node, s: usize, path: &Path) -> Result<Value> {
        if self.fuel == 0 {
            return Err(self.fail(n, "evaluation budget (100000 steps) exhausted"));
        }
        self.fuel -= 1;
        match &n.kind {
            Kind::Number(x) => {
                let x = x
                    .parse::<f64>()
                    .map_err(|_| self.fail(n, "invalid number"))?;
                if x.is_finite() {
                    Ok(Value::Number(x))
                } else {
                    Err(self.fail(n, "number is not finite"))
                }
            }
            Kind::String(x) => Ok(Value::String(x.clone())),
            Kind::Bool(x) => Ok(Value::Bool(*x)),
            Kind::Name(x) => self
                .lookup(s, x)
                .ok_or_else(|| self.fail(n, "binding is not initialized")),
            Kind::Group(x) => self.eval(x, s, path),
            Kind::Array(xs) => xs
                .iter()
                .map(|x| self.eval(x, s, path))
                .collect::<Result<Vec<_>>>()
                .map(Value::Array),
            Kind::Member(base, key) => {
                if let Value::Scope(child) = self.eval(base, s, path)? {
                    self.scopes[child]
                        .values
                        .get(name(key))
                        .cloned()
                        .ok_or_else(|| self.fail(n, "member is not initialized"))
                } else {
                    Err(self.fail(n, "member requires namespace"))
                }
            }
            Kind::Index(a, i) => {
                let a = self.eval(a, s, path)?;
                let v = self.eval(i, s, path)?;
                let i = self.index(v, i)?;
                if let Value::Array(xs) = a {
                    xs.get(i)
                        .cloned()
                        .ok_or_else(|| self.fail(n, "array index out of bounds"))
                } else {
                    Err(self.fail(n, "index requires array"))
                }
            }
            Kind::Unary(op, x) => {
                let v = self.eval(x, s, path)?;
                match (op.as_str(), v) {
                    ("-", Value::Number(x)) => Ok(Value::Number(-x)),
                    ("not", Value::Bool(x)) => Ok(Value::Bool(!x)),
                    _ => Err(self.fail(n, "invalid unary operand")),
                }
            }
            Kind::Call(c, args) => self.call(c, args, None, s, path, n),
            Kind::Binary(op, a, b) if op == "->" => {
                let first = self.eval(a, s, path)?;
                if let Kind::Call(c, args) = &b.kind {
                    self.call(c, args, Some(first), s, path, n)
                } else {
                    self.call(b, &[], Some(first), s, path, n)
                }
            }
            Kind::Binary(op, a, b) => {
                let a = self.eval(a, s, path)?;
                if (op == "and" && a == Value::Bool(false))
                    || (op == "or" && a == Value::Bool(true))
                {
                    return Ok(a);
                }
                let b = self.eval(b, s, path)?;
                self.binary(op, a, b, n)
            }
            Kind::Match { subject, arms } => {
                let subject = self.eval(subject, s, path)?;
                for arm in arms {
                    if let Kind::Arm { pattern, value } = &arm.kind {
                        let yes = if matches!(pattern.kind, Kind::Wildcard) {
                            true
                        } else if let Kind::Unary(op, x) = &pattern.kind {
                            if op != "-" {
                                let p = self.eval(x, s, path)?;
                                self.binary(op, subject.clone(), p, pattern)? == Value::Bool(true)
                            } else {
                                self.eval(pattern, s, path)? == subject
                            }
                        } else {
                            self.eval(pattern, s, path)? == subject
                        };
                        if yes {
                            return self.eval(value, s, path);
                        }
                    }
                }
                Err(self.fail(n, "match has no selected arm"))
            }
            _ => Err(self.fail(n, "unsupported expression")),
        }
    }
}
#[cfg(test)]
static WORKER_THREADS: std::sync::Mutex<Vec<std::thread::ThreadId>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
#[test]
fn parallel_uses_distinct_native_threads() {
    let nodes = crate::parse("fn square(x)\nreturn x*x\nend").unwrap();
    let mut engine = Engine {
        programs: BTreeMap::new(),
        modules: BTreeMap::new(),
        scopes: vec![Scope::default()],
        output: String::new(),
        fuel: 100000,
        depth: 0,
        error_file: None,
    };
    engine.prepare(&nodes, 0, Path::new("test.luma"));
    let f = engine.lookup(0, "square").unwrap();
    let result = engine
        .library(
            "parallel",
            vec![
                Value::Array((0..16).map(|i| Value::Number(i as f64)).collect()),
                f,
            ],
            &nodes[0],
        )
        .unwrap();
    assert_eq!(
        result,
        Value::Array((0..16).map(|i| Value::Number((i * i) as f64)).collect())
    );
    let ids = WORKER_THREADS.lock().unwrap();
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 8);
    assert!(!unique.contains(&std::thread::current().id()));
}
/// Checks and gates the entire import graph before evaluating any statement.
pub fn run(path: impl AsRef<Path>) -> std::result::Result<String, Failure> {
    let report = semantic::check(path.as_ref());
    if !report.diagnostics.is_empty() {
        return Err(Failure {
            diagnostics: report.diagnostics,
            runtime: false,
        });
    }
    let gates = report.runtime_gates;
    if !gates.is_empty() {
        return Err(Failure {
            diagnostics: gates,
            runtime: false,
        });
    }
    let path = path.as_ref().canonicalize().unwrap();
    let mut e = Engine {
        programs: report.programs,
        modules: BTreeMap::new(),
        scopes: Vec::new(),
        output: String::new(),
        fuel: 100000,
        depth: 0,
        error_file: None,
    };
    match e.load(&path) {
        Ok(_) => Ok(e.output),
        Err(d) => Err(Failure {
            diagnostics: vec![(
                e.error_file.as_ref().unwrap_or(&path).display().to_string(),
                d,
            )],
            runtime: true,
        }),
    }
}
