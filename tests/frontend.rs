use luma::{parse, Kind};
#[test]
fn fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = include_str!("../conformance/frontend.tsv");
    assert_eq!(
        cases
            .lines()
            .map(|l| l.split('\t').next().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        30
    );
    for row in cases.lines() {
        let fields: Vec<_> = row.split('\t').collect();
        let (id, file, expected) = (fields[0], fields[1], fields[2]);
        let source =
            std::fs::read_to_string(root.join("conformance/cases").join(id).join(file)).unwrap();
        let result = parse(&source);
        if matches!(id, "lex-escape" | "parse-end" | "parse-braces") {
            let d = result.expect_err(id);
            assert_eq!(d.code, expected);
            assert!(d.span.end <= source.len());
        } else {
            assert!(result.is_ok(), "{id}: {result:?}");
        }
    }
}
#[test]
fn precedence_and_spans() {
    let ast = parse("x = -2 ^ 3 ^ 2 + 4 * 5\n").unwrap();
    let Kind::Assign(_, value) = &ast[0].kind else {
        panic!()
    };
    let Kind::Binary(op, left, right) = &value.kind else {
        panic!()
    };
    assert_eq!(op, "+");
    assert!(matches!(right.kind, Kind::Binary(ref op, ..) if op == "*"));
    let Kind::Unary(_, power) = &left.kind else {
        panic!()
    };
    let Kind::Binary(op, _, exponent) = &power.kind else {
        panic!()
    };
    assert_eq!(op, "^");
    assert!(matches!(exponent.kind, Kind::Binary(ref op, ..) if op == "^"));
    assert_eq!(ast[0].span, luma::Span { start: 0, end: 22 });
}
#[test]
fn grammar_boundaries() {
    for s in [
        "a = 1 < 2 < 3",
        "a = [1,]",
        "a.b = 1",
        "fn f() return 1 end",
        "x = 1 -> 2",
        "x = 1;",
        "x = ([1)]",
        "fn f()\nfn g()\nend\nend",
        "object O\nprint(1)\nend",
        "pub return 1",
    ] {
        assert!(parse(s).is_err(), "{s}");
    }
    for s in [
        "x = [\n1,\n2\n]\r\ny = x ->\n# continued\nf",
        "x = match 1\n*: match 2\n*: 3\nend\nend",
        "return",
        "pub object O\nx = true\nend",
        "use math\nx = math.f(1)[0]",
    ] {
        assert!(parse(s).is_ok(), "{s}: {:?}", parse(s));
    }
}
#[test]
fn utf8_and_strings() {
    let src = "x = \"é\\n#\"\r\ny = 1";
    let tokens = luma::lex(src).unwrap();
    assert_eq!(tokens[2].value.as_deref(), Some("é\n#"));
    assert_eq!(luma::position(src, src.find('y').unwrap()), (2, 1));
    for s in ["x = \"bad\\q\"", "x = \"bad", "x = \"a\nb\"", "é = 1"] {
        assert!(parse(s).is_err());
    }
}
#[test]
fn cli_is_syntax_only() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luma"))
        .args([
            "check",
            "--syntax-only",
            "conformance/cases/scope-unknown/main.luma",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Syntax-only"));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luma"))
        .args(["check", "conformance/cases/parse-end/main.luma"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("E_END"));
}
