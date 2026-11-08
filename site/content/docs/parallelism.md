---
title: Parallelism
weight: 9
---

**Design decision:** parallel work must be requested explicitly. No async syntax is planned. The interpreter is intended to be written in Rust; this does not imply Rust's ownership or concurrency rules become Luma's rules.

**Proposed syntax — not runnable:**

```text
fn square(x)
    return x * x
end

numbers = [1, 2, 3, 4]
numbers -> parallel(square) -> result
```

The sketch requests parallel application of a function. `parallel` is a candidate API, not a working primitive. It does not guarantee a particular result order, speedup, thread count, or execution strategy.

## Open questions

- Is work scheduled on threads, tasks, a pool, or another mechanism?
- Are results ordered by input, completion time, or a separate rule?
- Which values may be shared, captured, or mutated safely?
- Must functions be pure, and how are side effects constrained?
- How are errors, partial results, cancellation, and nested parallel calls handled?
- What bounds memory use and the number of workers?

Correct sequential semantics and a clear error model should precede a parallel prototype. Performance and safety claims require an implementation and measurements.
