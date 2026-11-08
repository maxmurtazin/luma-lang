---
title: "Runnable examples and diagnostics"
aliases: ["/docs/conformance/"]
weight: 13
---

The repository examples directory includes every conformance fixture as a standalone runnable project, including sibling modules and deliberately invalid programs. manifest.json records expected output or diagnostic for every entry. Run cargo build --locked then node tools/test-examples.cjs. Successful cases must match exact stdout; negative cases must match exit status and diagnostic. Tests use the actual CLI, not a second implementation.

Coverage: lexical comments/escapes, precedence, scopes, monomorphic types, arity, functions/captures, objects, array copies/bounds/empty inference, modules/public/private/cycles, pipeline argument insertion, lazy ordered match, all library calls, parallel ordering/effects and repeat precheck. See operations for every arithmetic/comparison/Boolean operator and library for filter/reduce/repeat.

Planned/unimplemented: classes, async, loops for/while, package management, general I/O, compound equality, arbitrary indirect calls, debugger and visual graph editing. Historical design documents in spec/history describe earlier stages and are not the current implementation guide.
