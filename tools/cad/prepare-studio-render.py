"""Compose the Luma still-render entry point in a checkout of pinned Product Studio."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("studio", type=Path, help="Studio checkout at the CAD lock revision")
    parser.add_argument("destination", type=Path, help="new directory for the composed checkout")
    args = parser.parse_args()
    pin = json.loads((ROOT / "luma-cad.lock.json").read_text())["providers"]["studio"]["commit"]
    head = subprocess.check_output(["git", "-C", args.studio, "rev-parse", "HEAD"], text=True).strip()
    if head != pin:
        parser.error(f"expected Studio at {pin}")
    destination = args.destination.resolve()
    destination.mkdir()
    subprocess.run(["git", "clone", "--quiet", "--no-checkout", args.studio.resolve(), destination], check=True)
    subprocess.run(["git", "-C", destination, "checkout", "--quiet", "--detach", pin], check=True)
    crate = destination / "crates/presentation-cli"
    with (crate / "Cargo.toml").open("a", encoding="utf-8") as manifest:
        manifest.write(
            '\n[[bin]]\nname = "luma-cad-studio"\npath = "luma_render.rs"\n'
            f'\n[dependencies.luma]\npath = {json.dumps(str(ROOT))}\n'
            f'\n[dependencies.luma-cad-contract]\npath = {json.dumps(str(ROOT / "crates/luma-cad-contract"))}\n'
        )
    shutil.copyfile(ROOT / "integrations/living-studio/render.rs", crate / "luma_render.rs")
    (crate / "luma-studio-pin.txt").write_text(pin + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
