# Luma Language Tools & Graph Explorer 0.1.2

Install the VSIX in VS Code 1.90+. The standalone extension requires the Luma CLI; the Windows installer bundles both. Configure luma.executablePath or use the discovered default per-user install. Trust the workspace and save all .luma files.

Commands: Luma: Check File, Luma: Run File, Luma: Open Graph Explorer. Filter, zoom, pan and click graph nodes to navigate. Problems shows diagnostics; Luma output shows execution results. Saved imports refresh the graph; unsaved changes invalidate it.

All six 0.1D2 library operations execute. Processes are limited to 30 seconds and 16 MiB output. Debugging and visual editing are future work.

Node 22+: npm ci; npm test; npm run package. Output: luma-language-tools-0.1.2.vsix. Tests cover actual CLI graphs but do not launch an interactive Extension Host.

[English guide](https://maxmurtazin.github.io/luma-lang/en/docs/vscode/) · [Русское руководство](https://maxmurtazin.github.io/luma-lang/ru/docs/vscode/)
