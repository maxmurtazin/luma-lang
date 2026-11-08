---
title: Standard library
weight: 12
---

Luma 0.1D2 (distribution 0.1.1) executes these ordinary builtin calls. Pipelines insert their input as the first argument. Local declarations can shadow builtin names.

| Call | Contract |
| --- | --- |
| map(array, f) | Apply unary f in input order; return an array. |
| filter(array, predicate) | Keep elements whose unary predicate returns Bool true, preserving order. |
| reduce(array, initial, f) | Left fold: f(accumulator, element); empty arrays return initial. |
| sum(array) | Left fold of Number addition from positive zero. |
| repeat(seed, step, done) | Test done before each step; return first state satisfying done. |
| parallel(array, f) | Synchronous isolated CPU worker mapping; indexed input order. |

~~~text
fn square(x)
    return x * x
end
print([1, 2, 3] -> parallel(square) -> sum)
~~~

Array types are homogeneous and callback types monomorphic. reduce may use a different accumulator type. Empty arrays need enough context to infer their element type. sum uses finite IEEE 754 binary64 values, sequential addition, and rejects nonfinite results; reassociation is never performed. Division/remainder by zero fail.

repeat allows at most 10,000 transitions, with a final predicate test. A true initial predicate invokes no step. A false final predicate fails; the ordinary 100,000 expression budget and depth limit 32 can fail earlier. No convergence is assumed.

Sequential callbacks may print; returned output is buffered and discarded on failure. Errors stop sequential operations. No partial result is returned.

parallel accepts at most 256 elements and starts up to eight native scoped threads (one per fixed contiguous chunk). Every element gets a private cloned engine snapshot and 100,000 expression steps. No state or output is merged back. This bounds work independently of scheduling. No speedup is promised; snapshot cloning can dominate small workloads. Empty input starts no workers.

The checker rejects parallel callbacks with captures, I/O, indirect calls or higher-order library operations, including transitive effects. Local array mutation is by value. Callbacks containing parallel operations are conservatively rejected by effect analysis; ordinary functions may invoke parallel. All workers are joined before returning, including on failure. Each chunk stops on its first failure; other chunks finish their bounded work. The lowest failing input index wins. A worker panic becomes a runtime failure. There is no user cancellation API or preemptive interruption; cancellation is cooperative through bounded evaluation. Worker failures carry source spans; imported callback file attribution remains limited to the enclosing call file in parallel mode.

A4 physical pagination remains unverified. No syntax was added.

