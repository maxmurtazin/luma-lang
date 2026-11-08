use luma::runtime::run;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Case(PathBuf);
impl Case {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "luma-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn write(&self, file: &str, text: &str) {
        fs::write(self.0.join(file), text).unwrap();
    }
    fn run(&self) -> Result<String, luma::runtime::Failure> {
        run(self.0.join("main.luma"))
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn runtime_conformance() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../conformance/manifest.json")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let id = c["id"].as_str().unwrap();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("conformance/cases")
            .join(id);
        let temp = Case::new();
        for file in c["files"].as_array().unwrap() {
            let file = file.as_str().unwrap();
            fs::copy(dir.join(file), temp.0.join(file)).unwrap();
        }
        let mut source = fs::read_to_string(temp.0.join("main.luma")).unwrap();
        let mut expected = String::new();
        for (key, v) in c["expected"]["bindings"].as_object().into_iter().flatten() {
            if !v.is_array() {
                source.push_str(&format!("\nprint({key})\n"));
                expected.push_str(
                    &v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string()),
                );
                expected.push('\n');
            }
        }
        temp.write("main.luma", &source);
        let result = temp.run();
        if c["expected"]["outcome"] == "static-error" || c["expected"]["outcome"] == "runtime-error"
        {
            let e = result.expect_err(id);
            assert!(
                e.diagnostics
                    .iter()
                    .any(|(_, d)| d.code == c["expected"]["diagnostic"].as_str().unwrap()),
                "{id}: {e:?}"
            );
        } else if matches!(id, "parallel-order" | "repeat-precheck") {
            assert!(
                result
                    .unwrap_err()
                    .diagnostics
                    .iter()
                    .any(|(_, d)| d.code == "E_RUNTIME_UNSUPPORTED"),
                "{id}"
            );
        } else {
            assert_eq!(result.unwrap(), expected, "{id}");
        }
    }
}
#[test]
fn execution_regressions() {
    for (source,expected) in [
 ("a=[[1,2]]\nb=a\nb[0][1]=9\nprint(a[0][1])\nprint(b[0][1])","2\n9\n"),
 ("x=10\nfn f(x)\nx=x+1\nreturn x\nprint(999)\nend\nprint(f(2))\nprint(x)","3\n10\n"),
 ("fn f(n)\nreturn match n\n0: 1\n*: n*f(n-1)\nend\nend\nprint(f(5))","120\n"),
 ("print(false and (1/0==0))\nprint(true or (1/0==0))\nprint(match -2\n< 0: 7\n*: 1/0\nend)","false\ntrue\n7\n"),
 ("fn print(x)\nreturn x+1\nend\nx=print(2)",""),
 ] { let c=Case::new(); c.write("main.luma",source); assert_eq!(c.run().unwrap(),expected); }
    for source in [
        "a=[1]\nprint(a[-1])",
        "a=[1]\na[0.5]=2",
        "a=[1]\na[2]=3",
        "print(1%0)",
        "print((-1)^0.5)",
    ] {
        let c = Case::new();
        c.write("main.luma", source);
        assert!(c.run().unwrap_err().runtime);
    }
    let c = Case::new();
    c.write("main.luma", "use lib\nprint(lib.f())");
    c.write("lib.luma", "pub fn f()\nreturn 1/0\nend");
    let e = c.run().unwrap_err();
    assert!(e.diagnostics[0].0.ends_with("lib.luma"));
    assert_eq!(e.diagnostics[0].1.span.start, 18);
}
#[test]
fn preflight_and_cli() {
    let c = Case::new();
    c.write("main.luma", "print(123)\nx=sum([1])");
    let e = c.run().unwrap_err();
    assert!(!e.runtime);
    assert_eq!(e.diagnostics[0].1.code, "E_RUNTIME_UNSUPPORTED");
    for (source, status) in [("print(42)", 0), ("print(unknown)", 1), ("print(1/0)", 3)] {
        c.write("main.luma", source);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_luma"))
            .arg("run")
            .arg(c.0.join("main.luma"))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(status));
        if status == 0 {
            assert_eq!(out.stdout, b"42\n");
        }
    }
    assert_eq!(
        std::process::Command::new(env!("CARGO_BIN_EXE_luma"))
            .arg("run")
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn imports_gates_and_limits() {
    let c = Case::new();
    c.write("main.luma", "use a\nuse b\nprint(a.value+b.value)");
    c.write("a.luma", "use shared\npub value=shared.value\n");
    c.write("b.luma", "use shared\npub value=shared.value\n");
    c.write("shared.luma", "print(5)\npub value=2\n");
    assert_eq!(c.run().unwrap(), "5\n4\n");
    c.write("main.luma", "print(match true\ntrue: 1\n*: sum([2])\nend)");
    let e = c.run().unwrap_err();
    assert!(!e.runtime);
    assert!(e.diagnostics[0].1.span.start > 0);
    c.write(
        "main.luma",
        "fn f(n)\nreturn match n\n0: 0\n*: f(n-1)\nend\nend\nprint(f(200))",
    );
    let e = c.run().unwrap_err();
    assert!(e.runtime);
    assert!(e.diagnostics[0].1.message.contains("depth"));
}
