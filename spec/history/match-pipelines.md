---
title: Match and pipelines
weight: 12
---

**Draft 001 · Proposed, unimplemented.**

x -> f(a) calls f(x, a). Arrows always call; assignment uses =. match tests in order, evaluates only one arm and requires a final wildcard; arm types unify.

~~~text
result = 10 -> square
status = match result
    > 30: "Hot"
    *: "Cold"
end
~~~

See [the formal specification]({{< relref "specification" >}}) and [conformance matrix]({{< relref "conformance" >}}) for scope, open decisions and expected behavior.
