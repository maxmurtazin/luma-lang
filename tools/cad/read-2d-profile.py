#!/usr/bin/env python3
"""Export the evaluated Sketch/Profile of an authoritative standalone lir2d snapshot."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid


class ProfileFailure(Exception):
    def __init__(self, code, detail):
        self.code = code
        self.detail = detail


def lir2d(executable, *args):
    result = subprocess.run([executable, *args], capture_output=True, text=True, check=False)
    if result.returncode:
        try:
            diagnostic = json.loads(result.stderr)
        except json.JSONDecodeError:
            diagnostic = result.stderr.strip()
        raise ProfileFailure("Living2dError", diagnostic)
    return json.loads(result.stdout)


def decimal(value):
    if not math.isfinite(value):
        raise ProfileFailure("InvalidProfile", "non-finite provider geometry")
    return str(value) if value else "0"


def point(values):
    return [decimal(value) for value in values]


def edge(curve, direction):
    return {"curve_id": curve, "direction": direction}


def loop_id(sketch, curve):
    return str(uuid.uuid5(uuid.UUID(sketch), f"luma-profile-loop/{curve}"))


def bridge(document, snapshot, measured, sketch_report, input_hash):
    profile = measured["profile"]
    sketch_id = profile["sketch_id"]
    revision = document["revision"]
    document_id = document["document_id"]
    if (profile["document_id"] != document_id or snapshot["document_id"] != document_id
            or sketch_report["sketch_id"] != sketch_id
            or any(item["revision"] != revision for item in (snapshot, measured, sketch_report))):
        raise ProfileFailure("SourceMismatch", "provider responses must come from the same source pin")
    if (profile["purpose"] != "solid_profile" or profile["representation"] != "analytic"
            or profile["workplane_basis"] != "xy" or not profile["outer"]):
        raise ProfileFailure("UnsupportedProfile", "expected an analytic solid profile on XY")

    sketch = snapshot["sketches"][sketch_id]
    curves = []
    outer_edges = []
    segments_by_endpoints = {}
    for segment in profile["outer"]:
        curve_id = segment["entity_id"]
        authored = sketch["entities"][curve_id]
        direct = (authored["start_id"], authored["end_id"])
        traversal = (segment["from_id"], segment["to_id"])
        if direct == traversal:
            direction = "forward"
            start, end = segment["start_m"], segment["end_m"]
        elif direct == traversal[::-1]:
            direction = "reverse"
            start, end = segment["end_m"], segment["start_m"]
        else:
            raise ProfileFailure("SourceMismatch", f"endpoint identity changed for {curve_id}")
        curves.append({"id": curve_id, "kind": "segment", "start": point(start), "end": point(end)})
        outer_edges.append(edge(curve_id, direction))
        segments_by_endpoints.setdefault(frozenset(direct), []).append(curve_id)

    outer = loop_id(sketch_id, min(item["curve_id"] for item in outer_edges))
    loops = [{"id": outer, "role": "outer", "edges": outer_edges}]
    for hole in profile["holes"]:
        curve_id = hole["entity_id"]
        if hole["orientation"] != "cw":
            raise ProfileFailure("InvalidProfile", f"unexpected circular hole direction for {curve_id}")
        curves.append({"id": curve_id, "kind": "circle", "center": point(hole["center_m"]),
                       "radius": decimal(hole["radius_m"])})
        loops.append({"id": loop_id(sketch_id, curve_id), "parent_id": outer, "role": "hole",
                      "edges": [edge(curve_id, "reverse")]})

    common = {"schema_version": 1, "length_unit": "m", "angular_unit": "rad",
              "frame": {"origin_m": ["0", "0", "0"], "x": ["1", "0", "0"],
                        "y": ["0", "1", "0"], "z": ["0", "0", "1"]}}
    constraints = []
    for relation_id, constraint in sketch["constraints"].items():
        relation = constraint["relation"]
        kind = relation["kind"]
        if constraint["enabled"] and kind in ("horizontal", "vertical"):
            candidates = segments_by_endpoints.get(frozenset((relation["from"], relation["to"])), [])
            if len(candidates) == 1:
                constraints.append({"id": relation_id, "kind": kind, "curve_ids": candidates})
    return {
        "sketch": {**common, "kind": "Sketch", "id": sketch_id, "curves": curves,
                   "constraints": constraints},
        "profile": {**common, "kind": "Profile", "loops": loops,
                    "tolerance_m": decimal(snapshot["numeric_profile"]["position_abs_m"]),
                    "minimum_web_m": decimal(profile["required_clearance_m"]),
                    "source": {"type": "Sketch", "entity_id": sketch_id, "revision": revision,
                               "target": {"kind": "standalone_2d", "authority_id": "living-ir-2d",
                                          "project_id": document_id, "document_id": document_id}}},
        "source_container_sha256": input_hash,
        "source_profile_id": profile["profile_id"],
        "validation": {"provider": "living-ir-2d", "numeric_profile_id": profile["numeric_profile_id"],
                       "material_area_m2": profile["material_area_m2"],
                       "minimum_clearance_m": profile["minimum_clearance_m"],
                       "excluded_entity_ids": profile["excluded_entity_ids"]},
        "solver_report": {**sketch_report["solution"], "policy": sketch["solver_policy"],
                          "native_branch_token": sketch.get("branch_token"),
                          "native_constraints": sketch["constraints"],
                          "native_continuation_seed": sketch["continuation_seed"]},
    }


def export(executable, source, profile_id):
    caps = lir2d(executable, "capabilities")
    limit = caps["max_container_bytes"] if source.suffix == ".lir2d" else caps["max_input_bytes"]
    with source.open("rb") as file:
        data = file.read(limit + 1)
    if len(data) > limit:
        raise ProfileFailure("ResourceLimit", "snapshot exceeds lir2d container limit")
    # All provider reads use identical bytes even if an editor saves a new revision.
    with tempfile.TemporaryDirectory(prefix="luma-cad-profile-") as tmp:
        backup = Path(tmp) / ("source" + source.suffix)
        backup.write_bytes(data)
        document = lir2d(executable, "inspect", backup, "--json")
        if document["ownership_mode"] != "standalone":
            raise ProfileFailure("UnsupportedEmbedded2d", "embedded 2D requires a registered Core CAS adapter")
        measured = lir2d(executable, "profile.measure", backup, "--profile", profile_id)
        profile = measured["profile"]
        if profile["profile_id"] != str(uuid.UUID(profile_id)):
            raise ProfileFailure("SourceMismatch", "provider returned a different profile")
        report = lir2d(executable, "sketch.measure", backup, "--sketch", profile["sketch_id"])
        normalized = Path(tmp) / "snapshot.json"
        lir2d(executable, "normalize", backup, "--output", normalized)
        snapshot = json.loads(normalized.read_text(encoding="utf-8"))
        return bridge(document, snapshot, measured, report, hashlib.sha256(data).hexdigest())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lir2d", type=lambda value: Path(value).resolve())
    parser.add_argument("source", type=Path)
    parser.add_argument("profile", type=lambda value: str(uuid.UUID(value)))
    args = parser.parse_args()
    try:
        print(json.dumps(export(args.lir2d, args.source, args.profile), allow_nan=False))
    except ProfileFailure as failure:
        print(json.dumps({"code": failure.code, "detail": failure.detail}), file=sys.stderr)
        return 1
    except (OSError, KeyError, TypeError, ValueError) as failure:
        print(json.dumps({"code": "SourceError", "detail": str(failure)}), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
