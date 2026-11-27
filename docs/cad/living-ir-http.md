# Living IR HTTP adapter

Use the existing SketchHost recipe syntax to propose a fixed XY rectangle and
one positive normal extrusion in an existing Living IR single-document
research project. The local Luma command evaluates the SourceBundle through
the Rust checker and interpreter, then emits one JSON proposal.

```sh
cargo test --locked
cargo run --locked -- cad living-ir examples/cad/living-ir-rectangle.json --mode recipe > recipe.json
```

The bundle uses `sketch.create`, four `sketch.point` and `sketch.fix` pairs,
four `sketch.line` edges, `sketch.profile`, `brep.extrude` and `cad.output`.
The admission and exact source digest use the existing SourceBundle v1 and
LCAN/1 routines. The adapter uses the evaluated SI values from the SketchHost.
Luma quantities currently use binary64.

For an existing HTTP project, the Core operator's `command-context` action
combines a SourceBundle and the pinned project state. Use
`luma cad living-ir-command command-input.json --mode command` with an
ordinary `project.part`, `project.parameter` and `param.set` source such as
[the width example](../../examples/cad/plate-width-command.json). The Part
argument must be the configured HTTP document UUID. The command adapter
admits one changed parameter with an integral millimeter value. It emits
a literal string and the original revision, epoch, incarnation and
definition pin. For an exact fractional mm edit, use the operator's JSON
intent with the authored decimal string.

Compile the proposal with the matching Core integration and use its operator
CLI to prepare, inspect, and confirm each of the three commits. The Core CLI
requires the preceding prepared envelope and verifies its native receipt
before preparing each dependent step. To resize before extrusion, keep the
source sketch topology and UUIDs, compile a revised bundle, and pass its
proposal with the preceding rectangle envelope to Core's `resize` step.
Keep the envelopes if the run stops
partway through. A failed or unknown step leaves earlier accepted steps in
the project. For an ambiguous outcome, look up the original receipt before
continuing.

Source sketch, profile, point and segment UUIDs are preserved in the JSON
proposal. The HTTP sketch API allocates its own sketch and element UUIDs.
The extrude command uses the exact Luma extrusion UUID and the native sketch
digest. This adapter does not write Luma ownership or LCAN metadata into
Canonical HIR. The operator envelope retains the Luma source digest for local
provenance. Existing direct SQLite integrations use their own contracts.

[Core SDK and operator instructions](https://github.com/maxmurtazin/living-ir-core/blob/main/docs/api/API-05-LUMA-MCP.md)
