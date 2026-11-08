---
title: Objects
weight: 12
---

**Draft 001 · Proposed, unimplemented.**

Named structural scopes group values and functions. Functions read sibling bindings lexically. No implicit receiver, instances, constructors, inheritance or first-class object identity.

~~~text
object Building
    height = 25
    fn volume(area)
        return area * height
    end
end
result = Building.volume(120)
~~~

See [the formal specification]({{< relref "specification" >}}) and [conformance matrix]({{< relref "conformance" >}}) for scope, open decisions and expected behavior.
