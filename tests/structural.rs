use luma::structural::{analyze, Document, Type};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Case(PathBuf);
impl Case {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "luma-s1-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn write(&self, file: &str, s: &str) {
        std::fs::write(self.0.join(file), s).unwrap();
    }
    fn analyze(&self) -> Document {
        analyze(self.0.join("main.luma"))
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/structural/main.luma")
}
fn check_links(d: &Document) {
    let ids: BTreeSet<_> = d.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids.len(), d.nodes.len(), "semantic collision");
    let syntax: BTreeSet<_> = d.syntax.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(syntax.len(), d.syntax.len(), "syntax collision");
    for e in &d.edges {
        assert!(ids.contains(e.from.as_str()), "{e:?}");
        assert!(ids.contains(e.to.as_str()), "{e:?}");
    }
    for n in &d.nodes {
        if let Some(s) = &n.syntax {
            assert!(syntax.contains(s.as_str()));
        }
    }
    for n in &d.syntax {
        let f = d.files.iter().find(|f| f.id == n.location.file).unwrap();
        assert!(f.content.get(n.location.start..n.location.end).is_some());
        if let Some(p) = &n.parent {
            assert!(syntax.contains(p.as_str()) || d.files.iter().any(|f| &f.id == p));
        }
    }
    fn ty(t: &Type, ids: &BTreeSet<&str>) {
        match t {
            Type::Unresolved => panic!("unresolved valid type"),
            Type::Namespace { target } => assert!(ids.contains(target.as_str())),
            Type::Array { element } => ty(element, ids),
            Type::Function { parameters, result } => {
                for p in parameters {
                    ty(p, ids);
                }
                ty(result, ids);
            }
            _ => (),
        }
    }
    for n in &d.nodes {
        if let Some(t) = &n.ty {
            ty(t, &ids);
        }
    }
}
#[test]
fn pipelines_match_objects_callbacks_and_cross_module_links() {
    let d = analyze(fixture());
    assert!(d.valid, "{:?}", d.diagnostics);
    check_links(&d);
    for kind in ["pipeline", "match", "object", "call"] {
        assert!(d.syntax.iter().any(|n| n.kind == kind));
    }
    for kind in ["imports", "callback", "calls", "captures", "resolves_to"] {
        assert!(d.edges.iter().any(|e| e.kind == kind), "{kind}");
    }
    assert!(d.edges.iter().any(|e| e.kind == "resolves_to"
        && e.from.starts_with("file:main.luma")
        && e.to.starts_with("file:math.luma")));
    assert!(d.nodes.iter().any(|n| n.public == Some(true)));
    for n in &d.syntax {
        if n.kind == "pipeline" || n.kind == "match" || n.kind == "call" {
            assert!(
                d.nodes
                    .iter()
                    .any(|s| s.syntax.as_ref() == Some(&n.id) && s.ty.is_some()),
                "missing type: {n:?}"
            );
        }
    }
    assert!(d
        .nodes
        .iter()
        .any(|n| n.effect.as_deref() == Some("not_flagged")));
    assert!(d
        .nodes
        .iter()
        .any(|n| n.effect.as_deref() == Some("conservative_effectful")));
    assert_eq!(d, analyze(fixture()));
    assert_eq!(
        d,
        serde_json::from_str::<Document>(&d.to_json().unwrap()).unwrap()
    );
    assert_eq!(luma::runtime::run(fixture()).unwrap(), "3\n");
}
#[test]
fn identities_survive_comments_whitespace_and_unrelated_named_edits() {
    let c = Case::new();
    let source = "fn f(n)\nreturn n+1\nend\na=f(2)\n";
    c.write("main.luma", source);
    let a = c.analyze();
    c.write(
        "main.luma",
        "# unrelated comment: привет\nunrelated = true\nfn f(n)\n  return n + 1\nend\na = f(2)\n",
    );
    let b = c.analyze();
    assert!(a.valid && b.valid);
    check_links(&b);
    let b_ids: BTreeSet<_> = b.nodes.iter().map(|n| &n.id).collect();
    for n in &a.nodes {
        assert!(b_ids.contains(&n.id), "changed {}", n.id);
    }
    assert_eq!(a.files[0].id, b.files[0].id);
    assert_ne!(a.files[0].content, b.files[0].content);
    let relocated = Case::new();
    relocated.write("main.luma", source);
    assert_eq!(a, relocated.analyze());
}
#[test]
fn repeated_statements_and_shadowed_names_do_not_collide() {
    let c = Case::new();
    c.write(
        "main.luma",
        "x=1\nx=2\nfn f(x)\nreturn x+1\nend\nprint(f(x))\nprint(f(x))\n",
    );
    let d = c.analyze();
    assert!(d.valid);
    check_links(&d);
    assert!(d.nodes.iter().filter(|n| n.kind == "definition").count() >= 3);
}
#[test]
fn invalid_source_never_exports_usable_ir() {
    let c = Case::new();
    for (s, code) in [
        ("x = (", "E_SYNTAX"),
        ("x = absent", "E_NAME"),
        ("x=1+true", "E_TYPE"),
        ("fn f(x)\nreturn x\nend", "E_INFERENCE"),
        ("use missing", "E_IMPORT"),
        (
            "fn f(n)\nprint(n)\nreturn n\nend\nx=parallel([1], f)",
            "E_EFFECT",
        ),
    ] {
        c.write("main.luma", s);
        let d = c.analyze();
        assert!(!d.valid);
        assert!(
            d.diagnostics.iter().any(|d| d.code == code),
            "{s}: {:?}",
            d.diagnostics
        );
        assert!(d.nodes.is_empty() && d.syntax.is_empty() && d.edges.is_empty());
        assert_eq!(d.files[0].content, s);
    }
    c.write("main.luma", "use other");
    c.write("other.luma", "use main");
    assert!(c
        .analyze()
        .diagnostics
        .iter()
        .any(|d| d.code == "E_IMPORT_CYCLE"));
    let d = analyze(c.0.join("absent.luma"));
    assert!(!d.valid);
    assert!(d.files.is_empty());
}
#[test]
fn all_conformance_agrees_with_existing_checker() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../conformance/manifest.json")).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("conformance/cases")
            .join(case["id"].as_str().unwrap())
            .join("main.luma");
        let old = luma::semantic::check(&p);
        let d = analyze(&p);
        assert_eq!(d.valid, old.diagnostics.is_empty(), "{}", p.display());
        let codes: Vec<_> = d.diagnostics.iter().map(|d| d.code.as_str()).collect();
        let old_codes: Vec<_> = old.diagnostics.iter().map(|(_, d)| d.code).collect();
        assert_eq!(codes, old_codes);
        if d.valid {
            check_links(&d);
        }
    }
}
#[test]
fn cli_json_exit_codes_and_no_execution() {
    let c = Case::new();
    c.write("main.luma", "print(1/0)\n");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luma"))
        .arg("ir")
        .arg(c.0.join("main.luma"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let d: Document = serde_json::from_slice(&output.stdout).unwrap();
    assert!(d.valid);
    assert!(output.stderr.is_empty());
    c.write("main.luma", "no_such_name");
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_luma"))
        .arg("ir")
        .arg(c.0.join("main.luma"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(!serde_json::from_slice::<Document>(&o.stdout).unwrap().valid);
    assert_eq!(
        std::process::Command::new(env!("CARGO_BIN_EXE_luma"))
            .arg("ir")
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

#[cfg(unix)]
#[test]
fn canonical_modules_with_same_basename_stay_distinct() {
    use std::os::unix::fs::symlink;
    let c = Case::new();
    for dir in ["one", "two"] {
        std::fs::create_dir(c.0.join(dir)).unwrap();
    }
    c.write(
        "one/shared.luma",
        "use dep\npub fn f(n)\nreturn dep.f(n)\nend\n",
    );
    c.write(
        "two/shared.luma",
        "use dep\npub fn f(n)\nreturn dep.f(n)\nend\n",
    );
    c.write("one/dep.luma", "pub fn f(n)\nreturn n+1\nend\n");
    c.write("two/dep.luma", "pub fn f(n)\nreturn n+2\nend\n");
    symlink(c.0.join("one/shared.luma"), c.0.join("a.luma")).unwrap();
    symlink(c.0.join("two/shared.luma"), c.0.join("b.luma")).unwrap();
    c.write("main.luma", "use a\nuse b\nprint(a.f(1)+b.f(1))\n");
    let d = c.analyze();
    assert!(d.valid, "{:?}", d.diagnostics);
    check_links(&d);
    assert_eq!(d.files.len(), 5);
    for dir in ["one", "two"] {
        assert!(d.edges.iter().any(|e| e.kind == "resolves_to"
            && e.from.starts_with(&format!("file:{dir}/shared.luma"))
            && e.to.starts_with(&format!("file:{dir}/dep.luma"))));
    }
}
