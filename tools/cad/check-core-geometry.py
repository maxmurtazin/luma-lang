"""Build and run the geometry fixture against the exact Core revision in the CAD lock."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
CRATES = (
    "hir-definition",
    "hir-dependency",
    "hir-domain-registry",
    "hir-transaction",
    "hir-types",
    "living-cad-features",
    "living-frep",
    "living-symbolic",
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("core_checkout", type=Path)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    core = args.core_checkout.resolve()
    pin = json.loads((ROOT / "luma-cad.lock.json").read_text(encoding="utf-8"))["providers"]["core"]
    expected = pin["commit"]

    def git(*arguments):
        return subprocess.check_output(["git", "-C", core, *arguments], text=True).strip()

    if git("rev-parse", "HEAD") != expected or git("status", "--porcelain"):
        parser.error(f"expected a clean Core checkout at {expected}")

    with tempfile.TemporaryDirectory(prefix="luma-core-geometry-") as directory:
        path = Path(directory)
        dependencies = {"luma": ROOT, **{name: core / "crates" / name for name in CRATES}}
        manifest = [
            '[package]', 'name = "luma-core-geometry-fixture"', 'version = "0.0.0"', 'edition = "2021"',
            '[[bin]]', 'name = "fixture"', 'path = "fixture.rs"',
            '[dependencies]', 'serde_json = "1"',
        ]
        manifest.extend(
            f'{name} = {{ path = {json.dumps(location.as_posix(), ensure_ascii=False)} }}'
            for name, location in dependencies.items()
        )
        (path / "Cargo.toml").write_text("\n".join(manifest) + "\n", encoding="utf-8")
        shutil.copyfile(ROOT / "tools/cad/core-geometry-fixture.rs", path / "fixture.rs")
        shutil.copyfile(core / "Cargo.lock", path / "Cargo.lock")
        command = ["cargo", "run", "--manifest-path", path / "Cargo.toml"]
        if args.offline:
            command.append("--offline")
        command.extend(["--", ROOT / "examples/cad/plate-geometry.json"])
        subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
