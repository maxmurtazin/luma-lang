"""Validate request schema and exercise the real disk-edit CLI."""
import json
import subprocess
import tempfile
from pathlib import Path

from jsonschema import Draft202012Validator

schema = json.loads(Path("schema/edit-request-v1.json").read_text(encoding="utf-8-sig"))
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
ir_validator = Draft202012Validator(
    json.loads(Path("schema/structural-v1.json").read_text(encoding="utf-8"))
)

with tempfile.TemporaryDirectory(prefix="luma edit ") as directory:
    source = Path(directory) / "main.luma"
    source.write_bytes(b"# original\r\n")
    with subprocess.Popen(
        ["cargo", "run", "--quiet", "--locked", "--", "edit", "--workspace", directory],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    ) as cli:
        def request(action, **fields):
            message = {"schema_version": 1, "request": {"action": action, **fields}}
            validator.validate(message)
            cli.stdin.write(json.dumps(message) + "\n")
            cli.stdin.flush()
            return json.loads(cli.stdout.readline())

        opened = request("open_disk", entry=source.name, modules=[source.name])["result"]
        ir_validator.validate(opened["ir"])
        proposal = {
            "schema_version": 1,
            "expected_revision": opened["revision"],
            "operations": [
                {"kind": "create_value", "file": source.name, "name": "x",
                 "value": {"kind": "number", "value": 1}}
            ],
        }
        preview = request("preview", proposal=proposal)["result"]
        assert source.read_bytes() == b"# original\r\n"
        committed = request("commit", proposal=proposal)["result"]
        ir_validator.validate(committed["ir"])
        assert committed == preview["candidate"]
        assert source.read_bytes() == b"# original\r\nx = 1\r\n"
        assert request("commit", proposal=proposal)["error"]["code"] == "revision_conflict"
        cli.stdin.close()
        assert cli.wait() == 0
