---
title: Arrays
weight: 5
---

Arrays are intended as a simple way to work with collections. Their type and storage model is not specified.

**Proposed syntax — not runnable:**

```text
numbers = [1, 2, 3, 4]
numbers -> map(square) -> squares
```

Bracket literals and a `map` operation are candidates. `square` would be a user-defined function. Pipeline argument placement and result binding remain proposals; this example defines no traversal guarantees.

## Function-based repetition

Collection functions are part of the intended approach to repeated work. Candidate operations include `map`, `filter`, and a fold or reduction. Their names, signatures, and evaluation rules are open. The language does not yet provide any of them.

## Open questions

- Must all elements have the same inferred type? How is an empty literal typed?
- Are arrays fixed-size, growable, mutable, or persistent?
- What are the indexing, bounds, slicing, and iteration rules?
- Are transformations eager or lazy, and what is their evaluation order?
- How do nested arrays and multidimensional data behave?
- What copying or sharing occurs when arrays cross function boundaries?
