# Conformance fixtures

32 isolated projects with static/runtime expected outcomes in manifest.json. Historical metadata retains specified-not-executed as its original inventory status; cargo test executes syntax, semantic and runtime assertions today. tools/validate-conformance.cjs checks the historical matrix in spec/history/conformance.md without executing Luma.

The examples directory provides observable versions of all fixtures plus operator/library demonstrations; node tools/test-examples.cjs validates these with the real CLI after cargo build.
