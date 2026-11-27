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


def source(path, complete, height=4, *, width=40, recipe=False, feature=80):
    program = ["use units", "use sketch", "use brep"]
    if recipe:
        program.extend(["use cad", "use param",
                        f'width = param.length("width", units.mm({width}))',
                        f'height = param.length("height", units.mm({height}))'])
    else:
        program.append("use draft")
    program.append(f's = sketch.create("{entity(10)}", "{entity(11)}", "{entity(12)}")')
    coordinates = [(0, 0), (width, 0), (width, 30), (0, 30), (20, 15), (24, 15)]
    for index, (x, y) in enumerate(coordinates):
        x_length = "param.value(width)" if recipe and index in (1, 2) else f"units.mm({x})"
        xyz = f"{x_length}, units.mm({y})"
        program.extend([
            f'p{index} = sketch.point(s, "{entity(20 + index)}", {xyz})',
            f'sketch.fix(s, "{entity(30 + index)}", p{index}, {xyz})',
        ])
    for index in range(4 if complete else 3):
        program.append(f'sketch.line(s, "{entity(40 + index)}", p{index}, p{(index + 1) % 4})')
    program.extend([
        f'sketch.circle(s, "{entity(44)}", p4, p5)', "profile = sketch.profile(s)",
    ])
    if recipe:
        program.extend([f'body = brep.extrude("{entity(feature)}", profile, param.value(height))',
                        'cad.output("solid", body)'])
    else:
        program.extend([
            f'd = draft.create("{entity(60)}")',
            f'layer = draft.layer(d, "{entity(70)}")',
            f'a = draft.point(layer, "{entity(71)}", units.mm(0), units.mm(0))',
            f'b = draft.point(layer, "{entity(72)}", units.mm(10), units.mm(0))',
            f'edge = draft.add(layer, "{entity(73)}", a, b)',
            'draft.transform(edge, units.mm(5), units.mm(2))',
            f'body = brep.extrude("{entity(feature)}", profile, units.mm({height}))',
        ])
    return bundle(path, "draw-part.luma", program)


def bundle(path, entry, program):
    files = {entry: "\n".join(program) + "\n"}
    digest = lcan.digest("luma.cad.source", lcan.encode({
        "bundle_version": {"$u64": "1"}, "entry": entry, "files": files,
    }))
    return write(path, {"bundle_version": 1, "entry": entry, "files": files, "source_digest": digest})


def qualify_branch(call, fixtures, out, project, part, final, owner_key, expected_sources=()):
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
    assert sorted((source["guard"], source["revision"]) for source in preview["source_read_set"]) == sorted(expected_sources)
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
    return setup


def qualify_core_length(call, out, setup, part):
    producer = entity(130)
    revision = call("2d-inspect", part)["project_revision"]
    created = call("create-part", producer, revision, operation(130))
    examples = Path(__file__).resolve().parents[2] / "examples/cad"
    recipe = out / "length-recipe.token"
    call("preview", "recipe", producer, created["project_revision"], examples / "plate-geometry.json", recipe)
    installed = call("apply", recipe, operation(131))
    instance = uuid.UUID(call("recipe-info", producer)["recipe"])
    blank = uuid.uuid5(instance, "lowering-v1/feature/blank")
    width = str(uuid.uuid5(blank, "lowering-v1/parameter/width"))
    pin = call("2d-length-source", producer, width)
    assert pin["value_m"] == 0.1 and pin["project_revision"] == installed["project_revision"]
    missing = call("2d-length-source", producer, entity(131), status=3)
    assert missing["code"] == "MissingCoreLength"
    wrong_type = call("2d-length-source", producer, str(blank), status=3)
    assert wrong_type["code"] == "CoreLengthTypeMismatch"

    parameter = setup["commands"][-1]["constraint"]["relation"]["target"]["parameter_id"]
    current = call("2d-inspect", part)
    batch = {**setup, "expected_revision": current["embedded_revision"],
             "idempotency_key": "core-length-binding"}
    length = {"type": "parameter.set_literal", "parameter_id": parameter,
              "quantity_kind": "length", "value_si": pin["value_m"], "core_source": pin["source"]}

    def apply(name, number, commands, status=0):
        path = write(out / f"{name}.json", {**batch, "idempotency_key": name, "commands": commands})
        return call("2d-apply", part, current["project_revision"], operation(number), path, status=status)

    spoof = apply("forged-value", 132, [{**length, "value_si": 0.11}], status=3)
    assert spoof["code"] == "CoreLengthValueMismatch"
    forged = {**pin["source"], "definition_hash": "00" * 32}
    assert apply("forged-hash", 133, [{**length, "core_source": forged}], status=4)["code"] == "Conflict"
    assert call("2d-inspect", part) == current
    apply("bind-length", 134, [length])
    current = call("2d-inspect", part)
    batch["expected_revision"] = current["embedded_revision"]

    sketch = setup["commands"][1]["sketch_id"]
    segment = setup["commands"][3]["segment"]
    start, end = segment["start_id"], segment["end_id"]
    relation = {"kind": "signed_distance_x", "from": start, "to": end,
                "target": {"kind": "parameter", "parameter_id": parameter}}
    change = {"type": "constraint.update", "constraint_id": entity(121), "patch": {"relation": relation}}
    propose = {"type": "sketch.propose_branch", "sketch_id": sketch,
               "seed": [{"id": start, "initial_m": [0, 0]}, {"id": end, "initial_m": [0.1, 0]}]}
    proposed = write(out / "length-branch.json", {**batch, "commands": [change, propose]})
    branch = out / "length-branch.token"
    report = call("2d-propose-branch", part, current["project_revision"], proposed, branch)
    assert report["core_read_set"] == [{"parameter_id": parameter, "source": pin["source"], "value_m": 0.1}]

    command = json.loads((examples / "plate-width-command.json").read_bytes())
    entry = command["entry"]
    files = command["files"]
    files[entry] = files[entry].replace("00000000-0000-4000-8000-000000000002", producer)
    command["source_digest"] = lcan.digest("luma.cad.source", lcan.encode({
        "bundle_version": {"$u64": "1"}, "entry": entry, "files": files,
    }))
    edit = write(out / "edit-core-length.json", command)
    edit_token = out / "edit-core-length.token"
    call("preview", "command", producer, current["project_revision"], edit, edit_token)
    call("apply", edit_token, operation(135))
    assert call("2d-accept-branch", branch, operation(136), status=4)["code"] == "Conflict"
    current = call("2d-inspect", part)
    assert current["embedded_revision"] == batch["expected_revision"]
    disable = {"type": "constraint.update", "constraint_id": entity(121), "patch": {"enabled": False}}
    assert apply("keep-stale-length", 137, [disable], status=4)["code"] == "Conflict"
    assert call("2d-inspect", part) == current
    updated = call("2d-length-source", producer, width)
    assert updated["value_m"] == 0.12 and updated["source"] != pin["source"]
    apply("repin-length", 138,
          [disable, {**length, "value_si": updated["value_m"], "core_source": updated["source"]}])
    solved = call("2d-solve-report", part, sketch)["value"]
    assert math.isclose(abs(solved["points_m"][end][0]), 0.12)
    print("PASS authenticated Core Length, stale proposal and explicit repin")
    return producer, blank, pin, updated


def qualify_dependencies(call, fixtures, out, project, parent, seed, key, sources):
    producer, blank, old, new = sources
    part = entity(170)
    revision = call("2d-inspect", parent)["project_revision"]
    created = call("create-part", part, revision, operation(170))
    common = {"provider_id": "luma.core.part/1", "project_id": project, "document_id": producer}
    pinned = {**common, "object_id": old["source"]["entity_id"], "revision_policy": "pinned_revision",
              "pinned_revision": old["source"]["native_revision"], "content_digest": old["source"]["definition_hash"]}
    follow = {**common, "object_id": str(blank), "revision_policy": "follow_latest"}
    seed = {**seed, "document_id": part, "dependencies": [pinned, follow]}
    forged = write(out / "forged-dependency.json", {
        **seed, "dependencies": [{**pinned, "content_digest": "00" * 32}],
    })
    failure = call("2d-register", part, created["project_revision"], operation(171), forged, status=4)
    assert failure["code"] == "Conflict"
    unsupported = write(out / "unsupported-dependency.json", {
        **seed, "dependencies": [{**pinned, "provider_id": "external"}],
    })
    failure = call("2d-register", part, created["project_revision"], operation(172), unsupported, status=3)
    assert failure["code"] == "UnsupportedEmbedded2d"
    path = write(out / "dependency-seed.json", seed)
    state = call("2d-register", part, created["project_revision"], operation(173), path)
    out = out / "dependency-branch"
    out.mkdir()
    qualify_branch(call, fixtures, out, project, part, state, key, expected_sources=[
        ("expect_pinned_version", old["source"]["native_revision"]),
        ("expect_head", new["source"]["native_revision"]),
    ])
    print("PASS project-local Core entity dependency guards")


def qualify_recipe(call, evaluate, out, seed, setup, parent):
    part = entity(140)
    revision = call("2d-inspect", parent)["project_revision"]
    created = call("create-part", part, revision, operation(140))
    initial = write(out / "recipe-seed.json", {**seed, "document_id": part})
    call("2d-register", part, created["project_revision"], operation(141), initial)
    state = call("2d-inspect", part)

    def preview(bundle, name, status=0, action="2d-recipe-preview"):
        token = out / f"{name}.token"
        report = call(action, part, state["project_revision"], state["embedded_revision"],
                      bundle, token, status=status)
        assert token.exists() == (status == 0)
        return report, token

    def publish(number, width, height, extra=False):
        pin = evaluate("source", part, entity(80))
        assert pin["sketch"] == entity(10)
        wire = evaluate("sketch-profile", part, entity(80))
        expected = {entity(40 + i) for i in range(5)}
        if extra:
            expected.add(entity(164))
        assert {curve["id"] for curve in wire["sketch"]["curves"]} == expected
        op = entity(number)
        result = evaluate("publish", part, entity(80), pin["revision"], pin["definition_hash"], op)
        assert result["state"] == "published"
        measure = evaluate("measure", op)
        volume = (width * 0.03 - math.pi * (0.004 ** 2 + (0.003 ** 2 if extra else 0))) * height
        assert measure["volume_m3"][0] <= volume <= measure["volume_m3"][1]
        return measure

    open_profile = source(out / "recipe-open.json", complete=False, recipe=True)
    assert preview(open_profile, "open-profile", status=3)[0]["code"] == "Embedded2dRejected"
    assert call("2d-inspect", part) == state
    examples = Path(__file__).resolve().parents[2] / "examples/cad"
    original = examples / "sketch-recipe.json"
    report, token = preview(original, "original")
    assert report["candidate_changed"]
    assert call("2d-inspect", part) == state
    call("2d-recipe-apply", token, operation(142), status=73, crash="command-before-receipt")
    assert call("2d-inspect", part) == state
    accepted = call("2d-recipe-apply", token, operation(142))
    assert call("2d-recipe-apply", token, operation(142)) == accepted
    state = call("2d-inspect", part)
    info = call("2d-recipe-info", part)
    assert info["bundle"] == json.loads(original.read_bytes())
    assert info["profile_id"] == entity(12) and info["feature_id"] == entity(80)
    assert info["part_revision"] == accepted["document_revision"]
    if evaluate:
        prior = publish(143, 0.04, 0.004)

    raw = write(out / "recipe-raw-edit.json", {**setup, "document_id": part,
                "expected_revision": state["embedded_revision"]})
    failure = call("2d-apply", part, state["project_revision"], operation(144), raw, status=4)
    assert failure["code"] == "ManagedEntityConflict"
    cad1 = examples / "plate-geometry.json"
    failure = call("preview", "recipe", part, state["project_revision"], cad1,
                   out / "other-owner.token", status=4)
    assert failure["code"] == "ManagedEntityConflict"
    incompatible = source(out / "recipe-id-change.json", complete=True, recipe=True, feature=81)
    assert preview(incompatible, "topology-change", status=3)[0]["code"] == "SketchRecipeTopologyChanged"
    invalid = source(out / "recipe-invalid-width.json", complete=True, recipe=True, width=10)
    assert preview(invalid, "invalid-width", status=3)[0]["code"] == "Embedded2dRejected"
    assert call("2d-inspect", part) == state

    unchanged, same = preview(original, "same-recipe")
    assert not unchanged["candidate_changed"]
    no_change = call("2d-recipe-apply", same, operation(145))
    assert no_change["project_revision"] == accepted["project_revision"]
    assert no_change["document_revision"] == accepted["document_revision"]
    assert call("2d-inspect", part) == state
    newer = source(out / "recipe-new.json", complete=True, recipe=True, width=50, height=6)
    _, change = preview(newer, "recipe-update")
    changed = call("2d-recipe-apply", change, operation(146))
    assert int(changed["document_revision"]) == int(accepted["document_revision"]) + 1
    assert int(changed["embedded_revision"]) == int(accepted["embedded_revision"]) + 1
    assert call("2d-recipe-apply", same, operation(147), status=4)["code"] == "Conflict"
    solution = call("2d-solve-report", part, entity(10))["value"]
    assert solution["degrees_of_freedom"] == 0 and solution["points_m"][entity(22)] == [0.05, 0.03]
    if evaluate:
        publish(148, 0.05, 0.006)
        assert evaluate("measure", entity(143)) == prior
    state = call("2d-inspect", part)

    def command(name, edits, target=part):
        return bundle(out / f"{name}.json", "dimensions.luma", [
            "use project", "use param", "use units",
            f'p = project.part("{target}")', 'width = project.parameter(p, "width")',
            *edits,
        ])

    invalid = command("invalid-edit", ["param.set(width, units.mm(10))"])
    assert preview(invalid, "invalid-edit", 3, "2d-command-preview")[0]["code"] == "Embedded2dRejected"
    foreign = command("foreign-edit", ["param.set(width, units.mm(60))"], target=parent)
    assert preview(foreign, "foreign-edit", 3, "2d-command-preview")[0]["code"] == "Embedded2dCommandRejected"
    assert call("2d-inspect", part) == state

    edit = command("edit-width", [
        "param.set(width, units.mm(60))",
        'height = project.parameter(p, "height")',
        "param.set(height, units.mm(7))",
    ])
    report, edit_token = preview(edit, "edit-width", action="2d-command-preview")
    assert report["candidate_changed"] and report["source_digest"] == json.loads(newer.read_bytes())["source_digest"]
    assert report["command_digest"] == json.loads(edit.read_bytes())["source_digest"]
    assert report["parameters"]["width"]["override_value"] == {"Length": 0.06}
    call("2d-recipe-apply", edit_token, operation(149), status=73, crash="command-before-receipt")
    assert call("2d-inspect", part) == state
    assert call("2d-recipe-info", part)["parameters"]["width"]["override_value"] is None
    receipt = call("2d-recipe-apply", edit_token, operation(149))
    assert call("2d-recipe-apply", edit_token, operation(149)) == receipt
    assert call("2d-recipe-apply", edit_token, operation(150), status=4)["code"] == "Conflict"
    state = call("2d-inspect", part)
    info = call("2d-recipe-info", part)
    assert info["bundle"] == json.loads(newer.read_bytes())
    assert info["parameters"]["width"]["override_value"] == {"Length": 0.06}
    assert info["parameters"]["height"]["override_value"] == {"Length": 0.007}
    assert not preview(edit, "repeated-edit", action="2d-command-preview")[0]["candidate_changed"]
    if evaluate:
        publish(151, 0.06, 0.007)

    new_defaults = source(out / "new-defaults.json", complete=True, recipe=True, width=55, height=8)
    wire = json.loads(new_defaults.read_bytes())
    entry = wire["entry"]
    lines = wire["files"][entry].replace("param.value(width)", "units.mm(55)").splitlines()
    removed_width = bundle(out / "removed-width.json", entry,
                           [line for line in lines if not line.startswith("width = ")])
    assert preview(removed_width, "removed-width", 3)[0]["code"] == "Embedded2dPlanRejected"
    assert call("2d-inspect", part) == state
    _, defaults_token = preview(new_defaults, "new-defaults")
    call("2d-recipe-apply", defaults_token, operation(152))
    state = call("2d-inspect", part)
    info = call("2d-recipe-info", part)
    assert info["parameters"]["width"]["declaration"]["default"] == {"Length": 0.055}
    assert info["parameters"]["width"]["override_value"] == {"Length": 0.06}
    assert info["parameters"]["height"]["override_value"] == {"Length": 0.007}
    assert call("2d-solve-report", part, entity(10))["value"]["points_m"][entity(22)] == [0.06, 0.03]
    reset = command("reset-dimensions", [
        "param.reset(width)",
        'height = project.parameter(p, "height")',
        "param.reset(height)",
    ])
    _, reset_token = preview(reset, "reset-dimensions", action="2d-command-preview")
    call("2d-recipe-apply", reset_token, operation(153))
    info = call("2d-recipe-info", part)
    assert info["parameters"]["width"]["override_value"] is None
    assert info["parameters"]["height"]["override_value"] is None
    if evaluate:
        publish(154, 0.055, 0.008)
    state = call("2d-inspect", part)
    wire = json.loads(new_defaults.read_bytes())
    entry = wire["entry"]
    lines = wire["files"][entry].splitlines()
    profile_index = lines.index("profile = sketch.profile(s)")
    lines[profile_index:profile_index] = [
        f'c2 = sketch.point(s, "{entity(160)}", units.mm(40), units.mm(15))',
        f'r2 = sketch.point(s, "{entity(161)}", units.mm(43), units.mm(15))',
        f'sketch.fix(s, "{entity(162)}", c2, units.mm(40), units.mm(15))',
        f'sketch.fix(s, "{entity(163)}", r2, units.mm(43), units.mm(15))',
        f'sketch.circle(s, "{entity(164)}", c2, r2)',
    ]
    expanded = bundle(out / "two-holes.json", entry, lines)
    _, token = preview(expanded, "two-holes")
    call("2d-recipe-apply", token, operation(155))
    state = call("2d-inspect", part)
    assert preview(new_defaults, "remove-hole", 3)[0]["code"] == "SketchRecipeTopologyChanged"
    if evaluate:
        expanded_artifact = publish(156, 0.055, 0.008, extra=True)

    detach = ("2d-recipe-detach", part, state["project_revision"], operation(157))
    call(*detach, status=73, crash="command-before-receipt")
    assert not call("2d-recipe-info", part)["detached"]
    detached = call(*detach)
    assert detached == call(*detach)
    assert int(detached["project_revision"]) == int(state["project_revision"]) + 1
    assert detached["document_revision"] == state["part_revision"]
    assert detached["embedded_revision"] == state["embedded_revision"]
    state = call("2d-inspect", part)
    assert call("2d-recipe-info", part)["detached"]
    assert preview(expanded, "detached-recipe", 3)[0]["code"] == "DetachedRecipe"
    raw = write(out / "after-detach.json", {**setup, "document_id": part, "expected_revision": state["embedded_revision"]})
    call("2d-apply", part, state["project_revision"], operation(158), raw)
    assert call("2d-recipe-info", part)["detached"]
    if evaluate:
        assert evaluate("measure", entity(156)) == expanded_artifact
    print("PASS additive recipe topology, frozen B-rep and detach replay")
    print("PASS Sketch recipe rebuild, command overrides and reset, Core CAS and receipt replay")


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
    evaluate = None
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
    setup = qualify_branch(call, fixtures, out, project, part, final, key)
    sources = qualify_core_length(call, out, setup, part)
    initial_seed = json.loads(seed.read_bytes())
    qualify_dependencies(call, fixtures, out, project, part, initial_seed, key, sources)
    qualify_recipe(call, evaluate, out, initial_seed, setup, part)
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
