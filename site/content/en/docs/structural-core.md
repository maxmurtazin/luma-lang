---
title: "Structural Core S0/S1"
weight: 95
---

Luma provides a **read-only typed semantic snapshot** for future Studio and adapters. It reuses the existing checker without running the program.

Run **luma ir main.luma** to write schema-versioned JSON to stdout. Exit codes: 0 for a valid snapshot, 1 for source/import/semantic errors, 2 for incorrect usage. Invalid exports contain diagnostics and source files, but no usable typed graph.

The export includes structured expression/function types, resolved references, module links, source byte spans, semantic IDs, dependencies and conservative effects. File identity is separate from exact source content. IDs survive comments, whitespace and unrelated named declarations; renaming, moving and positional argument/arm changes can change them. This is not an API for editing or executing serialized programs.

Rust API: **luma::structural::analyze(path)**. Check schemaVersion == 1 and valid before consuming nodes. See [JSON Schema](https://github.com/maxmurtazin/luma-lang/blob/main/schema/structural-v1.json), the [S1 contract](https://github.com/maxmurtazin/luma-lang/blob/main/docs/structural-core-s1.md) and [S0 audit](https://github.com/maxmurtazin/luma-lang/blob/main/docs/structural-core-s0.md).

Effects report what the bounded checker knows; they do not prove termination or runtime success. CAD, Living IR integration, structural edits and Studio UI remain future work. Existing check/run, Graph Explorer and the current VSIX remain supported. Build current source for this command; the published 0.1.3 installer is unchanged.
