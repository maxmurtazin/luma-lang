# Luma Language Tools & Graph Explorer 0.1A

Install the VSIX with Extensions: Install from VSIX in VS Code 1.90+. Build the runtime separately with cargo build --release. Add target/release to PATH or set luma.executablePath to the absolute executable path (luma.exe on Windows). Paths with spaces work. The setting accepts an executable, not shell arguments. The standalone VSIX has no runtime. The [Windows installer](../../docs/windows-installation.md) bundles both and the default setting discovers its per-user runtime without changing settings. Execution requires a trusted workspace.

Save a local .luma file and use Luma: Check File, Luma: Run File or Luma: Open Graph Explorer. Problems uses CLI UTF-8 spans converted to editor positions, including sibling imports. Run output appears in the Luma output channel.

Graph Explorer displays exported relationships with direction labels. Filter by text or function/object/module category, zoom, drag to pan, and click a node or list button to navigate. Builtins have no source location. Saved file changes refresh the selected entry after luma.debounceMs (default 500). Save imported files too: the checker reads disk. Unsaved changes invalidate the graph until saved.

The viewer consumes schemaVersion 1 from luma check --graph. It slices source spans for labels and uses exported Function/Scope types for declaration categories. No semantic parser runs in TypeScript. The current schema uses sibling filenames and exposes expression nodes; large graphs may be slow. Partial graphs accompany checker errors. Debugging and visual editing are future work.

Runtime map, sum, parallel and repeat remain gated and cannot execute. Processes have a 30-second timeout and 16 MiB output limit.

## Development

Node.js 22+: npm ci, npm test, npm run package. Output: luma-language-tools-0.1.0.vsix. CI checks actual CLI output and packages on Windows, Linux and macOS. No production npm dependencies.

Manual release validation: install VSIX, configure CLI, check/run test/fixtures/main.luma, navigate function/object/module nodes, edit/save, and try invalid code and a missing executable. Unit tests do not launch an Extension Host.


0.1D2 / distribution 0.1.1: `map`, `filter`, `reduce`, `sum`, `repeat`, and CPU-threaded `parallel` now execute. The current contract and bounds are in [standard-library](../standard-library/) (Hugo docs: `site/content/docs/standard-library.md`); this supersedes earlier deferred-runtime statements. A4 pagination remains unverified.
