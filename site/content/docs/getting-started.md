---
title: Getting started
weight: 1
---

## Current status

There is no Luma installation command, executable, package, REPL, or extension yet. Rust is the chosen implementation language for an interpreter-first implementation. A compiler is not the initial delivery target.

## Read a first program

**Proposed syntax — not runnable:**

```text
fn greet(name)
    return "Hello, " + name
end

message = greet("world")
print(message)
```

This sketch introduces a named function, a parameter, a returned expression, a name without `let` or `var`, and a call. String concatenation with `+` and `print` are candidate operations; their signatures and behavior are unfinalized. No output is guaranteed by an implementation.

## What to read next

1. Review the [syntax draft]({{< relref "syntax" >}}).
2. Read [functions]({{< relref "functions" >}}) and [modules]({{< relref "modules" >}}).
3. Explore [match and pipelines]({{< relref "match-pipelines" >}}).
4. Consult the [tooling roadmap]({{< relref "tooling" >}}).

## Work on the documentation

The repository README contains the actual Hugo setup and preview commands. These commands build this website; they do not install or execute Luma.
