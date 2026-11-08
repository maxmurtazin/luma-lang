---
title: Getting started
weight: 12
---

Build the Rust execution core with `cargo build --release`. Save the following as `hello.luma`:

```text
print("Hello, Luma")
```

Run `cargo run -- run hello.luma`, or `target/release/luma run hello.luma`. Check without execution with `luma check hello.luma`.

Read [execution core]({{< relref "runtime" >}}) for supported behavior, diagnostics and deferred features, and [the specification]({{< relref "specification" >}}) for the broader design.
