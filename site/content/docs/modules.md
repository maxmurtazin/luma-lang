---
title: Modules
weight: 4
---

**Design decision:** one source file is one module. `pub` marks an export concept and `use` expresses an import concept.

**Proposed syntax — not runnable:**

```text
pub fn square(x)
    return x * x
end
```

This could live in `math.luma`. A second file could refer to it as follows:

```text
use math
answer = math.square(4)
```

The file-to-module direction is settled; this qualified-call spelling is a candidate.

## Open questions

- Are unmarked declarations private, and can data and objects be exported?
- How are local files, directories, packages, and standard modules resolved?
- Are aliases, selective imports, and re-exports needed?
- Are import cycles allowed? When are top-level expressions evaluated?
- How does type inference operate across module boundaries?

There is no package manager, module loader, package manifest, or stable standard import path yet.
