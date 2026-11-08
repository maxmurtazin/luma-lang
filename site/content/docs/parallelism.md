---
title: Parallelism
weight: 12
---

**Draft 001 · Proposed, unimplemented.**

parallel is explicit synchronous array mapping with input-ordered results. Conservative transitive effect analysis rejects I/O, external mutation and unknown effects. Local by-value mutation is permitted; scheduling and performance are unspecified.

~~~text
fn square(x)
    return x * x
end
result = [1, 2, 3] -> parallel(square)
~~~

See [the formal specification]({{< relref "specification" >}}) and [conformance matrix]({{< relref "conformance" >}}) for scope, open decisions and expected behavior.
