"""Compose the Luma catalog reader against the Elements checkout in the CAD lock."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elements", type=Path, help="checkout at the CAD lock revision")
    parser.add_argument("destination", type=Path, help="new directory for the composed checkout")
    args = parser.parse_args()
    pin = json.loads((ROOT / "luma-cad.lock.json").read_text())["providers"]["elements"]["commit"]
    head = subprocess.check_output(
        ["git", "-C", args.elements, "rev-parse", "HEAD"], text=True
    ).strip()
    if head != pin:
        parser.error(f"expected Elements at {pin}")
    destination = args.destination.resolve()
    destination.mkdir()
    subprocess.run(["git", "clone", "--quiet", "--no-checkout", args.elements.resolve(), destination], check=True)
    subprocess.run(["git", "-C", destination, "checkout", "--quiet", "--detach", pin], check=True)
    manifest = destination / "Cargo.toml"
    with manifest.open("a", encoding="utf-8") as output:
        output.write(
            '\n[[bin]]\nname = "luma-cad-elements"\npath = "luma_catalog.rs"\n'
            f'\n[dependencies.luma]\npath = {json.dumps(str(ROOT))}\n'
            f'\n[dependencies.luma-cad-contract]\npath = {json.dumps(str(ROOT / "crates/luma-cad-contract"))}\n'
        )
    shutil.copyfile(ROOT / "integrations/living-ir-elements/catalog.rs", destination / "luma_catalog.rs")
    (destination / "luma-elements-pin.txt").write_text(pin + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
