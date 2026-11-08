---
title: "Parallel limits"
aliases: ["/docs/parallelism/"]
weight: 10
---

parallel accepts at most 256 elements, with up to eight native scoped threads over fixed contiguous chunks. Every element receives a cloned private engine snapshot and 100,000 expression steps. No state/output merges back; results preserve input order. Empty input starts no workers. Snapshot cloning can outweigh work; no speedup is promised.

The checker rejects captures, I/O, indirect calls, higher-order library operations and nested parallel callbacks, including transitive effects. Local by-value array mutation is allowed. Ordinary functions may call parallel. All workers join before return. Each chunk stops at its first failure; other chunks finish bounded work. Lowest failing input index wins. Panics become runtime failures. No user cancellation API or preemptive interruption; evaluation bounds are cooperative. Imported worker error attribution is limited to the enclosing call file.
