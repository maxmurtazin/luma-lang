#!/usr/bin/env python3
"""Exercise standalone Sketch/Profile and drawing exports against pinned lir2d fixtures."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess
import sys
from xml.etree import ElementTree

ROOT = Path(__file__).resolve().parents[2]
PROFILE = "54000000-0000-4000-8000-000000000003"
DRAFT = "50000000-0000-4000-8000-000000000001"
SHEET = "56000000-0000-4000-8000-000000000001"
CORE_PLATE = "00000000-0000-0000-0000-00000000006e"


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def write(path, data):
    with path.open("x", encoding="utf-8") as file:
        json.dump(data, file, ensure_ascii=False)


def run(*args, error=None):
    result = subprocess.run(args, text=True, capture_output=True, check=False)
    if error is not None:
        assert result.returncode and not result.stdout, result
        diagnostic = json.loads(result.stderr)
        assert diagnostic["code"] == error, diagnostic
        return diagnostic
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def profiles(lir2d, fixtures, out):
    initial, edited = (out / f"profile-{size}.lir2d" for size in (100, 120))
    run(lir2d, "apply", fixtures / "plate/parameters-100mm.json", "--commands",
        fixtures / "profile/create-control-plate.json", "--output", initial)
    run(lir2d, "apply", initial, "--commands", fixtures / "profile/resize-control-plate-120mm.json",
        "--output", edited)
    bridge = (sys.executable, "-B", ROOT / "tools/cad/read-2d-profile.py", lir2d)
    first, second = (run(*bridge, source, PROFILE) for source in (initial, edited))
    for size, source, pair, revision in ((100, initial, first, "18"), (120, edited, second, "19")):
        reference = pair["profile"]["source"]
        assert reference["revision"] == revision and reference["target"]["kind"] == "standalone_2d"
        assert pair["source_container_sha256"] == hashlib.sha256(source.read_bytes()).hexdigest()
        assert pair["solver_report"]["degrees_of_freedom"] == 0
        assert pair["solver_report"]["rank"] == 18
        assert pair["solver_report"]["policy"] == "preserve_branch"
        native_constraints = pair["solver_report"]["native_constraints"]
        canonical_constraints = pair["sketch"]["constraints"]
        assert len(native_constraints) == 17 and len(canonical_constraints) == 4
        for constraint in canonical_constraints:
            assert native_constraints[constraint["id"]]["relation"]["kind"] == constraint["kind"]
        assert pair["solver_report"]["max_residual_m"] <= pair["solver_report"]["solve_tolerance_m"]
        expected_area = size / 1000 * 0.06 - 2 * math.pi * 0.003 ** 2
        assert math.isclose(pair["validation"]["material_area_m2"], expected_area, abs_tol=1e-12)
        assert math.isclose(pair["validation"]["minimum_clearance_m"], 0.007, abs_tol=1e-12)
        write(out / f"sketch-profile-{size}.json", pair)
    assert first["sketch"]["id"] == second["sketch"]["id"]
    assert [item["id"] for item in first["sketch"]["curves"]] == [item["id"] for item in second["sketch"]["curves"]]
    assert first["profile"]["loops"] == second["profile"]["loops"]

    normalized = out / "normalized.json"
    run(lir2d, "normalize", initial, "--output", normalized)
    embedded = out / "embedded.json"
    write(embedded, {**read(normalized), "ownership_mode": "embedded"})
    before = embedded.read_bytes()
    run(*bridge, embedded, PROFILE, error="UnsupportedEmbedded2d")
    forbidden = out / "forbidden.lir2d"
    run(lir2d, "apply", embedded, "--commands", fixtures / "profile/resize-control-plate-120mm.json",
        "--output", forbidden, error="SCHEMA_UNSUPPORTED")
    assert embedded.read_bytes() == before and not forbidden.exists()
    return initial


def drawings(lir2d, fixtures, out, font, profile):
    def apply(source, fixture, result, revision, edit=None):
        request = read(fixtures / fixture)
        request["expected_revision"] = revision
        if edit:
            edit(request)
        batch = result.with_suffix(".json")
        write(batch, request)
        return run(lir2d, "apply", source, "--commands", batch, "--output", result)

    draft = out / "draft.lir2d"
    applied = apply(fixtures / "plate/parameters-100mm.json", "draft/create-plate.json", draft, "17")
    dimensioned = out / "dimensioned.lir2d"
    apply(draft, "draft/dimension-plate.json", dimensioned, applied["commit"]["new_revision"])
    dimensions = run(lir2d, "draft.measure", dimensioned, "--draft", DRAFT)["dimensions"]
    labels = {dimension["label"] for dimension in dimensions.values()}
    assert labels == {"100 mm", "60 mm", "Ø6 mm"}

    # Association must resolve accepted source IDs before a drawing can be released.
    request = read(fixtures / "draft/dimension-plate.json")
    request["expected_revision"] = "18"
    request["commands"][1]["dimension"]["measurement"]["to"]["id"] = SHEET
    bad_batch, bad_output = (out / f"missing-dim-source.{suffix}" for suffix in ("json", "lir2d"))
    write(bad_batch, request)
    run(lir2d, "apply", draft, "--commands", bad_batch, "--output", bad_output, error="REFERENCE_MISSING")
    assert not bad_output.exists()

    cases = []
    for paper, width, height in (("a4", 297, 210), ("a3", 420, 297)):
        name = f"sheet-{paper}"
        sheet = out / f"{name}.lir2d"

        def edit(request):
            request["commands"][0]["sheet"].update(sheet_width_mm=width, sheet_height_mm=height,
                                                    name=f"Control plate · {paper.upper()}")

        created = apply(dimensioned, "sheet/create-plate-a4.json", sheet, "19", edit)
        half = out / f"{name}-half.lir2d"
        apply(sheet, "sheet/set-plate-scale-1-2.json", half, created["commit"]["new_revision"])
        for source, scale, radius in ((sheet, "1:1", "3"), (half, "1:2", "1.5")):
            exports = {}
            for fmt in ("svg", "pdf"):
                target = source.with_suffix(f".{fmt}")
                report = run(lir2d, f"sheet.export-{fmt}", source, "--sheet", SHEET,
                             "--policy", "manufacturing", "--font", font, "--output", target)
                content = target.read_bytes()
                assert report["page_mm"] == [width, height] and report["view"]["scale"] == scale
                assert report["release_policy"] == "manufacturing"
                assert report["source_read_set"][0]["source_id"] == DRAFT
                assert report["source_read_set"][0]["revision"] == report["revision"]
                assert hashlib.sha256(content).hexdigest() == report["artifact_body_sha256"]
                if fmt == "svg":
                    svg = ElementTree.fromstring(content)
                    assert svg.attrib["width"] == f"{width}mm" and svg.attrib["height"] == f"{height}mm"
                    assert [item.attrib["r"] for item in svg.iter("{http://www.w3.org/2000/svg}circle")] == [radius, radius]
                    assert all(label.encode() in content for label in labels)
                else:
                    assert content.startswith(b"%PDF-")
                    media = re.search(rb"/MediaBox\s*\[0 0 ([\d.]+) ([\d.]+)\]", content)
                    assert media, "missing physical PDF MediaBox"
                    for points, mm in zip(media.groups(), (width, height)):
                        assert math.isclose(float(points), mm / 25.4 * 72, abs_tol=0.001)
                exports[fmt] = report["artifact_body_sha256"]
            cases.append({"paper": paper.upper(), "page_mm": [width, height], "scale": scale, "exports": exports})
    dxf = out / "profile.dxf"
    report = run(lir2d, "export-profile", profile, "--profile", PROFILE, "--format", "dxf", "--output", dxf)
    text = dxf.read_text(encoding="ascii")
    assert report["scale"] == "1:1" and report["output_unit"] == "mm"
    assert "$INSUNITS\n70\n4\n" in text and "0\nLWPOLYLINE\n" in text and text.count("0\nCIRCLE\n") == 2
    return {"sheets": cases, "profile_dxf_sha256": report["artifact_body_sha256"]}


def core_drawing(lir2d, checkout, core, out, font):
    fixtures = checkout / "fixtures"
    sheet = out / "core-sheet.lir2d"
    run(lir2d, "apply", fixtures / "plate/parameters-100mm.json", "--commands",
        fixtures / "sheet/create-core-plate-a4.json", "--output", sheet)
    source = run("bash", checkout / "tools/core/read-parameter.sh", core, "--drawing-plate",
                 core / "fixtures/hir-s1/positive/12-cad.json", CORE_PLATE)
    unique, ambiguous, head = (out / f"core-{name}.json" for name in ("geometry", "ambiguous", "head"))
    write(unique, [source])
    write(ambiguous, [source, source])
    write(head, {**{key: source[key] for key in ("document_id", "definition_hash", "native_revision")},
                 "parameters": []})

    base = (lir2d, "sheet.export-svg", sheet, "--sheet", SHEET)
    review = run(*base, "--policy", "review", "--core-head", head, "--core-geometry", unique,
                 "--font", font, "--output", out / "core-review.svg")
    assert len(review["views"]) == 4
    assert {view["projection"]["source_definition_hash"] for view in review["views"]} == {source["definition_hash"]}

    for geometry, reason in ((unique, "projection.quality_unproven"),
                             (ambiguous, "projection.core_source_ambiguous")):
        result = out / (geometry.stem + ".svg")
        rejection = run(*base, "--policy", "manufacturing", "--core-head", head, "--core-geometry", geometry,
                        "--font", font, "--output", result, error="EXPORT_NOT_READY")
        assert reason in rejection["details"]["reason"] and not result.exists()
        if geometry == ambiguous:
            assert CORE_PLATE in rejection["object_refs"]
    return {"source_revision": source["native_revision"], "definition_hash": source["definition_hash"],
            "review_sha256": review["artifact_body_sha256"],
            "manufacturing_rejections": ["projection.quality_unproven", "projection.core_source_ambiguous"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkout", type=Path)
    parser.add_argument("lir2d", type=lambda value: Path(value).resolve())
    parser.add_argument("font", type=Path, help="TrueType font covering dimension labels")
    parser.add_argument("output", type=Path, help="new directory for qualification artifacts")
    parser.add_argument("--core", type=Path, help="pinned Core checkout for the analytical plate release guard")
    args = parser.parse_args()
    providers = read(ROOT / "luma-cad.lock.json")["providers"]
    for name, checkout in (("ir-2d", args.checkout), ("core", args.core)):
        if checkout:
            commit = providers[name]["commit"]
            head = subprocess.check_output(["git", "-C", checkout, "rev-parse", "HEAD"], text=True).strip()
            changes = subprocess.check_output(["git", "-C", checkout, "status", "--porcelain",
                                               "--untracked-files=no"], text=True)
            if head != commit or changes:
                parser.error(f"expected a clean {name} checkout at {commit}")
    args.output.mkdir()
    fixtures = args.checkout / "fixtures"
    initial = profiles(args.lir2d, fixtures, args.output)
    drawing_report = drawings(args.lir2d, fixtures, args.output, args.font, initial)
    if args.core:
        drawing_report["core_analytic_review"] = core_drawing(args.lir2d, args.checkout, args.core, args.output, args.font)
    write(args.output / "report.json", {"provider_revision": providers["ir-2d"]["commit"], **drawing_report})
    print("PASS standalone 2D profile, ownership guard and A4/A3 PDF/SVG/DXF qualification")


if __name__ == "__main__":
    main()
