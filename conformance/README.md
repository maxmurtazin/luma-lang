# Semantic fixtures, not implemented Luma tests

30 conceptual programs and machine-readable expected outcomes. Run node tools/validate-conformance.cjs for integrity validation only. No Luma code is executed. The Hugo conformance page contains the coverage matrix. Each fixture is an isolated program rooted at main.luma; all listed .luma files are inputs. Positive observations refer to final module bindings. Diagnostic IDs classify errors, not exact wording. All semantic statuses remain specified-not-executed.

Mutation of a local by-value array is covered by array-copy; transitive I/O is rejected by parallel-transitive-io. External shared writes are excluded by the assignment grammar and lexical local-assignment rule. Further effect-analysis implementation tests must cover indirect/unknown effects and changing captures once a checker exists.
