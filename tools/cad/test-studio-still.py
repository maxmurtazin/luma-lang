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
    if task["profile"] == "final-pathtrace":
        assert master["format"] == "studio-pathtrace-master/1"
        assert master["blender"]["engine"] == "CYCLES" and master["blender"]["device"] == "CPU"
        for key in ("samples", "threads"):
            assert master["blender"][key] == task["cycles"][key]
    else:
        assert master["format"] == "studio-raster-master/1" and master["chromium"]
    assert master["mode"] == "still"
    for field in ("presentation_id", "revision", "source_snapshot_digest", "inputs", "width", "height"):
        assert master[field] == task[field], field
    assert master["png"] == "RGB8/sRGB/full"
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
    parser.add_argument("--extended", action="store_true", help="also exercise Cycles CPU and FFmpeg")
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

        if args.extended:
            initial = read(edit)
            fps = {"numerator": 24000, "denominator": 1001}
            timeline = write(scratch / "timeline.json", {
                **initial, "command_id": "70000000-0000-4000-8000-000000000097",
                "expected_revision": "2", "idempotency_key": "luma-cad6-video-test",
                "operations": [{"type": "SetTimeline", "timeline": {
                    "frame_count": 3, "fps": fps,
                    "active_camera_id": "50000000-0000-4000-8000-000000000001",
                }}],
            })
            run(native, "apply", project, timeline)
            video = ROOT / "examples/cad/studio-video-command.json"
            video_options = write(scratch / "video.json", {
                "presentation_id": presentation, "revision": 3, "height": 144,
            })
            video_key = REQUEST[:-1] + "2"
            plan = adapter("prepare", video_options, video)["result"]["task"]
            admitted = adapter("submit", video_key, video_options, video)["result"]
            assert plan == admitted["task"]
            assert plan["operation"] == "render.video"
            assert plan["frames"] == 3 and plan["fps"] == fps and plan["outputs"] == ["png", "mp4"]
            job = admitted["receipt"]["id"]
            run("node", queue_cli, "--root", queue, "run")
            delivered = status("status", job)
            assert delivered["status"] == "succeeded", delivered
            output = Path(delivered["output"])
            master_file = output / "master.json"
            master = read(master_file)
            assert master["inputs"] == plan["inputs"] and master["fps"] == fps
            assert master["revision"] == "3" and master["mode"] == "sequence"
            for index, frame in enumerate(master["frames"]):
                time = frame["exact_time"]
                assert int(time["ticks"]) * 24000 == index * 1001 * time["scale"]
                assert hashlib.sha256((output / frame["path"]).read_bytes()).hexdigest() == frame["digest"].removeprefix("sha256:")
            validation = read(output / "video/validation.json")
            stream = validation["stream"]
            assert validation["decoded_frames"] == 3
            assert stream["codec_name"] == "h264" and stream["color_space"] == "bt709"
            assert stream["time_base"] == "1/24000" and stream["duration_ts"] == 3003
            video_path = output / "video/studio.mp4"
            media = json.loads(run("ffprobe", "-v", "error", "-show_entries",
                                   "stream=codec_type:frame=pts", "-of", "json", video_path))
            assert media["streams"] == [{"codec_type": "video"}]
            assert [frame["pts"] for frame in media["frames"]] == [0, 1001, 2002]

            reencoded = status("reencode", job, "--crf", "23")
            recovered = Path(reencoded["output"])
            assert (recovered / "master.json").read_bytes() == master_file.read_bytes()
            assert (recovered / master["frames"][0]["path"]).samefile(output / master["frames"][0]["path"])
            assert read(recovered / "video/validation.json")["crf"] == 23

            project = scratch / "pathtrace.sqlite"
            queue = scratch / "cycles-queue"
            run(native, "init-mock", project)
            for path in ("s1/apply.json", "s3/look.json", "s4/dolly.json", "s5/final-orbit-light.json"):
                run(native, "apply", project, studio / "fixtures" / path)
            cycles = write(scratch / "cycles.json", {
                **opts, "revision": 5, "profile": "final-pathtrace", "samples": 2, "threads": 2,
            })
            still = adapter("submit", REQUEST, cycles, command)["result"]
            run("node", queue_cli, "--root", queue, "run")
            verify_master(status("status", still["receipt"]["id"]), still["task"])
            print("PASS Studio render: Cycles CPU, rational MP4 and PNG master reencode")

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
            if source["adapter_contract"] == "living-ir-capture/1":
                final_options = write(scratch / "legacy-final.json", {
                    **read(options), "profile": "final-pathtrace",
                })
                rejected = adapter("submit", REQUEST[:-1] + "2", final_options, command, code=2)
                assert rejected["error"]["diagnostic"]["code"] == "CAD_FINAL_MESH_UNAVAILABLE"
                assert len(status("list")) == 1
            run("node", queue_cli, "--root", queue, "run")
            verify_master(status("status", pinned["receipt"]["id"]), pinned["task"])
            print("PASS Studio still: imported historical Core assembly capture")
    print("PASS Studio still: typed command, exact time, idempotency, historical revision and PNG master")


if __name__ == "__main__":
    main()
