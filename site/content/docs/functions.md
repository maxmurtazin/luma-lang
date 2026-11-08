---
title: Functions
weight: 12
---

**Draft 001 · Proposed, unimplemented.**

Functions are the only user-defined executable units. One inferred monomorphic signature per function; exact arity, lexical scope, explicit return, recursion, and Unit fallthrough. repeat(initial, step, predicate) checks before stepping.

~~~text
fn square(x)
    return x * x
end
result = 5 -> square
~~~

See [the formal specification]({{< relref "specification" >}}) and [conformance matrix]({{< relref "conformance" >}}) for scope, open decisions and expected behavior.
