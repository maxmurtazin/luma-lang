---
title: Arrays
weight: 12
---

**Draft 001 · Proposed, unimplemented.**

Homogeneous dynamic-length Array&lt;T&gt;; indexes start at zero. Empty arrays require element-type context. Bounds errors occur at runtime; by-value function arguments protect caller arrays.

~~~text
values = [10, 20, 30]
values[0] = 15
~~~

See [the formal specification]({{< relref "specification" >}}) and [conformance matrix]({{< relref "conformance" >}}) for scope, open decisions and expected behavior.
