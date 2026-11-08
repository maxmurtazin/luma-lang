---
title: Functions
weight: 6
---

**Design decision:** functions are the main unit of behavior. Types are statically inferred, and function overloading is excluded.

**Proposed syntax — not runnable:**

```text
fn square(x)
    return x * x
end

answer = square(4)
```

`fn`, parameter lists, and explicit `return` form the current draft. The example expresses intent without specifying an inferred numeric type or multiplication behavior.

## Repetition through functions

The chosen direction is function-based repetition. Recursion and collection operations are possible mechanisms to investigate. Candidate notation such as `repeat(3, tick)` is not a settled API. No promise is made about tail calls, stack use, or recursion limits.

## Open questions

- Are functions first-class, and what function types can be inferred?
- Are anonymous functions or closures needed? How do they capture values?
- Are explicit type annotations supported or required in some contexts?
- Are recursion and mutual recursion permitted across inference boundaries?
- What happens when execution reaches the end without `return`?
- What are argument evaluation order, arity errors, and default-argument rules?

The lack of overloading does not itself decide whether generic functions exist.
