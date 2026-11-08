---
title: Match and pipelines
weight: 8
---

**Design decision:** include `match` and use a single arrow spelling, `->`. The exact grammar and execution model are open.

## Match

**Proposed syntax — not runnable:**

```text
match value
    0 -> "zero"
    _ -> "other"
end
```

This proposes literal and fallback arms. It does not settle `_` as a wildcard, whether match yields a value, whether arms are ordered, or how exhaustiveness is checked. Destructuring, guards, and binding patterns remain open.

## Pipelines

**Proposed syntax — not runnable:**

```text
numbers
    -> map(square)
    -> filter(positive)
    -> result
```

This sketches a readable sequence of transformations. `map` and `filter` are candidate library operations; `square` and `positive` would be user functions. The final `result` sketches a destination name, but terminal binding has not been adopted as a semantic rule.

## Open questions

- Which argument receives the incoming value?
- Does every arrow mean function application, or can the last stage bind a name?
- How are multiline pipelines parsed and associated with surrounding blocks?
- How do failures and side effects propagate?
- How is match-arm syntax distinguished from pipeline syntax?

The future graph visualizer may display these relationships. Graph-directed control transfers are an exploratory idea, not an accepted `goto` feature or a defined execution model.
