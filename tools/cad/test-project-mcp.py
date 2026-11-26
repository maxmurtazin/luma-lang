"""Check stdio CAD MCP requests against an isolated transport fixture."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    bridge = Path(__file__).with_name("project-mcp.py")
    part = "00000000-0000-4000-8000-000000000002"
    operation = "7" * 32
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        # Use the Python interpreter as the process fixture so the test does
        # not require a private Core build or platform-specific launcher.
        fixture = root / "fixture.py"
        fixture.write_text(
            """
import json
from pathlib import Path
import sys

_, project, command, *args = sys.argv
out = {"protocol": "luma.cad.core-project/1", "schema_version": 1}
match command:
    case "preview":
        mode, part, revision, bundle, token, *keys = args
        assert mode == "command" and keys == ["definition"]
        frame = json.loads(Path(bundle).read_bytes())
        assert frame == {"bundle_version": 1}
        Path(token).write_bytes(b"authenticated candidate")
        out["result"] = {"project_revision": revision, "candidate_changed": True}
    case "apply":
        assert Path(args[0]).read_bytes() == b"authenticated candidate"
        out["result"] = {"project_revision": "2", "operation": args[1]}
    case "receipt":
        out["result"] = {"project_revision": "2", "operation": args[1]}
    case _:
        raise AssertionError(command)
print(json.dumps(out))
""", encoding="utf-8",
        )
        with subprocess.Popen(
            [sys.executable, "-B", bridge, sys.executable, fixture, part],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding="utf-8",
        ) as server:
            def rpc(method, params):
                message = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
                server.stdin.write(json.dumps(message) + "\n")
                server.stdin.flush()
                return json.loads(server.stdout.readline())["result"]

            def use(name, **arguments):
                response = rpc("tools/call", {"name": name, "arguments": arguments})
                return (response["isError"], json.loads(response["content"][0]["text"]))

            assert rpc("initialize", {"protocolVersion": "2024-11-05"})["protocolVersion"] == "2024-11-05"
            assert "prepare_commands" in {tool["name"] for tool in rpc("tools/list", {})["tools"]}
            assert use("commit", candidate="../../token", operation=operation)[0]
            failed, staged = use("prepare_commands", part=part, expected_project_revision="1",
                                 bundle={"bundle_version": 1}, job_keys=["definition"])
            assert not failed and staged["candidate_changed"]
            assert use("preview", candidate=staged["candidate"]) == (
                False, {"project_revision": "1", "candidate_changed": True})
            first = use("commit", candidate=staged["candidate"], operation=operation)
            assert first == (False, {"project_revision": "2", "operation": operation})
            assert use("receipt", part=part, operation=operation) == first
            server.stdin.close()
            assert server.wait(timeout=10) == 0
    print("PASS CAD MCP stdio prepare, commit and receipt transport")


if __name__ == "__main__":
    main()
