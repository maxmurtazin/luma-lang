"""Check deterministic CAD candidate evidence and fail closed on a changed manifest."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


TOOL = Path(__file__).with_name("release-manifest.py")
command = [sys.executable, "-B", str(TOOL)]


def run(*args, code=0):
    result = subprocess.run([*command, *args], capture_output=True, check=False)
    assert result.returncode == code, (result.stdout, result.stderr)
    return result.stdout


def main():
    raw = run()
    manifest = json.loads(raw)
    commit = manifest["commit"]
    assert raw == run("--revision", commit)
    assert manifest["status"] == "candidate"
    assert set(manifest["qualification"]["candidate_runtime"].values()) == {"not_run"}
    registry = json.loads(subprocess.check_output(
        ["git", "-C", TOOL.resolve().parents[2], "show", f"{commit}:{manifest['registry']['path']}"]
    ))["capabilities"]
    assert set(manifest["operations"]) == set(registry)
    for name, row in registry.items():
        operation = manifest["operations"][name]
        assert operation["recorded_target_verification"] == row["target_verification"]
        assert operation["availability"] == row["availability"]
    assert manifest["contract_inputs"][manifest["registry"]["path"]] == manifest["registry"]["sha256"]
    assert set(manifest["qualification"]["evidence"]) == {f"CAD-{i}" for i in range(7)}

    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "candidate.json"
        path.write_bytes(raw)
        run("--revision", commit, "--verify", path)

        manifest["qualification"]["candidate_runtime"]["t2"] = "passed"
        path.write_text(json.dumps(manifest), encoding="utf-8")
        run("--revision", commit, "--verify", path, code=1)

        path.write_bytes(b'{"status":"candidate",' + raw[1:])
        run("--revision", commit, "--verify", path, code=1)

    print("PASS CAD release candidate: pinned evidence, reproducible bytes and unchanged target status")


if __name__ == "__main__":
    main()
