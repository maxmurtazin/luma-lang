"""Exercise the local project CLI, including process failure inside Core's SQLite commit."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile

import lcan

ROOT = Path(__file__).resolve().parents[2]
PROJECT = "00000000-0000-4000-8000-000000000001"
PART = "00000000-0000-4000-8000-000000000002"


def operation(index):
    return f"{index:032x}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path, help="binary built with checkpoint-fault-injection")
    parser.add_argument("--scene", type=Path, help="save the accepted View V2 scene from the composed build")
    args = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="luma-cad1-workflow-") as directory:
        root = Path(directory)
        db = root / "project.db"
        recipe = root / "recipe.token"
        command = root / "command.token"
        env = dict(os.environ, LUMA_CAD_PROJECT_KEY=secrets.token_hex(32))
        env.pop("LUMA_CAD_TEST_CRASH_AT", None)

        def call(*arguments, crash=None, code=0):
            process_env = dict(env, LUMA_CAD_TEST_CRASH_AT=crash) if crash else env
            result = subprocess.run(
                [args.cli.resolve(), db, PROJECT, *map(str, arguments)], env=process_env,
                capture_output=True, text=True, timeout=90,
            )
            assert result.returncode == code, (arguments, result.returncode, result.stdout, result.stderr)
            if code == 73:
                return
            envelope = json.loads(result.stdout)
            assert envelope["protocol"] == "luma.cad.core-project/1"
            assert ("result" in envelope) is (code == 0)
            return envelope["result" if code == 0 else "error"]

        assert call("init", PART, operation(1))["project_revision"] == "1"
        call("preview", "recipe", PART, 1, ROOT / "examples/cad/plate-geometry.json", recipe)
        for stage in ("command-after-checkpoint", "command-after-receipt", "command-before-commit"):
            call("apply", recipe, operation(2), crash=stage, code=73)
            assert call("receipt", PART, operation(2), code=3)["code"] == "UnknownOperation"
        # The operation committed, but the client never received stdout.
        call("apply", recipe, operation(2), crash="command-after-commit", code=73)
        initial = call("receipt", PART, operation(2))
        assert initial["project_revision"] == "2"
        assert call("apply", recipe, operation(2)) == initial

        saved = call("preview", "command", PART, 2, ROOT / "examples/cad/plate-width-command.json", command)
        assert saved["candidate_changed"]
        with ThreadPoolExecutor(max_workers=2) as pool:
            applications = [pool.submit(call, "apply", command, operation(3)) for _ in range(2)]
            accepted, repeated = [application.result() for application in applications]
        assert accepted == repeated == call("receipt", PART, operation(3))
        assert accepted["project_revision"] == "3"
        assert call("apply", recipe, operation(4), code=4)["code"] == "Conflict"
        if args.scene:
            scene = call("scene", PART, operation(3), root)
            assert scene["scene"]["stamp"]["revision"] == "3"
            assert scene["resources"]
            with args.scene.open("x", encoding="utf-8") as output:
                json.dump(scene, output)
        # Source-only provenance gets a project revision and reuses the exact native revision.
        edited = json.loads((ROOT / "examples/cad/plate-geometry.json").read_text(encoding="utf-8"))
        edited["files"][edited["entry"]] += "\n# source-only provenance change\n"
        typed = {"bundle_version": {"$u64": "1"}, "entry": edited["entry"], "files": edited["files"]}
        edited["source_digest"] = lcan.digest("luma.cad.source", lcan.encode(typed))
        edited_path = root / "edited.json"
        edited_path.write_text(json.dumps(edited), encoding="utf-8")
        next_token = root / "edited.token"
        source_only = call("preview", "recipe", PART, 3, edited_path, next_token)
        assert source_only["candidate_changed"]
        reaccepted = call("apply", next_token, operation(5))
        assert reaccepted["project_revision"] == "4"
        assert reaccepted["document_revision"] == accepted["document_revision"]
    print("PASS Core CAD-1 CLI: recipe, command, competing retry, crashes, recovery")


if __name__ == "__main__":
    main()
