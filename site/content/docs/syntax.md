---
title: One-page syntax draft
weight: 2
---

{{< callout type="warning" >}}
Every example below is proposed syntax, not executable Luma. This page is a design sketch, not a grammar or compatibility promise.
{{< /callout >}}

## Names and simple values

```text
count = 3
title = "Luma"
numbers = [1, 2, 3]
```

No `let` or `var`. Static type inference is the direction. Mutability, numeric types, string rules, array element rules, and statement boundaries are open.

## Functions and blocks

```text
fn square(x)
    return x * x
end
```

An indentation-only alternative is also intended:

```text
fn square(x)
    return x * x
```

How a parser selects a block form, handles nesting, or permits mixing is unresolved. Explicit `end` is used in most drafts to make their boundaries visible. Functions do not overload.

## Files as modules

Candidate contents of `math.luma`:

```text
pub fn square(x)
    return x * x
end
```

Candidate contents of `main.luma`:

```text
use math
answer = math.square(4)
```

`pub` and `use` are chosen concepts. Import paths, qualified access, lookup, and visibility defaults still need specification.

## Structural objects

```text
object Building
    height = 25
    floors = 8
end
```

An object groups related data and functions without classes. Member lookup, capture, identity, and mutability are open.

## Match and one arrow

Candidate match-arm notation:

```text
match value
    0 -> "zero"
    _ -> "other"
end
```

Candidate pipeline notation:

```text
numbers -> map(square) -> result
```

The arrow spelling is `->`. Patterns, wildcard spelling, exhaustiveness, arm values, argument placement, and terminal binding are unfinalized. `map` is a candidate library function.

## Repetition and parallel work

Candidate function-based repetition and explicit parallel transformation:

```text
repeat(3, tick)
numbers -> parallel(square) -> result
```

`repeat`, `tick`, and `parallel` illustrate intent, not defined APIs. No async syntax is planned. Scheduling, shared state, result ordering, recursion limits, and cancellation are open.
