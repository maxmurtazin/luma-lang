---
title: 0.1A conformance matrix
weight: 12
---

**30 semantic specification fixtures; zero interpreter executions.** Positive cases assert final module bindings; negative cases assert static or runtime diagnostic categories. Each folder under conformance/cases/ is a complete conceptual program with an entry module and any imports. Stable diagnostic identifiers are fixture categories; diagnostic wording is not prescribed. File and source location are required when available.

| Fixture | Area | Expected | Execution |
|---|---|---|---|
| lex-comments | lexical | accept | Not executed |
| lex-string | lexical | accept | Not executed |
| lex-escape | lexical | static-error | Not executed |
| parse-end | parser | static-error | Not executed |
| parse-braces | parser | static-error | Not executed |
| precedence | parser | accept | Not executed |
| scope-local | scope | accept | Not executed |
| scope-unknown | scope | static-error | Not executed |
| type-reassign | type-inference | static-error | Not executed |
| type-monomorphic | type-inference | static-error | Not executed |
| module-public | module-imports | accept | Not executed |
| module-private | module-imports | static-error | Not executed |
| module-cycle | module-imports | static-error | Not executed |
| match-first | match | accept | Not executed |
| match-lazy | match | accept | Not executed |
| match-exhaustive | match | static-error | Not executed |
| match-types | match | static-error | Not executed |
| pipe-chain | pipelines | accept | Not executed |
| pipe-first-argument | pipelines | accept | Not executed |
| pipe-destination | pipelines | static-error | Not executed |
| object-capture | objects | accept | Not executed |
| array-copy | arrays | accept | Not executed |
| array-heterogeneous | arrays | static-error | Not executed |
| array-bounds | arrays | runtime-error | Not executed |
| parallel-order | parallel-effects | accept | Not executed |
| parallel-transitive-io | parallel-effects | static-error | Not executed |
| repeat-precheck | repetition | accept | Not executed |
| divide-zero | errors | runtime-error | Not executed |
| array-empty | arrays | static-error | Not executed |
| arity | functions | static-error | Not executed |

Run node tools/validate-conformance.cjs from the repository root. It checks manifest shape, file inventory, categories, outcomes, expected bindings/diagnostics and generated matrix coverage. It does not lex, parse, infer types or evaluate Luma. Node.js 22+ is a validation-tool dependency only.

No interpreter runner adapter is provided until a real interpreter and diagnostic protocol exist. A future adapter must check exit status, phase, diagnostic location/category and exact expected bindings, record interpreter revision and distinguish skips from passes. Runtime-error cases must pass static analysis first.

Read [Draft 001]({{< relref "specification" >}}). **Bounded GO for parser implementation review**; numeric edge cases and remaining APIs still prevent full runtime acceptance. Actual checks are recorded in spec/validation-results.md.
