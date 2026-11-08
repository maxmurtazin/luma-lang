# 0.1A validation results — 2026-11-08

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
