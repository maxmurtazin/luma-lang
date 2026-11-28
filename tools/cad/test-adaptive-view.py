"""Verify the adaptive View bundle and write its checked scene for Chromium."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pack", type=Path)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    tree = args.evidence / "adaptive-job.lvxadap"
    report_bytes = (args.evidence / "adaptive-report.json").read_bytes()
    report = json.loads(report_bytes)["result"]
    packed = subprocess.check_output([args.pack, "adaptive", tree], input=report_bytes)
    output = json.loads(packed)
    source, = (occurrence["source"] for occurrence in output["bundle"]["scene"]["occurrences"].values())
    evidence = output["evidence"]
    digest = hashlib.sha256(tree.read_bytes()).hexdigest()
    assert source["kind"] == "external_asset"
    assert source["asset_id"] == evidence["lvxadap_sha256"] == digest
    assert evidence["tree_digest"] == report["tree_digest"]
    assert evidence["part_source"] == report["part_source"]
    assert source["content_digest"] == evidence["provider_report_sha256"]
    assert source["content_digest"] == hashlib.sha256(report_bytes).hexdigest()
    assert evidence["provider_surface_report"] == report["surface"]["report"]
    assert evidence["view_mesh_accuracy"] == "Unavailable"

    with tempfile.TemporaryDirectory() as directory:
        corrupt = Path(directory) / tree.name
        data = bytearray(tree.read_bytes())
        data[-1] ^= 1
        corrupt.write_bytes(data)
        rejected = subprocess.run(
            [args.pack, "adaptive", corrupt], input=report_bytes, capture_output=True, check=False
        )
        assert rejected.returncode != 0 and not rejected.stdout
        assert b"LVXADAP digest" in rejected.stderr

    (args.evidence / "adaptive-view.json").write_bytes(packed)
    (args.evidence / "scene.json").write_text(json.dumps(output["bundle"]), encoding="utf-8")
    print("PASS Adaptive view retained the LVXADAP, Part pin and provider surface report")


if __name__ == "__main__":
    main()
