---
title: Language design
weight: 3
---

## Chosen direction

These are design decisions, not implemented capabilities.

| Area | Direction |
| --- | --- |
| Implementation | Interpreter first, written in Rust. |
| Type system | Static type inference. |
| Bindings | Names without `let` or `var`. |
| Values | Keep the user-facing number and string model simple. |
| Functions | Primary unit of behavior; no overloading. |
| Organization | One file is one module; `pub` and `use` express visibility and imports. |
| Objects | Structural grouping of related data and functions; no classes. |
| Blocks | Support indentation and explicit `end` forms; exact interaction remains open. |
| Flow | A single arrow spelling, `->`; include `match`. |
| Repetition | Express repetition through functions. |
| Parallelism | Explicit parallel work, without async syntax. |
| Editor | Future VS Code support and a program graph visualizer. |

## Keep semantics explicit

Static inference does not decide numeric representation, conversions, unions, generics, or inference boundaries. A simple number model does not imply arbitrary precision, floating-point defaults, or automatic coercion. Objects do not imply class constructors, inheritance, or a hidden receiver.

## Open questions

- Are bindings mutable, and how are reassignment and shadowing distinguished?
- What numeric representations, overflow rules, division rules, and conversions apply?
- How are strings encoded, indexed, escaped, and compared?
- Where is type inference local or global, and are annotations needed at module boundaries?
- What are the ownership, lifetime, and memory-management rules?
- How do errors propagate and how does a program recover?
- Can indentation and `end` coexist within one file? What closes a nested block?
- What evaluation order and side-effect model apply?

Each question needs a separate decision with examples before it becomes a normative rule. This documentation deliberately does not supply defaults.
