"""Verify Core sphere samples and provenance after a Living Voxel LVX round trip."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]


def qualify(cli):
    def call(*args, status=0):
        proc = subprocess.run([cli, *map(str, args)], capture_output=True, text=True, check=False)
        assert proc.returncode == status, (args, proc.stdout, proc.stderr)
        return json.loads(proc.stdout)["result" if status == 0 else "error"]

    fixture = ROOT / "examples/cad/voxel-sphere.json"
    lock = json.loads((ROOT / "luma-cad.lock.json").read_bytes())
    with tempfile.TemporaryDirectory() as directory:
        output = Path(directory) / "sphere.lvx"
        report = call("sample", fixture, output)
        assert report["provider_pins"] == {key: lock["providers"][key]["commit"] for key in ("core", "voxel")}
        assert report["known_samples"] == 4913 and report["unknown_samples"] == 0
        assert report["source"]["semantic_kind"] == "SignedDistanceEstimate"
        sha = report["digests"]["blob_digest"]
        assert hashlib.sha256(output.read_bytes()).hexdigest() == sha
        for x in (0, 4, 8):
            loaded = call("inspect", output, sha, x, 0, 0)
            assert loaded["digests"] == report["digests"]
            assert loaded["source"]["source_digest"] == report["source"]["fingerprint"]
            assert loaded["backend"]["build_digest"] == hashlib.sha256(cli.read_bytes()).hexdigest()
            sample = loaded["observation"]["sample"]
            assert sample["metric_status"] == "Estimate"
            assert math.isclose(sample["value"]["value"], x / 1024 - 0.004, abs_tol=1e-15)
        assert call("inspect", output, "00" * 32, 0, 0, 0, status=2)["code"] == "CorruptArtifact"
        assert call("sample", fixture, output, status=2)["code"] == "IoFailure"
        assert hashlib.sha256(output.read_bytes()).hexdigest() == sha

        request = json.loads(fixture.read_bytes())
        request["center_m"][0] = 1 / 1024
        moved = Path(directory) / "moved.json"
        moved.write_text(json.dumps(request), encoding="utf-8")
        moved_lvx = Path(directory) / "moved.lvx"
        shifted = call("sample", moved, moved_lvx)
        assert shifted["source"]["fingerprint"] != report["source"]["fingerprint"]
        assert shifted["digests"]["logical_field_digest"] != report["digests"]["logical_field_digest"]
        loaded = call("inspect", moved_lvx, shifted["digests"]["blob_digest"], 1, 0, 0)
        assert loaded["observation"]["sample"]["value"]["value"] == -0.004

        request["radius_m"] = -0.004
        invalid = Path(directory) / "negative.json"
        invalid.write_text(json.dumps(request), encoding="utf-8")
        rejected = Path(directory) / "rejected.lvx"
        assert call("sample", invalid, rejected, status=2)["code"] == "NumericRange"
        assert not rejected.exists()
    print("PASS Core F-Rep sphere → Living Voxel → LVX round trip and exact provenance")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=lambda path: Path(path).resolve())
    qualify(parser.parse_args().cli)
