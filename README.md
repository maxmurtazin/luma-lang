# Luma 0.1.2

A bounded Rust interpreter with static inference, functions, arrays, structural objects, sibling modules, match, pipelines, and map/filter/reduce/sum/repeat/isolated CPU parallel.

- [English documentation](https://maxmurtazin.github.io/luma-lang/en/)
- [Документация на русском](https://maxmurtazin.github.io/luma-lang/ru/)
- [Windows releases](https://github.com/maxmurtazin/luma-lang/releases)
- [Windows Actions artifacts](https://github.com/maxmurtazin/luma-lang/actions/workflows/windows-installer.yml)
- [Runnable examples and expected results](examples/manifest.json)

Build: stable Rust plus platform C/C++ linker, then cargo build --release --locked. Run target/release/luma run examples/library/main.luma (add .exe on Windows). Windows setup bundles CLI and VSIX; see [installation](docs/windows-installation.md).

Validate: cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test --locked; cargo build --locked; node tools/validate-conformance.cjs; node tools/test-examples.cjs. For the extension: npm ci, npm test, npm run package from editors/vscode.

Documentation uses Hugo Extended 0.167.0, Go 1.25.7 and pinned Hextra v0.13.0. Run hugo --source site --gc --minify --panicOnWarning. Both /en/ and /ru/ have matching guides and a language selector. GitHub Pages deploys main via Actions; configure Pages source as GitHub Actions. Update both translations together. Historical stage documents remain in spec/history.

Implementation limits: finite binary64 numbers, monomorphic callbacks, 100,000 expression steps, depth 32, repeat at most 10,000 transitions, parallel at most 256 elements/eight workers. Parallel callbacks exclude captures, I/O and higher-order effects. Output is buffered until success. No package manager, interactive I/O, classes, async, arbitrary indirect calls or debugger. See the published guides for full contracts.

Windows CI tests installation, repeated installation, CLI/graph export, isolated VSIX registration, user-file preservation and uninstall. Interactive Windows/VS Code behavior and migration from an older installer require separate manual validation.

## Structural Core S0/S1

Read-only typed export: `luma ir main.luma`. Rust API: `luma::structural::analyze(path)`. [S0 audit](docs/structural-core-s0.md), [S1 contract](docs/structural-core-s1.md), [JSON Schema](schema/structural-v1.json). Existing checker, runtime, Graph Explorer and VSIX remain supported. Build current source for this additive API; no new installer release is implied.
