use luma::semantic::check;
use std::path::PathBuf;
#[test]
fn all_conformance_contracts() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../conformance/manifest.json")).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let report = check(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("conformance/cases")
                .join(id)
                .join("main.luma"),
        );
        if case["expected"]["outcome"] == "static-error" {
            let code = case["expected"]["diagnostic"].as_str().unwrap();
            assert!(
                report.diagnostics.iter().any(|(_, d)| d.code == code),
                "{id}: {:?}",
                report.diagnostics
            );
        } else {
            assert!(
                report.diagnostics.is_empty(),
                "{id}: {:?}",
                report.diagnostics
            );
        }
        let again = check(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("conformance/cases")
                .join(id)
                .join("main.luma"),
        );
        assert_eq!(report.graph, again.graph);
        let nodes = report.graph["nodes"].as_array().unwrap();
        for edge in report.graph["edges"].as_array().unwrap() {
            for end in ["from", "to"] {
                assert!(
                    nodes.iter().any(|n| n["id"] == edge[end]),
                    "dangling edge {edge}"
                );
            }
        }
    }
}
fn source(s: &str) -> luma::semantic::Report {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "luma-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.luma");
    std::fs::write(&path, s).unwrap();
    let r = check(&path);
    std::fs::remove_dir_all(dir).unwrap();
    r
}
#[test]
fn semantic_boundaries() {
    for s in [
        "a=f()\nx=1\nfn f()\nreturn x\nend",
        "a=map([1], f)\nx=1\nfn f(n)\nreturn x+n\nend",
    ] {
        assert!(source(s)
            .diagnostics
            .iter()
            .any(|(_, d)| d.code == "E_INITIALIZATION"));
    }
    for s in [
        "x=1\nfn f(n)\nreturn n+x\nend\na=parallel([1],f)",
        "object O\nx=1\nend\nfn f(n)\nreturn n+O.x\nend\na=parallel([1],f)",
        "a=[1]\nfn f()\na[0]=2\nend\nf()",
    ] {
        assert!(source(s)
            .diagnostics
            .iter()
            .any(|(_, d)| d.code == "E_EFFECT"));
    }
    for (s, code) in [
        ("return 1", "E_RETURN"),
        ("fn f(x,x)\nreturn x\nend", "E_DUPLICATE"),
        (
            "fn f()\nreturn 1\nend\nfn f(x)\nreturn x\nend",
            "E_DUPLICATE",
        ),
        (
            "x = 1\nfn f()\ny = x\nx = 2\nreturn y\nend\nz = f()",
            "E_INITIALIZATION",
        ),
        ("x = 1\ny = x(2)", "E_TYPE"),
        ("fn f(x)\nreturn x\nend", "E_INFERENCE"),
        ("x = match true\n*: 1\ntrue: 2\nend", "E_MATCH_FALLBACK"),
        ("fn f(x)\nreturn f([x])\nend\ny=f(1)", "E_TYPE"),
        ("use missing", "E_IMPORT"),
    ] {
        let r = source(s);
        assert!(
            r.diagnostics.iter().any(|(_, d)| d.code == code),
            "{s}: {:?}",
            r.diagnostics
        );
    }
    for s in [
        "fn f(x)\nreturn x\nend\na=f(1)\nb=f(true)",
        "fn f(x)\nreturn x\nend\nb=f(true)\na=f(1)",
    ] {
        assert!(source(s)
            .diagnostics
            .iter()
            .any(|(_, d)| d.code == "E_TYPE"));
    }
    assert!(source("fn f(x)\nreturn x\nend\nx=[]\ny=f(x)\nx=[1]")
        .diagnostics
        .is_empty());
}
