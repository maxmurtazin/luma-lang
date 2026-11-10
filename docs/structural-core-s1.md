# Structural Core S1 contract

Read-only typed semantic representation, schemaVersion 1. [S0 audit and GO decision](structural-core-s0.md). Rust API: luma::structural::analyze(path) -> Document; Document::to_json(). CLI: luma ir file.luma (JSON stdout; exit 0 valid, 1 invalid, 2 usage). No execution. Existing check/run/--graph and Graph Explorer v1 remain supported. No installer/VSIX version bump.

## Consumer contract

[schema/structural-v1.json](../schema/structural-v1.json) defines the JSON Schema. src/structural.rs contains serializable/deserializable Rust DTOs. Check schemaVersion == 1 and valid == true before consuming semantic data. Invalid documents include diagnostics and successfully read source snapshots, but syntax/nodes/edges are empty. Missing files have no content entry; diagnostic locations can name unavailable files. Files read before a parse failure remain recorded. OS error messages may differ by platform.

files separates file id, normalized path and exact UTF-8 content. Content equality is the collision-free content identity in v1, not a hash. Consumers can validate spans against the checked snapshot without rereading disk. This increases payload size and includes source text. No timestamps, random IDs or host roots occur for ordinary sibling imports. Paths are relative to the canonical entry directory; symlink targets outside it use explicit canonical paths, which are machine-specific. Rename changes file identity. Path matching follows the host filesystem.

syntax contains the complete AST with parent links, kind, optional literal/operator/name text and half-open UTF-8 byte locations. Array/argument/arm order is encoded in ID indices, not JSON sort order. This is not a lossless formatter, CFG, SSA or execution plan.

nodes contains checker observations: definition, expression, reference, pattern, module, builtin. Each has an optional syntax link, location, structured ty, declaration public flag and owner effect. Modules/builtins have no syntax/span. Namespace types target a module/object definition. Functions have monomorphic parameters and result. Null ty means no value type recorded for that role, not dynamic typing. Statements, wildcard patterns and member labels need not have expression observations. Direct callees have function types. Pipeline calls record the result and the implicit first argument in the callee signature. Comparison patterns carry the compared value type. Valid exports must not contain unresolved types. Where parser pattern wrappers share a span, observations link to the first outer matching syntax node (name-preferred for references/definitions); children remain in syntax.

edges: contains, reads, resolves_to, calls, callback, imports, captures. Calls/callbacks are owner-level dependencies, not per-invocation/control-flow edges. Reference-to-definition/builtin edges identify targets. Captures include transitive propagation. All semantic endpoints exist in nodes. Syntax parents refer to syntax IDs or file IDs. Public visibility is not a capability grant.

## IDs and collision/edit policy

Module anchors are file:<normalized path>. Body anchors use statement kind and hex-encoded name for functions, objects, simple assignments and imports; repeated anchors get deterministic ordinals. Anonymous statements use kind and same-kind ordinal. Children use structural field names or positional indices. Semantic IDs append @role; builtins use builtin:<name>. No hashing or truncated fingerprints. Distinct syntax nodes have distinct paths. Checker observations with exactly the same file/span/role are intentionally one observation.

Whitespace/comments and unrelated named sibling edits preserve anchors; tests cover insertion and relocation. Rename/move, adding/removing pub, changing tree shape, inserting/reordering parameters/arguments/arms or same-anchor statements may change IDs. Repeated assignments get separate ordinals; duplicate declarations remain errors. IDs are structural identities, not immutable entities across arbitrary edits. S2 needs a persistent identity registry and candidate-source validation. Future edits must check expected file content/schema and re-run checking before acceptance. S1 provides no transactions, incremental invalidation or safe-by-construction editing guarantee.

## Effects and prospective adapter

Only module/function owners are classified. conservative_effectful means flagged I/O, captures or unproven callback/indirect effects, including propagated dependencies. not_flagged means absence from that set only. Neither proves purity, termination, runtime success, CAD capability safety or absence of local mutation. Generic builtin nodes have no single polymorphic type; contracts remain direct-call-only. All type rules, resolution and effect propagation stay in semantic.rs; the new AST walk only assigns structural/source identities.

A future Living IR/CAD adapter can consume valid Document, resolve function/module targets, inspect types/dependencies and validate content before creating its own domain requests. It must independently validate units, geometry contracts, capabilities and transactions. No CAD operations/types, host bridge, keywords, Studio UI or living-ir-core changes are implemented. Runtime still executes the checked AST and enforces bounds/division/resource limits. Legacy graph keeps historical basename/byte-offset IDs.

## Verification

Run cargo fmt --check; cargo clippy --locked --all-targets -- -D warnings; cargo test --locked; node tools/validate-conformance.cjs; node tools/test-examples.cjs; node tools/test-doc-examples.cjs. New structural tests cover conformance agreement, pipelines, match, objects, callbacks, module targets, spans, collisions, invalid input, JSON round trip, deterministic output and irrelevant edits. Fixture: tests/fixtures/structural/main.luma. CI validates the export against JSON Schema and uploads it. Existing Hugo/docs and extension workflows remain acceptance gates.
