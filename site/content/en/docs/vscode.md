---
title: "VS Code Graph Explorer"
aliases: ["/docs/vscode/"]
weight: 12
---

Install the bundled VSIX in VS Code 1.90+ and trust the workspace for execution. Save a local .luma file; use Luma: Check File, Luma: Run File or Luma: Open Graph Explorer. Problems receives diagnostics; execution output appears in the Luma output channel. luma.executablePath accepts an executable path only, without shell arguments.

Graph Explorer shows directed semantic relationships. Filter by text/function/object/module, zoom, drag to pan and click nodes/list entries to navigate. Builtins have no source location. Saved edits refresh the selected entry after luma.debounceMs (500 by default); save imports too. Unsaved changes invalidate the graph until saved.

CLI graph schemaVersion 1 includes valid, producer, nodes and edges; relations include contains, imports, reads, resolves_to, calls and callback. Source spans are UTF-8 bytes converted by the extension to UTF-16 positions. Partial graphs can accompany errors: inspect valid. IDs based on spans change after edits; large graphs may be slow. No debugger or visual editing. Processes have a 30-second timeout and 16 MiB output limit. Unit tests do not exercise an interactive Extension Host.
