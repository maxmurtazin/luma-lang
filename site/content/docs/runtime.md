---
title: Execution core — 0.1D1
weight: 15
---

`luma run main.luma` checks the entire import graph, applies execution gates, then interprets the exact AST snapshot retained by semantic analysis. It uses the existing lexer and parser. `luma check` remains a static command; acceptance by that command alone does not promise that deferred library calls can run.

Supported: implicit local declarations and reassignment, Number/String/Bool, homogeneous arrays and nested indexing, by-value array copies and parameters, named functions, explicit returns, lexical captures, structural object fields and methods, sibling-file `use` and `pub`, literal/comparison `match`, sequential `->`, and scalar `print`. Functions without a return produce Unit. Only the selected match arm runs. Arrays are copied recursively; indexed writes require a local binding as enforced by the checker. Ordinary assignment declares or updates a binding in the current scope.

```text
fn square(x)
    return x * x
end
result = 4 -> square
print(result)
```

This prints `16` followed by a newline. Build with `cargo build`, or use `cargo run -- run main.luma`.

Evaluation follows source order, imports initialize depth first in `use` order, and each canonical module initializes once. Functions capture their defining scope. Call arguments evaluate left to right; a pipeline input evaluates before explicit stage arguments and becomes the first argument. Assignment evaluates its right side before index expressions. `and` and `or` short circuit. Public visibility, import cycles, initialization, monomorphic types and indirect-call restrictions use the existing checker rules.

Execution policy for this bounded implementation: Number uses finite IEEE-754 binary64 arithmetic and decimal output; integer precision is limited to 53 bits. Division and remainder by zero produce `E_DIV_ZERO`. Nonfinite literals/results produce `E_RUNTIME`. Indexing is zero based and requires a nonnegative integral Number within bounds; invalid indices and indexed writes produce `E_INDEX`. These numeric/resource choices describe this implementation, not a claim that all open numerical design questions are settled. Evaluation has a 100000-expression budget and a 32-call depth limit, reported as `E_RUNTIME`.

Output is collected deterministically and emitted only after successful execution. A failure discards collected output. This core does not stream or provide interactive input. The call depth limit is 32 to keep the guard within Windows stack limits.

Exit statuses: **0** success, **1** source/import/static error or preflight gate, **2** invalid command usage, **3** runtime error. Diagnostics include source file and half-open UTF-8 byte span. Runtime failures inside imported functions identify the defining file.

`sum`, `map`, `parallel` and `repeat` retain their static contracts but are deferred to **0.1D2**. Every resolved call to these builtins, including unused functions and unselected match arms in any imported file, causes `E_RUNTIME_UNSUPPORTED` before any execution. The diagnostic names the operation and asks the author to remove the call. User functions with these names follow normal shadowing rules. No parallel execution or full standard library is provided.

`tests/semantic.rs` retains independent static conformance coverage of all 30 original fixtures. `tests/runtime.rs` executes their accepted core cases, checks expected scalar bindings through appended print observations in temporary copies, verifies specified runtime errors and static rejections, and explicitly verifies gates for deferred fixtures. Additional regressions cover nested array copies/writes, lazy evaluation, recursion, local scopes, early return, builtin shadowing, imported error spans, and CLI statuses. Historical fixture metadata remains the design contract; it does not itself record execution success.

Validation commands: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --locked`, `node tools/validate-conformance.cjs`, and `hugo --source site --gc --minify --panicOnWarning`. Rust CI runs both semantic and runtime suites. A4 print pagination requires separate visual verification.
