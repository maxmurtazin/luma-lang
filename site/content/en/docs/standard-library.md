---
title: "Standard library"
aliases: ["/docs/standard-library/"]
weight: 9
---

All calls below execute in 0.1.2. Pipelines insert the first argument; local declarations can shadow builtin names.

| Call | Contract |
| --- | --- |
| print(scalar) | Buffer a scalar and newline; return Unit |
| map(array, f) | Unary callback, input order, homogeneous result |
| filter(array, predicate) | Unary Bool predicate, preserve order |
| reduce(array, initial, f) | Left fold f(accumulator, element); empty returns initial |
| sum(array) | Sequential Number addition from positive zero |
| repeat(seed, step, done) | Test done before each step; return first satisfying state |
| parallel(array, f) | Synchronous isolated CPU mapping, results in input order |

Callbacks are monomorphic; reduce may use an accumulator type different from the element. sum never reassociates arithmetic and rejects nonfinite results. Sequential callbacks may print; failure stops the operation and discards all buffered output.

repeat allows at most 10,000 transitions and a final predicate test; a true initial predicate invokes no step. The ordinary 100,000-expression budget and depth 32 may fail earlier. No convergence guarantee.
