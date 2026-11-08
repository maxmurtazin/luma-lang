---
title: Luma
description: Programming, without the noise. An early language design built around functions and explicit data flow.
---

# Programming, without the noise.

Luma is a planned minimal procedural language built around functions, data flow, and explicit parallelism.

**Early design · Interpreter-first · Rust**

{{< callout type="warning" >}}
Luma is not implemented yet. This site records design decisions and proposals. No interpreter, standard library, or VS Code extension is available. All Luma examples are illustrative and cannot be run.
{{< /callout >}}

```text
fn square(x)
    return x * x
end

numbers = [1, 2, 3, 4]
result = numbers -> parallel(square)
```

*Proposed syntax.* The example sketches explicit parallel work. Draft 001 defines first-argument piping and ordered synchronous results; scheduling remains unspecified.

{{< cards >}}
  {{< card link="docs/getting-started" title="Start here" subtitle="Understand the project status and read your first draft." >}}
  {{< card link="docs/syntax" title="Syntax at a glance" subtitle="A one-page sketch, with unresolved rules called out." >}}
  {{< card link="docs/language-design" title="Language design" subtitle="The decisions that guide Luma, and questions still to answer." >}}
{{< /cards >}}

## A small set of ideas

Ordinary names and inferred static types. Functions as the main unit of behavior. Files as modules. Objects that group related data and functions. One arrow, `->`, for the proposed flow notation. Parallel work requested explicitly.

## Help shape the language

Read the [roadmap]({{< relref "docs/tooling" >}}), then discuss a focused design question in the [repository](https://github.com/maxmurtazin/luma-lang). Prefer examples and tradeoffs over adding syntax before its behavior is understood.
