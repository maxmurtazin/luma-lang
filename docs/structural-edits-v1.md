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

The workspace accepts at most 16 lowercase sibling .luma names, 32 KiB of total source, 2048 tokens, 32 operations per proposal, 16 arguments per inserted call and 64 ASCII bytes per identifier. The CLI caps a request frame at 128 KiB. The default edit session uses supplied sources only. The checker never executes source. In Rust, use a shared mutex for a shared in-memory workspace.

## Disk transactions

Launch `luma edit --workspace <trusted-directory>` or use `luma::edits::DiskWorkspace`. A JSON client opens the authorized directory with:

```json
{"schema_version":1,"request":{"action":"open_disk","entry":"main.luma","modules":["main.luma","lib.luma"]}}
```

List the entry and all sibling imports. Module names are portable lowercase identifiers. The JSON request cannot choose the directory. The directory and its journal must be controlled by a trusted user. Direct symlinks and Windows reparse point files are rejected. Preview uses the in-memory base snapshot. Commit checks the proposal revision and compares every supplied module byte for byte under an OS file lock. Reopen after an external change. The lock file must remain in place and all concurrent filesystem writers and consistency-sensitive readers must coordinate with it. A content change followed by an external restoration of identical bytes is indistinguishable from no external edit.

Each changed file is staged in the same directory and synced before publication. A synced undo journal records old and new contents before the first replacement. Commit replaces the files, syncs the directory on Unix, removes the journal and syncs the directory again. Journal removal is the commit point. If the final sync fails, the API reports `commit_uncertain`; reopen and inspect the files before retrying. An ordinary failure before the commit point attempts rollback under the lock. If rollback cannot complete, the journal remains available and the API reports `recovery_required`. Opening a workspace automatically retries rollback. Recovery refuses to overwrite contents that match neither the old nor new bytes. Process termination can leave orphan staging files that may be removed manually while no edit sessions are active.

Individual replacements use same-directory rename. Readers that ignore the lock, including the existing check/run/ir commands, may observe intermediate contents. Multi-file isolation applies to cooperating sessions. Recovery is based on the journal and is distinct from simultaneous visibility to arbitrary readers. Unix directory sync depends on the filesystem honoring flushes and atomic replacement. Windows syncs file contents, and the current implementation has no portable directory flush, so sudden power-loss durability of directory entries is unverified. Network filesystems, hostile concurrent filesystem changes, and preservation of hard links, extended attributes and custom ACLs are outside the contract.
