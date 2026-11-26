"""Serve the local owner-only CAD project CLI over MCP stdio."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import uuid

FRAME_BYTES = 512 * 1024
BUNDLE_BYTES = 256 * 1024
OUTPUT_BYTES = 8 * 1024 * 1024
VERSIONS = ("2024-11-05", "2025-03-26")
PART = {"type": "string", "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"}
REVISION = {"type": "string", "pattern": "^[0-9]{1,20}$"}
HANDLE = {"type": "string", "pattern": "^[0-9a-f]{32}$"}
KEY = {"type": "string", "pattern": "^[a-z][a-z0-9_.:/-]{0,127}$"}
JOB = {"type": "integer", "minimum": 1, "maximum": 2**63 - 1}


def tool(name, description, required, **optional):
    return {
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": required | optional,
            "required": list(required),
            "additionalProperties": False,
        },
    }


PREPARE = {"part": PART, "expected_project_revision": REVISION, "bundle": {"type": "object"}}
KEYS = {"type": "array", "items": KEY, "maxItems": 128}
TOOLS = {entry["name"]: entry for entry in [
    tool("capabilities", "Read the active CAD project profile.", {}),
    tool("inspect_project", "Read the current project revision and document index.", {}),
    tool("inspect_recipe", "Read accepted recipe outputs and bindings.", {"part": PART}),
    tool("prepare_recipe", "Preview a source bundle and stage it for explicit commit.", PREPARE, job_keys=KEYS),
    tool("prepare_commands", "Preview a command bundle against one admitted Part.", PREPARE, job_keys=KEYS),
    tool("preview", "Read the prepared candidate without applying it.", {"candidate": HANDLE}),
    tool("commit", "Apply one prepared candidate with a caller-supplied idempotency key.",
         {"candidate": HANDLE, "operation": HANDLE}),
    tool("receipt", "Recover the accepted result for an operation.", {"part": PART, "operation": HANDLE}),
    tool("job_status", "Read the job for a recipe's intent key.", {"part": PART, "key": KEY}),
    tool("cancel_job", "Cancel a job before publication.", {"job": JOB}),
    tool("artifact_info", "Read the manifest of a published definition.", {"job": JOB}),
]}


def matches(value, schema):
    match schema["type"]:
        case "string":
            return isinstance(value, str) and re.fullmatch(schema["pattern"], value) is not None
        case "object":
            return isinstance(value, dict)
        case "integer":
            return type(value) is int and schema["minimum"] <= value <= schema["maximum"]
        case "array":
            return (isinstance(value, list) and len(value) <= schema["maxItems"]
                    and all(matches(item, schema["items"]) for item in value))
    return False


class ToolError(Exception):
    def __init__(self, problem):
        self.problem = problem


class Bridge:
    def __init__(self, root, invoke):
        self.root = root
        self.invoke = invoke
        self.candidates = {}

    def call(self, name, args):
        if name not in TOOLS or not isinstance(args, dict):
            raise ValueError("Unknown CAD tool or invalid arguments")
        schema = TOOLS[name]["inputSchema"]
        if not set(schema["required"]) <= args.keys() or not all(
                key in schema["properties"] and matches(value, schema["properties"][key])
                for key, value in args.items()):
            raise ValueError("Arguments do not match the CAD tool schema")

        command = {
            "capabilities": ("capabilities", ()),
            "inspect_project": ("inspect", ()),
            "inspect_recipe": ("recipe-info", ("part",)),
            "receipt": ("receipt", ("part", "operation")),
            "job_status": ("job", ("part", "key")),
            "cancel_job": ("job-cancel", ("job",)),
            "artifact_info": ("job-definition", ("job",)),
        }.get(name)
        if command is not None:
            action, keys = command
            response = self.invoke(action, *(str(args[key]) for key in keys))
            return response["manifest"] if name == "artifact_info" else response
        if name in ("prepare_recipe", "prepare_commands"):
            if len(self.candidates) >= 32:
                raise ValueError("Prepared-candidate limit reached. Reconnect after recovering any receipts")
            payload = json.dumps(args["bundle"], ensure_ascii=False, allow_nan=False).encode()
            if len(payload) > BUNDLE_BYTES:
                raise ValueError("Source bundle exceeds input budget")
            candidate = uuid.uuid4().hex
            bundle = self.root / f"{candidate}.json"
            token = self.root / f"{candidate}.token"
            mode = "recipe" if name == "prepare_recipe" else "command"
            bundle.write_bytes(payload)
            try:
                result = self.invoke("preview", mode, args["part"], args["expected_project_revision"],
                                     bundle, token, *args.get("job_keys", ()))
            finally:
                bundle.unlink()
            self.candidates[candidate] = token, result
            return {"candidate": candidate, **result}
        if args["candidate"] not in self.candidates:
            raise ValueError("Unknown prepared candidate. Use receipt to recover a committed operation")
        token, result = self.candidates[args["candidate"]]
        if name == "preview":
            return result
        return self.invoke("apply", token, args["operation"])


def cli_invoker(executable, database, project):
    def invoke(*arguments):
        try:
            result = subprocess.run(
                [executable, database, project, *arguments], capture_output=True, timeout=90,
            )
            if len(result.stdout) > OUTPUT_BYTES:
                raise ValueError("CAD response exceeds transport budget")
            envelope = json.loads(result.stdout)
            if envelope.get("protocol") != "luma.cad.core-project/1" or envelope.get("schema_version") != 1:
                raise ValueError("Unexpected CAD protocol")
            if "error" in envelope:
                raise ToolError(envelope["error"])
            if result.returncode != 0:
                raise ValueError("CAD process did not return a receipt")
            return envelope["result"]
        except ToolError:
            raise
        except (OSError, ValueError, TypeError, KeyError, AttributeError, subprocess.TimeoutExpired):
            raise ToolError({"code": "ProjectTransport", "message": "CAD process failed. Recover an apply by operation ID"})
    return invoke


def respond(bridge, request):
    if not isinstance(request, dict) or request.get("jsonrpc") != "2.0":
        return {"jsonrpc": "2.0", "id": None, "error": {"code": -32600, "message": "Invalid request"}}
    if "id" not in request:
        return None
    response = {"jsonrpc": "2.0", "id": request["id"]}
    method = request.get("method")
    params = request.get("params", {})
    if not isinstance(params, dict):
        response["error"] = {"code": -32602, "message": "Invalid parameters"}
    elif method == "initialize":
        version = params.get("protocolVersion")
        response["result"] = {
            "protocolVersion": version if version in VERSIONS else VERSIONS[-1],
            "serverInfo": {"name": "luma-cad-project", "version": "1.0.0"},
            "capabilities": {"tools": {}},
        }
    elif method == "tools/list":
        response["result"] = {"tools": list(TOOLS.values())}
    elif method == "ping":
        response["result"] = {}
    elif method == "tools/call":
        try:
            value = bridge.call(params.get("name"), params.get("arguments", {}))
            problem = False
        except ToolError as error:
            value, problem = error.problem, True
        except (ValueError, TypeError, OverflowError):
            value, problem = {"code": "InvalidInput", "message": "Invalid CAD arguments or candidate"}, True
        response["result"] = {
            "content": [{"type": "text", "text": json.dumps(value, ensure_ascii=False)}],
            "isError": problem,
        }
    else:
        response["error"] = {"code": -32601, "message": "Unknown method"}
    return response


def unique_object(entries):
    result = {}
    for key, value in entries:
        if key in result:
            raise ValueError("Duplicate JSON key")
        result[key] = value
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("database", type=Path)
    parser.add_argument("project")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="luma-cad-mcp-") as temporary:
        bridge = Bridge(Path(temporary), cli_invoker(args.cli.resolve(), args.database.resolve(), args.project))
        while line := sys.stdin.buffer.readline(FRAME_BYTES + 1):
            if len(line) > FRAME_BYTES:
                return 2
            try:
                result = respond(bridge, json.loads(line, object_pairs_hook=unique_object))
            except (ValueError, RecursionError):
                result = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "Invalid JSON"}}
            if result is not None:
                print(json.dumps(result), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
