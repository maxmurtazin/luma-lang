"""Validate the public S1 wire contract in CI (jsonschema 4.23.0)."""
import json
import sys
from pathlib import Path
from jsonschema import Draft202012Validator

schema = json.loads(Path("schema/structural-v1.json").read_text(encoding="utf-8"))
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
for name in sys.argv[1:]:
    document = json.loads(Path(name).read_text(encoding="utf-8-sig"))
    validator.validate(document)
    assert document["valid"] and not document["diagnostics"]
    ids = {n["id"] for n in document["nodes"]}
    assert len(ids) == len(document["nodes"])
    assert all(e["from"] in ids and e["to"] in ids for e in document["edges"])
    print(f"PASS S1 JSON Schema and edge closure: {name}")
