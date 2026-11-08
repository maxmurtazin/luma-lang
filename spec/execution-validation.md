# 0.1D1 execution validation

Starting point: fetched `main` at `c607469` (clean checkout).

Local Windows validation on 2026-11-08:

- `cargo fmt --check`: PASS.
- `cargo +stable-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`: PASS.
- `cargo +stable-x86_64-pc-windows-gnu test --locked`: PASS, 11 integration test functions including all 30 fixtures in separate semantic/runtime suites and runtime regressions.
- `node tools/validate-conformance.cjs`: PASS, 30 fixtures (13 accept, 15 static errors, 2 runtime errors).
- `hugo --source site --gc --minify --panicOnWarning`: PASS, Hugo Extended 0.167.0, 26 pages.
- Existing semantic JSON graph consistency tests: PASS; schema version remains 1.

The default MSVC toolchain could not link because `link.exe` is unavailable. The installed GNU toolchain completed the Rust checks. Incremental cache hard-link fallback warnings did not affect results.

A recursion regression initially exposed Windows stack exhaustion with a 128-call limit. The runtime guard was reduced to 32 and the regression now produces the intended diagnostic.

Deferred `sum`, `map`, `parallel` and `repeat` calls are gated before execution, including unreachable code and imported modules. This is bounded Execution Core coverage, not full library/parallel conformance. Original manifest status fields remain historical specification metadata; test results are recorded here separately.

A4 pagination is unverified. Remote CI status must be checked against the pushed commit; local results alone do not establish deployment or remote CI success.
