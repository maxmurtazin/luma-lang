#!/usr/bin/env python3
"""Exercise the embedded solver and the Core registry's atomic commit and replay."""
import argparse
import hmac
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile
import uuid

import lcan


def entity(number):
    return str(uuid.UUID(int=(0x98420000000040008000000000000000 + number)))


def operation(number):
    return entity(number).replace("-", "")


def write(path, value):
    path.write_text(json.dumps(value), encoding="utf-8")
    return path


def source(path, complete, height=4):
    program = ["use units", "use sketch", "use draft", "use brep",
               f's = sketch.create("{entity(10)}", "{entity(11)}", "{entity(12)}")']
    coordinates = [(0, 0), (40, 0), (40, 30), (0, 30), (20, 15), (24, 15)]
    for index, (x, y) in enumerate(coordinates):
        xyz = f"units.mm({x}), units.mm({y})"
        program.extend([
            f'p{index} = sketch.point(s, "{entity(20 + index)}", {xyz})',
            f'sketch.fix(s, "{entity(30 + index)}", p{index}, {xyz})',
        ])
    for index in range(4 if complete else 3):
        program.append(f'sketch.line(s, "{entity(40 + index)}", p{index}, p{(index + 1) % 4})')
    program.extend([
        f'sketch.circle(s, "{entity(44)}", p4, p5)', "profile = sketch.profile(s)",
        f'd = draft.create("{entity(60)}")',
        f'layer = draft.layer(d, "{entity(70)}")',
        f'a = draft.point(layer, "{entity(71)}", units.mm(0), units.mm(0))',
        f'b = draft.point(layer, "{entity(72)}", units.mm(10), units.mm(0))',
        f'edge = draft.add(layer, "{entity(73)}", a, b)',
        'draft.transform(edge, units.mm(5), units.mm(2))',
        f'body = brep.extrude("{entity(80)}", profile, units.mm({height}))',
    ])
    files = {"draw-part.luma": "\n".join(program) + "\n"}
    digest = lcan.digest("luma.cad.source", lcan.encode({
        "bundle_version": {"$u64": "1"}, "entry": "draw-part.luma", "files": files,
    }))
    return write(path, {"bundle_version": 1, "entry": "draw-part.luma", "files": files, "source_digest": digest})


def qualify_branch(call, fixtures, out, project, part, final, owner_key):
    setup = json.loads((fixtures / "sketch/create-length-segment.json").read_bytes())
    setup.update(project_id=project, document_id=part, expected_revision=final["embedded_revision"])
    first = write(out / "branch-setup.json", setup)
    initialized = call("2d-apply", part, final["project_revision"], operation(120), first)
    revision = initialized["project_revision"]
    embedded = initialized["embedded_revision"]
    sketch = setup["commands"][1]["sketch_id"]
    start = setup["commands"][3]["segment"]["start_id"]
    end = setup["commands"][3]["segment"]["end_id"]
    stable = call("2d-inspect", part)
    solution = lambda: call("2d-solve-report", part, sketch)["value"]
    assert solution()["points_m"][end][0] == 0.1

    reverse = {"type": "constraint.create", "sketch_id": sketch, "constraint": {
        "id": entity(121), "enabled": True, "relation": {"kind": "signed_distance_x",
        "from": start, "to": end, "target": {"kind": "literal", "value_m": -0.1}},
    }}
    batch = {**setup, "expected_revision": embedded, "commands": [reverse],
             "idempotency_key": "luma-branch-accept"}
    bare = write(out / "branch-bare.json", batch)
    refused = call("2d-apply", part, revision, operation(122), bare, status=3)
    assert "sketch.branch_change_required" in refused["details"]["provider_diagnostic"]
    batch["commands"].append({"type": "sketch.propose_branch", "sketch_id": sketch,
                              "seed": [{"id": start, "initial_m": [0.0, 0.0]},
                                       {"id": end, "initial_m": [-0.1, 0.0]}]})
    proposal = write(out / "branch-batch.json", batch)
    refused = call("2d-apply", part, revision, operation(123), proposal, status=3)
    assert "sketch.branch_confirmation_required" in refused["details"]["provider_diagnostic"]
    token = out / "branch-token.json"
    preview = call("2d-propose-branch", part, revision, proposal, token)
    branch_id = preview["branch"]["branch_token"]
    assert preview["sketch_solutions"][sketch]["points_m"][end][0] == -0.1
    assert call("2d-inspect", part) == stable
    assert solution()["branch_token"] is None
    alternate = out / "branch-alternate.json"
    call("2d-propose-branch", part, revision, proposal, alternate)
    original = token.read_bytes()
    call("2d-propose-branch", part, revision, proposal, token, status=2)
    assert token.read_bytes() == original

    # Exercise the expiry and recomputation checks with tokens for this fixture's test key.
    envelope = json.loads(original)
    payload = json.loads(envelope["payload"])
    domain = b"living-ir.multi-part-checkpoint/2\0luma-embedded-2d-branch/1\0"
    for name, change, code, status in (
        ("tampered", {"expires_at": 0}, "CheckpointAuthentication", 5),
        ("expired", {"expires_at": 0}, "PreparedExpired", 3),
        ("different-candidate", {"candidate_digest": "00" * 32}, "Embedded2dRejected", 3),
        ("different-build", {"integration": "unexpected"}, "PreparedContextChanged", 3),
    ):
        altered = json.dumps({**payload, **change}, separators=(",", ":"))
        auth = envelope["authentication"] if name == "tampered" else list(
            hmac.digest(bytes.fromhex(owner_key), domain + altered.encode(), "sha256")
        )
        wrong = write(out / f"branch-{name}.json", {"payload": altered, "authentication": auth})
        assert call("2d-accept-branch", wrong, operation(124), status=status)["code"] == code
        assert call("2d-inspect", part) == stable

    accept = ("2d-accept-branch", token, operation(125))
    call(*accept, status=73, crash="command-before-receipt")
    assert call("2d-inspect", part) == stable
    accepted = call(*accept)
    assert int(accepted["project_revision"]) == int(revision) + 1
    assert int(accepted["embedded_revision"]) == int(embedded) + 1
    assert accepted["document_revision"] == initialized["document_revision"]
    assert solution()["branch_token"] == branch_id
    assert solution()["points_m"][end][0] == -0.1
    assert call(*accept) == accepted
    assert call("2d-accept-branch", alternate, operation(126), status=4)["code"] == "Conflict"
    assert call("2d-accept-branch", alternate, operation(125), status=4)["code"] == "RequestIdReuse"
    print("PASS explicit solver branch proposal, authenticated acceptance, stale rejection and rollback")


def qualify(cli, fixtures, lir2d, out, disabled, brep):
    project, part = entity(1), entity(2)
    db = out / "project.db"
    key = "19" * 32
    env = {**os.environ, "LUMA_CAD_PROJECT_KEY": key}
    env.pop("LUMA_CAD_TEST_CRASH_AT", None)

    def call(*args, status=0, crash=None, executable=cli):
        process = subprocess.run([executable, db, project, *args],
                                 env={**env, **({"LUMA_CAD_TEST_CRASH_AT": crash} if crash else {})},
                                 capture_output=True, text=True, check=False)
        assert process.returncode == status, (args, process.returncode, process.stdout, process.stderr)
        if crash:
            assert not process.stdout, process.stdout
            return None
        result = json.loads(process.stdout)
        return result["error" if status else "result"]

    base = json.loads((fixtures / "plate/parameters-100mm.json").read_bytes())
    base["document_id"] = part
    standalone = write(out / "standalone.json", base)
    seed = write(out / "seed.json", {**base, "ownership_mode": "embedded"})
    original = seed.read_bytes()
    batch_file = out / "batch.json"

    def batch(revision, name):
        return write(batch_file, {
            "api_version": "1.0", "request_id": entity(98), "project_id": project,
            "document_id": part, "expected_revision": revision, "idempotency_key": f"draft-{name}",
            "commands": [{"type": "draft.create", "draft_id": entity(61), "name": name}],
            "execution_policy": {"numeric_profile_id": "mechanical-si-v1",
                                 "source_policy": "pin_current", "solver_policy": "preserve_branch",
                                 "timeout_ms": 5000, "max_memory_bytes": 50331648},
        })

    rejected_file = out / "no-standalone-output.lir2d"
    rejected = subprocess.run([lir2d, "normalize", seed, "--output", rejected_file],
                              capture_output=True, check=False)
    assert rejected.returncode and not rejected_file.exists()
    native = call("init", part, operation(1))
    if disabled:
        assert call("2d-inspect", part, executable=disabled, status=3)["code"] == "UnsupportedEmbedded2d"
    assert call("2d-apply", part, "1", operation(2), batch("17", "unregistered"), status=3)["code"] == "UnsupportedEmbedded2d"
    wrong_owner = call("2d-register", part, "1", operation(3), standalone, status=3)
    assert wrong_owner["code"] == "UnsupportedEmbedded2d", wrong_owner
    registered = call("2d-register", part, "1", operation(4), seed)
    assert registered["project_revision"] == "2" and registered["embedded_revision"] == "17"
    assert registered["document_revision"] == native["document_revision"]
    assert call("2d-register", part, "1", operation(4), seed) == registered

    bad = source(out / "open.bundle.json", complete=False)
    good = source(out / "closed.bundle.json", complete=True)
    failure = call("2d-luma", part, "2", "17", operation(5), bad, status=3)
    assert failure["code"] == "Embedded2dRejected", failure
    state = call("2d-inspect", part)
    assert state["project_revision"] == "2" and state["embedded_revision"] == "17"
    assert not state["value"]["sketches"]

    negative = source(out / "negative.bundle.json", complete=True, height=-4)
    assert call("2d-luma", part, "2", "17", operation(11), negative, status=3)["code"] == "Embedded2dPlanRejected"
    assert call("2d-inspect", part) == state
    plan = ("2d-luma", part, "2", "17", operation(6), good)
    call(*plan, status=73, crash="command-before-receipt")
    assert call("2d-inspect", part) == state
    receipt = call(*plan)
    assert receipt["project_revision"] == "3" and receipt["embedded_revision"] == "18"
    assert int(receipt["document_revision"]) == int(native["document_revision"]) + 1
    assert call(*plan) == receipt
    solution = call("2d-solve-report", part, entity(10))
    assert solution["value"]["degrees_of_freedom"] == 0
    assert solution["value"]["feasibility"] == "feasible"
    assert solution["value"]["solver_policy"] == "preserve_branch"
    assert "branch_token" in solution["value"]
    accepted = call("2d-inspect", part)
    assert accepted["value"]["ownership_mode"] == "embedded"
    assert entity(12) in accepted["value"]["profiles"] and entity(60) in accepted["value"]["drafts"]
    entities = accepted["value"]["drafts"][entity(60)]["entities"]
    assert entities[entity(71)]["kind"]["x_m"] == 0.005
    assert entities[entity(72)]["kind"]["y_m"] == 0.002
    assert entities[entity(73)]["kind"]["start_id"] == entity(71)
    assert call("2d-luma", part, "2", "17", operation(6), bad, status=4)["code"] == "RequestIdReuse"
    assert call("2d-apply", part, "3", operation(7), batch("17", "stale"), status=4)["code"] == "Conflict"
    assert call("2d-inspect", part) == accepted

    # Concurrent owner requests race against the same authenticated checkpoint.
    draft = batch("18", "next")
    calls = [
        subprocess.Popen([cli, db, project, "2d-apply", part, "3", operation(number), draft],
                         env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        for number in (8, 9)
    ]
    for child in calls:
        stdout, stderr = child.communicate(timeout=30)
        assert child.returncode in (0, 4), (stdout, stderr)
    assert sorted(child.returncode for child in calls) == [0, 4]
    final = call("2d-inspect", part)
    assert final["project_revision"] == "4" and final["embedded_revision"] == "19"
    assert final["part_revision"] == receipt["document_revision"]
    assert call(*plan) == receipt and seed.read_bytes() == original
    assert entity(61) in final["value"]["drafts"]
    if brep:
        def evaluate(*args):
            process = subprocess.run([brep, db, project, out / "derived.db", *args],
                                     env=env, capture_output=True, text=True, check=False)
            assert process.returncode == 0, (args, process.stdout, process.stderr)
            return json.loads(process.stdout)["result"]

        pin = evaluate("source", part, entity(80))
        assert pin["revision"] == receipt["document_revision"] and pin["sketch"] == entity(10)
        wire = evaluate("sketch-profile", part, entity(80))
        assert {curve["id"] for curve in wire["sketch"]["curves"]} == {entity(40 + i) for i in range(5)}
        published = evaluate("publish", part, entity(80), pin["revision"], pin["definition_hash"], entity(99))
        assert published["state"] == "published"
        volume = evaluate("measure", entity(99))["volume_m3"]
        expected = (0.04 * 0.03 - math.pi * 0.004 ** 2) * 0.004
        assert volume[0] <= expected <= volume[1], volume
    qualify_branch(call, fixtures, out, project, part, final, key)
    print("PASS embedded 2D Core CAS, crash rollback, concurrent conflict, replay and typed Luma extrusion")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project_cli", type=lambda value: Path(value).resolve())
    parser.add_argument("ir2d_checkout", type=Path)
    parser.add_argument("standalone_lir2d", type=lambda value: Path(value).resolve())
    parser.add_argument("--disabled-cli", type=lambda value: Path(value).resolve())
    parser.add_argument("--brep-cli", type=lambda value: Path(value).resolve())
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="luma-embedded-2d-") as directory:
        qualify(args.project_cli, args.ir2d_checkout / "fixtures", args.standalone_lir2d,
                Path(directory), args.disabled_cli, args.brep_cli)


if __name__ == "__main__":
    main()
