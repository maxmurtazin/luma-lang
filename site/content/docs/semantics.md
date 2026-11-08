---
title: Static analysis — 0.1C
weight: 14
---

`cargo run -- check main.luma` parses the entry and reachable sibling imports, resolves lexical names and checks monomorphic types. `--syntax-only` retains 0.1B behavior. `cargo run -- check --graph graph.json main.luma` writes a deterministic JSON graph, including on static failure (`valid: false`). Exit status is nonzero on diagnostics or output failure. No code executes and no runtime/interpreter is supplied.

## Adopted semantic boundary

0.1C adopts Draft 001's bounded proposals for local assignment scopes, same-type reassignment, exact arity, one signature per function, forward function names, homogeneous arrays, pipeline first-argument insertion, final wildcard match fallback, sibling imports, `pub`, and lexical structural objects. User function constraints are solved across the import graph, including recursive calls; conflicting calls fail in either source order. Unresolved types (including unused unconstrained functions and empty arrays without context) fail with `E_INFERENCE`. Recursive infinite types fail the occurs check. Unit results and implicit function-end returns participate in inference.

`use math` resolves only sibling `math.luma`. Imports are checked once in dependency/source order; cycles, missing files, duplicate names and private access fail. Objects expose declared fields and methods with lexical sibling reads and no implicit self. Locals are collected before checking a function: reads before local assignment cannot fall back to an outer name. Captured values must already be initialized when their function declaration is analyzed; forward captures are conservatively rejected. Direct calls also check transitive capture initialization at their source position.

Static library contracts cover direct `print`, `sum`, `map`, `parallel`, and `repeat` calls only. They describe types, not implementations. Print accepts Number, String or Bool. Arithmetic and ordered comparisons require Number; equality is bounded to identical scalar types. Other library APIs, compound equality, first-class namespaces, and indirect user calls lack supported contracts and are rejected. Named user callbacks work through the bounded library contracts. Builtin function values are unsupported. No syntax was added; the compact syntax reference is unchanged.

## Conservative effects

The checker propagates effects through direct calls to a fixed point, including recursive call groups. Parallel requires a statically named callback. I/O, all captured data reads, and indirect or higher-order effects are conservatively unsafe. Thus even an immutable capture is rejected pending a stronger stability proof. Wrappers containing map/repeat/parallel are conservatively unsafe as parallel workers. Local array writes and by-value parameter writes are allowed; indexed writes to outer bindings fail. This establishes a bounded static rejection rule, not scheduling, isolation, copying or runtime execution behavior.

## Graph version 1

The envelope has `schemaVersion: 1`, `producer`, `valid`, `nodes`, and `edges`. Module IDs are `file:<sibling filename>`; source IDs are `<filename>#<start>:<end>:<role>`. Builtins use `builtin:<name>`. IDs are independent of checkout path and traversal order for identical sources in a single sibling directory. Byte-span IDs can change after edits; persistent edit tracking is deferred. Symlink aliases and multiple-directory/package resolution are outside the supported boundary.

Source nodes carry `file`, half-open UTF-8 byte `start`/`end`, and one-based Unicode-scalar `line`/`column`. Definition and expression nodes carry inferred type descriptions. Scope numbers and Debug type strings are informational, not a type serialization protocol. Editors must convert UTF-8 spans to UTF-16 positions. Module and builtin nodes have no source span. Edges include `contains`, `imports`, `reads`, `resolves_to`, `calls` and `callback`. Every endpoint exists; collections have deterministic ordering. This is a semantic/dependency graph, not a control-flow graph or an implemented VS Code extension.

Diagnostics retain filename, stable category and source byte span. Imported file failures are included. Analysis can produce follow-on diagnostics; graph consumers must inspect `valid` before treating types as established.

## Validation

Syntax tests independently check all files listed in `frontend.tsv`. Semantic integration tests read all 30 manifest entries: lexical/parser failures retain their categories, semantic failures require the specified category, and accept/runtime-error fixtures must pass static checking. Runtime-error fixtures passing static analysis do not verify their runtime result. Regression tests cover scope shadowing, duplicates, return placement, call-order conflicts, inference and infinite types; graph tests check determinism and endpoint integrity. CI runs formatting, clippy, tests, fixture inventory validation and uploads a sample graph. Documentation retains its strict Hugo build.

A4 physical pagination remains unverified. Numeric runtime representation, overflow, dynamic bounds, division by zero, parallel scheduling and execution remain deferred.
