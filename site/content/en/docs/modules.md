---
title: "Modules"
aliases: ["/docs/modules/"]
weight: 7
---

use math resolves sibling math.luma; external access requires pub on the declaration. Imports initialize depth first in use order and each canonical module initializes once. Missing files, cycles and private access fail. Type inference covers the reachable graph. Package management and resolution across directories are unsupported. See examples/module-public for separate main.luma and math.luma files.
