# 0.1A validation results — 2026-11-08

## 0.1C validation — 2026-11-08

Built on 4e3bfe6 on main. Implemented bounded static checking and graph schema v1; see the semantics documentation for conservative exclusions.

- PASS locally using Rust 1.99.1 Windows GNU: cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test --locked (five frontend and two semantic integration tests; semantic test checks all 30 manifest outcomes and graph determinism/endpoints).
- PASS: node tools/validate-conformance.cjs (30 fixture inventories).
- PASS: Hugo Extended 0.167.0 strict build (--gc --minify --panicOnWarning), 25 pages.
- PASS: git diff --check. Windows incremental-cache hard-link warnings fall back to copying; checks exit successfully.
- Default Windows MSVC compilation initially failed because link.exe is absent; installing the GNU toolchain enabled actual local test execution.
- A4 pagination remains unverified. The one-page syntax source is unchanged.
- No interpreter/runtime tests or execution results are claimed. Runtime-error fixtures pass static checking only.

Earlier results below are historical.

Baseline inspected: repository commit 66aaab6 and the referenced conversation's November 2026 Draft 001. No interpreter, Rust crate, analyzer or runtime was present.

| Check actually executed | Result |
| --- | --- |
| `node tools/validate-conformance.cjs` | PASS: 30 fixture manifests and source inventories; 13 accept, 15 static-error, 2 runtime-error; all required coverage areas present |
| `hugo --source site --gc --minify --panicOnWarning --cacheDir <absolute-workspace-cache>` | PASS: Hugo Extended 0.167.0, Hextra v0.13.0, 23 pages; no warnings |
| `git diff --check` | PASS; Git reports local LF/CRLF normalization notices |
| Luma lexer/parser/type/effect/runtime conformance | NOT EXECUTED: no implementation exists |
| Physical A4 pagination | NOT VERIFIED: retained as a compact-reference design goal |

The initial build attempt could not write the default theme cache. Using a workspace cache resolved that environment issue. The first strict render found unescaped generic type angle brackets in prose; escaping those corrected the render, and the strict build then passed.

The manifest validator only checks metadata, inventory and matrix consistency. It cannot establish that a program parses, infers or executes correctly. Fixture expected outcomes are specification assertions for future implementation. CI now runs this integrity validator before building documentation; no remote CI or Pages deployment result is claimed here.

Decision: bounded GO for reviewing the lexer/parser baseline. Labeled 0.1A proposals need design acceptance; remaining numeric, library and resource questions are explicit in the specification. Full language/runtime acceptance is pending. Next stage: 0.1B — Rust Lexer, Parser & AST.

## 0.1B validation — 2026-11-08

Baseline: f404373 on main. Adopted lexical/grammar proposals; semantic proposals and open runtime decisions remain documented in the frontend page.

- PASS: cargo fmt; cargo clippy --offline --all-targets -- -D warnings (environment emitted incremental-cache hard-link warnings).
- PASS: fixture integrity and independent frontend expectation inventory, 30 cases / every file.
- PASS: strict Hugo Extended 0.167.0 build, 24 pages.
- BLOCKED: cargo test cannot link because Windows MSVC link.exe is absent, including outside restricted execution. Tests type-check under clippy; execution is delegated to the Linux CI job and is not claimed locally.
- A4 physical pagination remains unverified; no rendered print artifact was produced.
- Semantic/runtime fixtures remain unexecuted. No interpreter exists.

Remote verification: [Rust frontend CI](https://github.com/maxmurtazin/luma-lang/actions/runs/42458555390) passed for ee1e08c, including formatting, clippy, fixture integrity and all five integration tests (one parses every file across 30 cases). This validates syntax behavior only; no semantic/runtime expectations were executed.
