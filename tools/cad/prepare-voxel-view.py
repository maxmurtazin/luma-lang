"""Prepare a pinned View checkout to pack Living Voxel surface meshes."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("view_checkout", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    providers = json.loads((ROOT / "luma-cad.lock.json").read_bytes())["providers"]
    source, destination = args.view_checkout.resolve(), args.destination.resolve()
    pin = providers["view"]["commit"]
    head = subprocess.check_output(["git", "-C", source, "rev-parse", "HEAD"], text=True).strip()
    if head != pin or destination.exists():
        parser.error(f"expected View at {pin} and an unused destination")
    subprocess.run(["git", "clone", "--quiet", "--no-checkout", source, destination], check=True)
    subprocess.run(["git", "-C", destination, "checkout", "--quiet", "--detach", pin], check=True)
    adapter = destination / "crates/core-adapter/src/bin"
    manifest = adapter.parents[1] / "Cargo.toml"
    contents = manifest.read_text(encoding="utf-8")
    dependency = "serde_json.workspace = true"
    if contents.count(dependency) != 1:
        parser.error("unexpected serde_json dependency in pinned View adapter")
    manifest.write_text(contents.replace(
        dependency, 'serde_json = { workspace = true, features = ["float_roundtrip"] }'
    ), encoding="utf-8")
    shutil.copyfile(ROOT / "integrations/living-voxel-core/pack-voxel.rs", adapter / "pack-voxel.rs")
    (adapter / "voxel-pin.txt").write_text(providers["voxel"]["commit"] + "\n", encoding="utf-8")
    print(destination / "Cargo.toml")


if __name__ == "__main__":
    main()
