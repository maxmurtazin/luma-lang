---
title: CLI and VS Code roadmap
weight: 11
---

No interpreter, CLI, language server, or VS Code extension is available. This roadmap is ordered by dependency, with no dates or release commitments.

## 0.1A — Syntax and Semantics Validation

Draft 001 and 30 semantic fixtures now provide a bounded specification baseline. Fixture integrity is runnable; interpreter semantics remain unexecuted. Review the labeled bounded proposals before implementation.

## 0.1B — Rust Lexer, Parser & AST

Resolve block parsing, value types, bindings, function calls, evaluation order, and errors. Turn accepted examples into fixtures. Record each decision and its remaining constraints.

## 2. Build the Rust interpreter

Implement a lexer and parser with source locations, static type inference, diagnostics, and an evaluator. Begin with sequential functions and values, then add modules and the agreed object model. An interpreter is the first execution target.

## 3. Introduce a CLI

Script execution and useful diagnostics are the initial goals. A command such as `luma run main.luma` is only a candidate spelling; there is no executable to invoke. REPL, formatting, checking commands, exit codes, and distribution format remain open.

## 4. Add collections and explicit parallel work

Specify library operations and their tests before exposing stable APIs. Define failure propagation and shared-state rules before promising parallel execution.

## 5. Develop VS Code support

Start with file recognition, syntax highlighting, and diagnostics once the grammar is stable enough. Navigation, completion, and run integration follow interpreter support. Whether to use a language server is an implementation decision still to make.

## 6. Explore the program graph

The future visualizer should help readers inspect relationships among functions, modules, objects, and data flow. Candidate interactions include clicking a node to open its source and selecting source to highlight a node. Dependency, data-flow, and control-flow views are possible separate views, not implemented features.

Graph editing, runtime tracing, and graph-directed control flow need separate design decisions. Visualization alone does not define execution semantics.

## Contributing now

Open a focused issue or propose a documentation change in the repository. State the problem, a small example, alternatives, and unresolved semantics. Keep accepted direction distinct from a proposal. Once an interpreter exists, label behavior as implemented only with corresponding code and verification.
