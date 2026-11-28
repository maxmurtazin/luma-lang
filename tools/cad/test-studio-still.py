"""Render one historical Studio revision through Luma and verify its durable PNG."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile

import lcan


ROOT = Path(__file__).resolve().parents[2]
REQUEST = "00000000-0000-4000-8000-000000000001"


def run(*args, code=0):
    result = subprocess.run(args, capture_output=True, check=False, timeout=90)
    assert result.returncode == code, (args, result.stdout, result.stderr)
    return result.stdout


def read(path):
    return json.loads(path.read_bytes())


def write(path, value):
    path.write_text(json.dumps(value), encoding="utf-8")
    return path


def bundle(path, source):
    entry = "invalid.luma"
    files = {entry: source}
    body = {"bundle_version": {"$u64": "1"}, "entry": entry, "files": files}
    return write(path, {
        **body, "bundle_version": 1,
        "source_digest": lcan.digest("luma.cad.source", lcan.encode(body)),
    })


def verify_master(delivered, task):
    assert delivered["status"] == "succeeded" and delivered["completed_frames"] == 1
    assert delivered["frame_recipe_hash"] == task["frame_recipe_hash"]
    output = Path(delivered["output"])
    master = read(output / "master.json")
    assert master["format"] == "studio-raster-master/1" and master["mode"] == "still"
    for field in ("presentation_id", "revision", "source_snapshot_digest", "inputs", "width", "height"):
        assert master[field] == task[field], field
    assert master["png"] == "RGB8/sRGB/full" and master["chromium"]
    assert len(master["frames"]) == 1 and not (output / "video").exists()
    frame = master["frames"][0]
    assert frame["exact_time"] == task["time"] and frame["index"] == 0
    scene_path = output.parent / "packet" / "scene.json"
    assert hashlib.sha256(scene_path.read_bytes()).hexdigest() == task["inputs"]["scene.json"].removeprefix("sha256:")
    assert read(scene_path)["frame"]["camera"] == task["camera"]
    png = (output / frame["path"]).read_bytes()
    assert hashlib.sha256(png).hexdigest() == frame["digest"].removeprefix("sha256:")
    assert png[:8] == b"\x89PNG\r\n\x1a\n" and png[12:16] == b"IHDR"
    assert struct.unpack(">II", png[16:24]) == (256, 144)
    assert png[24:26] == b"\x08\x02" and png[37:41] == b"sRGB"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("studio", type=Path, help="pinned Studio checkout with npm dependencies")
    parser.add_argument("adapter", type=lambda path: Path(path).resolve())
    parser.add_argument("--capture", type=Path, help="historical Living IR assembly capture")
    args = parser.parse_args()
    studio = args.studio.resolve()
    queue_cli = studio / "apps/standalone/tools/render-queue.mjs"
    native = args.adapter.with_name("product-studio")
    command = ROOT / "examples/cad/studio-still-command.json"

    with tempfile.TemporaryDirectory() as tmp:
        scratch = Path(tmp)
        project = scratch / "project.sqlite"
        queue = scratch / "queue"
        run(native, "init-mock", project)
        edit = studio / "fixtures/s1/apply.json"
        presentation = read(edit)["presentation_id"]
        opts = {"presentation_id": presentation, "revision": 1,
                "time": {"ticks": "1", "scale": 50}, "height": 144}
        options = write(scratch / "still.json", opts)

        def adapter(action, *rest, code=0):
            return json.loads(run(args.adapter, action, studio, project, queue, *rest, code=code))

        def status(*rest):
            return json.loads(run("node", queue_cli, "--root", queue, *rest))

        prepared = adapter("prepare", options, command)["result"]
        assert "receipt" not in prepared and status("list") == []
        task = prepared["task"]
        assert task["time"] == opts["time"] and task["revision"] == "1"
        assert task["estimated_bytes"] > 0 and task["policy"] == "historical_pinned"
        invalid = bundle(scratch / "invalid.json", (
            "use studio\nuse render\nrender.still(studio.presentation(), 0, render.options())\n"
        ))
        assert adapter("submit", REQUEST, options, invalid, code=2)["error"]["code"] == "RenderCommandRejected"
        assert status("list") == []

        admitted = adapter("submit", REQUEST, options, command)["result"]
        receipt = admitted["receipt"]
        assert admitted["task"] == task
        assert receipt["status"] == "queued" and receipt["frames"] == 1
        assert adapter("submit", REQUEST, options, command)["result"]["receipt"]["id"] == receipt["id"]
        run(native, "apply", project, edit)
        current = write(scratch / "new.json", {**opts, "revision": 2})
        conflict = adapter("submit", REQUEST, current, command, code=2)["error"]
        assert conflict["diagnostic"]["code"] == "IDEMPOTENCY_CONFLICT"
        assert adapter("prepare", options, command)["result"]["task"] == task

        run("node", queue_cli, "--root", queue, "run")
        delivered = status("status", receipt["id"])
        verify_master(delivered, task)
        assert len(status("list")) == 1

        if args.capture:
            project = scratch / "imported.sqlite"
            queue = scratch / "cad-queue"
            imported = json.loads(run(native, "import-living-ir", project, args.capture.resolve()))
            source = imported["cad_source"]
            assert source["adapter_id"] == "living-ir-cad-viewer"
            options = write(scratch / "imported-still.json", {
                **opts, "presentation_id": imported["id"], "revision": int(imported["revision"]),
            })
            pinned = adapter("submit", REQUEST, options, command)["result"]
            assert pinned["capture"]["cad_document_id"] == source["document_id"]
            assert pinned["capture"]["cad_revision"] == source["cad_revision"]
            assert pinned["capture"]["cad_geometry"] == "preview"
            assert pinned["task"]["source_snapshot_digest"] == source["snapshot_digest"]
            run("node", queue_cli, "--root", queue, "run")
            verify_master(status("status", pinned["receipt"]["id"]), pinned["task"])
            print("PASS Studio still: imported historical Core assembly capture")
    print("PASS Studio still: typed command, exact time, idempotency, historical revision and PNG master")


if __name__ == "__main__":
    main()
