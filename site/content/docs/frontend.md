---
title: Rust frontend — 0.1B
weight: 13
---

The Rust crate provides `lex(source)` and `parse(source)`, and a `luma check file.luma...` CLI. Build with `cargo build`; run with `cargo run -- check path/to/main.luma`. `luma version` reports the frontend version.

This page records the **0.1B historical syntax baseline**. In 0.1C, use `luma check --syntax-only file.luma` to select that behavior. The default now performs [bounded semantic analysis](../semantics/) across sibling imports and can export a versioned graph. There is no `run` command or object runtime. Statements below describing missing analysis refer to 0.1B.

## Review of 0.1A proposals

0.1B adopts the bounded lexical rules, newline/continuation rules, grammar, operator precedence and associativity, ASCII identifiers and string escapes from Draft 001. Imports must precede other top-level forms; public definitions, function bodies and object bodies follow the bounded grammar. Return outside a function is parsed and left for semantic analysis. Literal pattern types and wildcard exhaustiveness are also semantic checks.

Scope, monomorphic inference, initialization order, by-value calls, numeric operations, library contracts, module visibility/resolution and effect rules remain documented semantic proposals for the next stage. Open numeric and parallel-runtime decisions remain unresolved. No implementation result validates them.

## Source structure and diagnostics

Every token and AST node retains a half-open UTF-8 byte span. Keep the source file path alongside the returned tree: spans are relative to that file, with no global file registry. Identifiers, parameters, members, match arms, pipeline stages and grouped expressions retain structure for a future Graph Explorer. Numbers preserve decimal spelling; strings retain decoded values and original source spans. Pipelines remain explicit binary AST nodes and are not executed or desugared.

Diagnostics include a stable category, span and message; CLI output adds path and one-based line/Unicode-scalar column. Future VS Code integration must convert byte offsets to UTF-16 positions. Parsing stops at the first error. Expressions have a nesting limit of 128. No error recovery, incremental parsing, graph export, type analyzer or interpreter is provided.


## Validation

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `node tools/validate-conformance.cjs` are CI gates. The existing strict Hugo build remains enabled.

All 30 existing cases have separate frontend expectations in `conformance/frontend.tsv`, checked against the manifest by the integrity validator. Every listed file is parsed. Only lex-escape, parse-end and parse-braces fail the frontend. Semantic failures (including missing wildcard, invalid names and types) must parse successfully. Original bindings, static semantic errors and runtime errors remain unexecuted contracts.

Tests cover AST precedence, source spans, Unicode string contents, CRLF, comments, nested match, continuation, invalid syntax and CLI exit behavior.

