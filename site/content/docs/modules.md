---
title: Modules
weight: 12
---

**Draft 001 · Proposed, unimplemented.**

One file per module. use math imports math.luma; only pub definitions are externally accessible. Initialize once; reject cycles. The bounded resolver covers sibling files. Inference spans reachable modules.

~~~text
use math
answer = math.square(4)
~~~

See [the formal specification]({{< relref "specification" >}}) and [conformance matrix]({{< relref "conformance" >}}) for scope, open decisions and expected behavior.
