"""Exercise the local project CLI, including process failure inside Core's SQLite commit."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import secrets
import sqlite3
import subprocess
import tempfile

import lcan

ROOT = Path(__file__).resolve().parents[2]
PROJECT = "00000000-0000-4000-8000-000000000001"
PART = "00000000-0000-4000-8000-000000000002"


def operation(index):
    return f"{index:032x}"


def bundle(root, name, original, rewrite):
    frame = json.loads((ROOT / "examples/cad" / original).read_text(encoding="utf-8"))
    entry = frame["entry"]
    frame["files"][entry] = rewrite(frame["files"][entry])
    typed = {"bundle_version": {"$u64": "1"}, "entry": entry, "files": frame["files"]}
    frame["source_digest"] = lcan.digest("luma.cad.source", lcan.encode(typed))
    path = root / f"{name}.json"
    path.write_text(json.dumps(frame), encoding="utf-8")
    return path


def jobs_and_closure(call, root):
    preview_job = bundle(
        root, "queue", "plate-geometry.json",
        lambda s: 'use jobs\n' + s + '\njob = jobs.submit("definition", jobs.task("part_definition"))\n',
    )
    token = root / "queue.token"
    call("preview", "recipe", PART, 4, preview_job, token)
    call("apply", token, operation(7), crash="command-after-luma-outbox", code=73)
    assert call("receipt", PART, operation(7), code=3)["code"] == "UnknownOperation"
    assert call("work") is None
    call("apply", token, operation(7), crash="command-after-commit", code=73)
    accepted = call("receipt", PART, operation(7))
    delivery = accepted["jobs"][0]
    assert delivery["key"] == "definition"
    assert call("apply", token, operation(7)) == accepted
    assert call("job", PART, "definition")["state"] == "admitted"
    assert call("job-cancel", delivery["job"])["state"] == "cancelled"
    assert call("job-retry", delivery["job"], 0)["state"] == "admitted"
    finished = call("work")
    assert finished["job"] == delivery["job"]
    assert finished["state"] == "published"
    result = call("job-definition", delivery["job"])
    digest = hashlib.sha256(result["canonical_definition"].encode()).hexdigest()
    assert result["manifest"]["manifest_digest"] == f"sha256:{digest}"
    repeat_token = root / "repeat-queue.token"
    assert not call("preview", "recipe", PART, 5, preview_job, repeat_token)["candidate_changed"]
    assert call("apply", repeat_token, operation(8))["jobs"] == accepted["jobs"]
    pinned_revision = call("recipe-info", PART)["recipe_revision"]

    follow = "00000000-0000-4000-8000-000000000003"
    pinned = "00000000-0000-4000-8000-000000000004"
    bounded = bundle(
        root, "bounded", "plate-geometry.json",
        lambda s: s.replace('param.length("width", units.mm(100))',
                            'param.length("width", units.mm(120))')
                   .replace("height = ", "param.bounds(width, units.mm(100), units.mm(130))\nheight = "),
    )
    revision = 5
    for index, part in enumerate((follow, pinned)):
        op = 10 + 3 * index
        assert call("create-part", part, revision, operation(op))["project_revision"] == str(revision + 1)
        revision += 1
        source_token = root / f"source-{index}.token"
        call("preview", "recipe", part, revision, bounded, source_token)
        call("apply", source_token, operation(op + 1))
        revision += 1
        manifest = root / f"bindings-{index}.json"
        binding = {"target": "width", "producer": PART, "output": "width"}
        if part == pinned:
            binding["revision"] = pinned_revision
        manifest.write_text(json.dumps([binding]), encoding="utf-8")
        binding_token = root / f"bindings-{index}.token"
        call("bind", part, revision, manifest, binding_token)
        call("apply", binding_token, operation(op + 2))
        revision += 1

    def width(part):
        return call("recipe-info", part)["outputs"]["width"]["Length"]

    assert width(PART) == width(follow) == width(pinned) == 0.12
    command = bundle(
        root, "wider", "plate-width-command.json",
        lambda s: s.replace("units.mm(120)", "units.mm(125)"),
    )
    closure = root / "closure.token"
    call("preview", "command", PART, revision, command, closure)
    call("apply", closure, operation(16), crash="command-before-commit", code=73)
    assert width(PART) == width(follow) == width(pinned) == 0.12
    accepted = call("apply", closure, operation(16))
    revision += 1
    assert accepted["project_revision"] == str(revision)
    assert width(PART) == width(follow) == 0.125
    assert width(pinned) == 0.12

    failed = bundle(
        root, "out-of-bounds", "plate-width-command.json",
        lambda s: s.replace("units.mm(120)", "units.mm(140)"),
    )
    failure = root / "failed-closure.token"
    assert call("preview", "command", PART, revision, failed, failure, code=3)["code"] == "ParameterBounds"
    assert not failure.exists()
    assert width(PART) == width(follow) == 0.125
    assert width(pinned) == 0.12

    detached = root / "detached.token"
    assert call("detach", PART, revision, detached, code=3)["code"] == "ReferencedEntityRemoval"
    unbind = root / "unbind.json"
    unbind.write_text("[]", encoding="utf-8")
    unbind_token = root / "unbind.token"
    call("bind", follow, revision, unbind, unbind_token)
    call("apply", unbind_token, operation(17))
    revision += 1
    report = call("detach", PART, revision, detached)["detach"]
    assert report["released_entities"]
    receipt = call("apply", detached, operation(18))
    assert receipt["project_revision"] == str(revision + 1)
    assert receipt["document_revision"] == accepted["document_revision"]
    assert call("recipe-info", PART)["detached"]
    assert width(PART) == 0.125 and width(pinned) == 0.12


def rejected_inputs(call, root, database):
    before = call("inspect")["project_revision"]
    operation_id = operation(99)
    raw = root / "over-limit.json"
    with raw.open("wb") as stream:
        stream.truncate(256 * 1024 + 1)
    assert call("preview", "recipe", PART, before, raw, root / "too-large.token", code=6)["code"] == "ResourceLimit"
    with raw.open("wb") as stream:
        stream.truncate(8 * 1024 * 1024 + 1)
    assert call("apply", raw, operation_id, code=6)["code"] == "ResourceLimit"
    assert call("receipt", PART, operation_id, code=3)["code"] == "UnknownOperation"

    # Corrupt a result in the disposable fixture after its authority commit.
    # Recovery recomputes the digest from the authenticated native definition.
    with sqlite3.connect(database) as connection:
        job, attempt, artifact = connection.execute(
            "SELECT job, number, artifact FROM lcad_attempts WHERE state = 'published' ORDER BY job LIMIT 1"
        ).fetchone()
        invalid = json.loads(artifact)
        invalid["manifest_digest"] = "sha256:" + "0" * 64
        connection.execute(
            "UPDATE lcad_attempts SET artifact = ? WHERE job = ? AND number = ?",
            (json.dumps(invalid), job, attempt),
        )
    assert call("job-definition", job, code=5)["code"] == "CheckpointCorrupt"
    assert call("inspect")["project_revision"] == before

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
        edited_path = bundle(
            root, "edited", "plate-geometry.json",
            lambda s: s + "\n# source-only provenance change\n",
        )
        next_token = root / "edited.token"
        source_only = call("preview", "recipe", PART, 3, edited_path, next_token)
        assert source_only["candidate_changed"]
        reaccepted = call("apply", next_token, operation(5))
        assert reaccepted["project_revision"] == "4"
        assert reaccepted["document_revision"] == accepted["document_revision"]
        jobs_and_closure(call, root)
        rejected_inputs(call, root, db)
    print("PASS Core CAD-1 CLI: model, job and receipt rollback, retry, closure and detach")


if __name__ == "__main__":
    main()
