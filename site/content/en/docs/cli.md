---
title: "CLI and execution"
aliases: ["/docs/runtime/", "/docs/frontend/", "/docs/semantics/", "/docs/tooling/"]
weight: 11
---

Commands: luma version (or --version); luma run main.luma; luma check main.luma; luma check --syntax-only main.luma; luma check --graph graph.json main.luma. Options must follow this order. check can accept multiple files without graph; graph requires one entry and cannot combine with syntax-only.

run checks the entire import graph and executes the retained AST. check never executes; acceptance does not guarantee freedom from runtime errors. Arguments evaluate left to right. Output is buffered and emitted only on success, not streamed. No interactive input. Execution limits are 100,000 expressions and call depth 32.

Exit codes: 0 success; 1 source/import/static failure; 2 invalid usage; 3 runtime failure. Semantic/runtime diagnostics include the file and half-open UTF-8 byte span; syntax-only uses line/column. E_DIV_ZERO is division/remainder by zero, E_INDEX invalid indexing, E_RUNTIME resource/nonfinite failures.
