# Validated Structural Edits v1

Use Rust's `luma::edits::Workspace` or launch `luma edit` and exchange one UTF-8 JSON object per line on stdin/stdout. Each request and response has `schema_version: 1`. The request schema is in [schema/edit-request-v1.json](../schema/edit-request-v1.json).

```json
{"schema_version":1,"request":{"action":"open","entry":"main.luma","files":{"main.luma":"# example\n"}}}
{"schema_version":1,"request":{"action":"snapshot"}}
{"schema_version":1,"request":{"action":"preview","proposal":{"schema_version":1,"expected_revision":"COPY_FROM_SNAPSHOT","operations":[{"kind":"create_value","file":"main.luma","name":"count","value":{"kind":"number","value":2}}]}}}
```

Submit the same proposal with `action: "commit"` to publish it. Each reply contains exactly one of `result` and `error`. An open, snapshot or commit result has `revision`, `ir` (Structural IR v1) and `graph` (Graph Explorer v1). Preview returns `base_revision`, `candidate` (the same snapshot shape), and `edits` (file, start, end and replacement). Edits are half-open UTF-8 byte ranges in the base source. Apply them in reverse array order. Errors have `code`, `message`, a nullable zero-based `operation`, a nullable `actual_revision`, and checker `diagnostics` with candidate spans on validation failure.

| Operation | Semantics |
|---|---|
| `create_function` | Append an empty private module function. |
| `add_parameter` | Append one fresh parameter to a function by syntax ID. The checker must infer its type and validate all uses in the supplied import graph. |
| `create_value` | Append a private module binding with a fresh name and a scalar or name operand. |
| `insert_call` | Append a call statement at module scope. The callee is one name or a two-component member name. |
| `connect` | Replace an existing simple argument at its zero-based index with a scalar or name operand. The checker resolves the name in the call's scope and verifies its type. |
| `remove_node` | Remove a private top-level function or scalar literal binding. Any same-file identifier mention outside the statement blocks removal, including possible writes or shadowed mentions. |

All IDs target base-revision syntax. Use a later proposal to target a newly created node by ID. Proposed edits must not overlap. Appends preserve proposal order and use CRLF if present in the file. Text outside edited spans is preserved byte for byte. Removing a statement can leave blank lines and comments. Rename, public deletion, cascading edits and automatic call-site migrations are unsupported. Only the supplied entry and its transitive imports are checked, so separate downstream projects are outside the validation scope.

The revision is a deterministic SHA-256 digest of the version, generation, entry and sorted source map. Every accepted commit advances generation, including edits that restore old content. A token is a precondition within one workspace session. A new session starts at generation zero. S1 named IDs typically survive unrelated edits. Renamed declarations, duplicate statement ordinals, and positional children can change or reuse IDs. Re-resolve selection and spans against each revision. Legacy graph IDs include byte offsets and can shift after text edits.

The workspace accepts at most 16 lowercase sibling .luma names, 32 KiB of total source, 2048 tokens, 32 operations per proposal, 16 arguments per inserted call and 64 ASCII bytes per identifier. The CLI caps a request frame at 128 KiB. The edit API never resolves disk paths, opens imports from disk, or executes source. In Rust, use a shared mutex to serialize access from multiple writers. Commit publishes source, IR and graph together inside the session. Persistence, filesystem writer coordination and crash recovery across files remain outside v1.
