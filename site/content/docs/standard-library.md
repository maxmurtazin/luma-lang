---
title: Standard library
weight: 10
---

There is no implemented standard library. The following is a planning map, not an API reference. Module names and function names are candidates.

| Candidate area | Intended purpose | Questions before specification |
| --- | --- | --- |
| Core | Basic output and common value operations; candidate `print` | Formatting, output streams, errors, conversions |
| Math | Basic numeric calculations | Numeric model, precision, domains, overflow |
| Array | Transform and select collections; candidate `map`, `filter`, fold | Element typing, eagerness, order, mutation |
| String | Text operations | Encoding, Unicode units, indexing, normalization |
| File | Read and write external data | Paths, encoding, resource lifetime, failures |
| Parallel | Explicit parallel operations; candidate `parallel` | Ordering, scheduling, shared state, cancellation |

## API design process

For each proposed operation, first define inputs and outputs, inferred types, evaluation order, side effects, and failure behavior. Only then add examples that can become implementation tests. Stable import paths and signatures must wait for the module and type-system decisions.

The short names used elsewhere on this site communicate design intent. They do not establish built-ins or reserve identifiers.
