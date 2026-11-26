"""Register the CAD lock's B-rep publication producer in a pinned View checkout."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("view_checkout", type=Path)
    parser.add_argument("destination", type=Path, help="new View checkout for the scene packer")
    args = parser.parse_args()
    providers = json.loads((ROOT / "luma-cad.lock.json").read_text(encoding="utf-8"))["providers"]
    view = providers["view"]["commit"]
    brep = providers["brep"]["commit"]
    source, work = args.view_checkout.resolve(), args.destination.resolve()
    if subprocess.check_output(["git", "-C", source, "rev-parse", "HEAD"], text=True).strip() != view:
        parser.error(f"expected View checkout at {view}")
    if work.exists() or work.is_symlink():
        parser.error("destination must be unused")
    subprocess.run(["git", "clone", "--quiet", "--no-checkout", source, work], check=True)
    subprocess.run(["git", "-C", work, "checkout", "--quiet", "--detach", view], check=True)
    adapter = work / "crates/core-adapter/src/publication.rs"
    text = adapter.read_text(encoding="utf-8")
    before = "mesh.source.provider_revision != BREP_PROVIDER_REVISION"
    if text.count(before) != 1:
        parser.error("unexpected publication provider check in the pinned View source")
    adapter.write_text(text.replace(before, f'!matches!(mesh.source.provider_revision.as_str(), BREP_PROVIDER_REVISION | "{brep}")'),
                       encoding="utf-8")
    print(work / "Cargo.toml")


if __name__ == "__main__":
    main()
