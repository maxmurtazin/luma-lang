---
title: VS Code extension
---

Luma VS Code 0.1A includes syntax highlighting, Check File diagnostics, Run File and a read-only interactive Graph Explorer with source navigation. See the [extension README](https://github.com/maxmurtazin/luma-lang/tree/main/editors/vscode) for installation and limitations.

Install the VSIX from a successful VS Code extension workflow artifact. Build Luma separately and configure luma.executablePath or PATH; the extension does not bundle a runtime. Save a local .luma file, then use Luma: Open Graph Explorer. Filter nodes, pan/zoom, and click to navigate. Saved changes refresh automatically.

The graph comes from the Rust checker. Unsaved edits invalidate navigation until saved. Partial graphs may accompany diagnostics. Runtime map, sum, parallel and repeat remain unsupported for execution. Debugging and visual editing are future work.
