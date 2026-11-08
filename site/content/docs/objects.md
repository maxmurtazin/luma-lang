---
title: Objects
weight: 7
---

**Design decision:** an object is a structural grouping of related data and functions. A building, graph, table, or phenomenon can be modeled this way. Luma has no classes or class hierarchy.

**Proposed syntax — not runnable:**

```text
object Building
    height = 25
    floors = 8

    fn volume(area, height)
        return area * height
    end
end
```

The draft groups facts and a calculation under a name. The calculation takes an explicit height parameter so the example does not depend on an undecided member-capture rule. Dot access such as `Building.height` and `Building.volume(120, 25)` is a candidate spelling.

A table or graph would be a data model built using ordinary values, not automatically a specialized runtime object with database or graph operations.

## Open questions

- Is an object a value, a namespace, an identity-bearing entity, or some combination?
- Can objects be copied, nested, passed to functions, or created dynamically?
- How are structural types inferred and compared?
- How do members become visible, and can functions capture sibling members?
- Can members change? How is access coordinated during parallel work?

Constructors, implicit receivers, inheritance, and method dispatch must not be inferred from this draft.
