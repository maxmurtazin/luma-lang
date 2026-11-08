# Luma

An early design for a minimal procedural language built around functions, data flow, and explicit parallelism. The bounded execution core is an interpreter written in Rust.

**Luma 0.1D1 provides `luma run file.luma`, a bounded interpreter using the existing checker and validated AST.** See [execution core](site/content/docs/runtime.md) for supported behavior, gates and exit statuses.

## Documentation

The Hugo + Hextra site lives in `site/`. It covers getting started, a one-page syntax draft, language design, modules, arrays, functions, structural objects, match and pipelines, parallelism, standard-library planning, and the CLI/VS Code graph roadmap.

Intended Pages address: https://maxmurtazin.github.io/luma-lang/ (available only after deployment).

### Local setup

Install **Hugo Extended 0.167.0**, **Go 1.25.7**, and Git. These are documentation tools, not Luma dependencies. Hextra is pinned to v0.13.0 through `site/go.mod` and `site/go.sum`; the first build downloads it and requires internet access. No theme submodule or Node setup is required.

```sh
git clone https://github.com/maxmurtazin/luma-lang.git
cd luma-lang/site
hugo server --disableFastRender
```

Open the local URL printed by Hugo. Stop the server with Ctrl+C.

### Production build

From `site/`:

```sh
hugo --gc --minify --panicOnWarning
```

The generated site is in `site/public/` and is ignored by Git. The configured production base URL includes `/luma-lang/`. For another host, pass `--baseURL https://example.com/your-path/`.

### GitHub Pages

1. Push this repository to the `main` branch on GitHub.
2. In **Settings → Pages → Build and deployment**, choose **GitHub Actions** as the source.
3. Open **Actions → Documentation → Run workflow**, or push another change to `main`.
4. Inspect the successful deployment job for the actual site URL.

The workflow builds pull requests without deploying them. Pushes to `main` and manual runs on `main` build and deploy through the `github-pages` environment. Repository Pages availability and any environment approval rules must permit deployment. No personal access token is required by the workflow.

The workflow pins the same Hugo and Go versions used for local verification. Hextra updates should be intentional: run `hugo mod get github.com/imfing/hextra@VERSION` from `site/`, commit both module files, and rebuild.

## Design boundary

Chosen direction: interpreter-first Rust, static type inference, simple number/string concepts, no classes, overloading, `let`, or `var`; file-based modules with `pub`/`use`; structural objects; insignificant indentation and mandatory `end`; one `->` arrow; `match`; function-based repetition; explicit parallel work without async; future VS Code graph visualization.

Draft 001 fixes the core syntax and semantics; additional bounded rules are labeled proposals. Numeric edge cases, resource limits and wider library APIs remain open. See `site/content/docs/specification.md`. Do not describe a proposal as implemented until code and verification support it.

## Contributing

Keep pages in English. Label examples as proposed and use `text` fences until there is a stable Luma grammar. Record open questions instead of silently choosing semantics. Use Hugo `relref` links for internal pages so the build catches missing targets.

## References

- [Hextra setup](https://imfing.github.io/hextra/docs/getting-started/)
- [Hugo GitHub Pages deployment](https://gohugo.io/host-and-deploy/deploy-to-github-pages/)

Hextra is an external theme with its own license. No license for the Luma project is selected by this documentation change.

## Stage 0.1A

Draft 001: `site/content/docs/specification.md`. Compact syntax and conformance matrix live alongside it. Thirty fixtures in `conformance/` specify positive and negative outcomes. They are not implemented interpreter tests.

Run `node tools/validate-conformance.cjs` (Node.js 22+) to validate fixture metadata and coverage. This does not execute Luma. Actual validation results: `spec/validation-results.md`. Next stage: **0.1B — Rust Lexer, Parser & AST**.

## Rust frontend (0.1B)

Build with cargo build. Run cargo run -- check file.luma for bounded semantic validation; add --syntax-only for the original parser behavior. Export a graph with cargo run -- check --graph graph.json file.luma. See [0.1C documentation](site/content/docs/semantics.md) for decisions, graph schema and limitations. The execution core is available through `luma run`; see its documented limits.
