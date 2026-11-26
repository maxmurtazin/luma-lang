"""Assemble the CAD-2 native CLI from the pinned Core and B-rep checkouts."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def git(checkout, *args):
    return subprocess.check_output(["git", "-C", checkout, *args], text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("core_checkout", type=Path)
    parser.add_argument("brep_checkout", type=Path)
    parser.add_argument("destination", type=Path, help="new directory for the isolated build")
    args = parser.parse_args()
    lock = json.loads((ROOT / "luma-cad.lock.json").read_text(encoding="utf-8"))["providers"]
    providers = [
        (args.core_checkout.resolve(), "living-ir-core", lock["core"]["commit"]),
        (args.brep_checkout.resolve(), "living-brep-core", lock["brep"]["commit"]),
    ]
    for checkout, _, commit in providers:
        if git(checkout, "rev-parse", "HEAD") != commit:
            parser.error(f"{checkout} must be at {commit}")

    directory = args.destination.resolve()
    directory.mkdir()
    for source, name, commit in providers:
        checkout = directory / name
        subprocess.run(["git", "clone", "--quiet", "--no-checkout",
                        source, checkout], check=True)
        git(checkout, "checkout", "--quiet", "--detach", commit)

    integration = ROOT / "integrations/living-brep-core"
    project = directory / "living-ir-core/crates/living-cad-viewer/src/checkpoint/project.rs"
    text = project.read_text(encoding="utf-8")
    anchor = "    /// Resolve a supported authored extrusion without holding the writer lock.\n"
    if text.count(anchor) != 1:
        parser.error("unexpected ProjectStore source in the pinned Core checkout")
    method = (integration / "project-drawing.rs").read_text(encoding="utf-8")
    project.write_text(text.replace(anchor, method + "\n" + anchor), encoding="utf-8")

    bridge = directory / "native"
    bridge.mkdir()
    (bridge / "Cargo.toml").write_text("""[package]
name = "luma-cad-brep"
version = "0.0.0"
edition = "2021"

[[bin]]
name = "luma-cad-brep"
path = "cli.rs"

[[test]]
name = "project"
path = "cli_test.rs"

[dependencies]
brep-core = { path = "../living-brep-core/crates/brep-core" }
hir-evaluation-api = { path = "../living-ir-core/crates/hir-evaluation-api" }
hir-transaction = { path = "../living-ir-core/crates/hir-transaction" }
hir-types = { path = "../living-ir-core/crates/hir-types" }
living-cad-features = { path = "../living-ir-core/crates/living-cad-features" }
living-sketch = { path = "../living-ir-core/crates/living-sketch" }
living-cad-viewer = { path = "../living-ir-core/crates/living-cad-viewer", features = ["trusted-checkpoint"] }
living-ir-brep-preview = { path = "../living-brep-core/integrations/living-ir-preview", features = ["project"] }
serde_json = "=1.0.145"
sha2 = "=0.10.9"
tempfile = "=3.27.0"
""", encoding="utf-8")
    for filename in ("cli.rs", "cli_test.rs", "profile.rs", "artifact.rs", "drawing.rs"):
        shutil.copyfile(integration / filename, bridge / filename)
    shutil.copyfile(directory / "living-brep-core/integrations/living-ir-preview/Cargo.lock",
                    bridge / "Cargo.lock")
    (bridge / "brep-pin.txt").write_text(lock["brep"]["commit"] + "\n", encoding="utf-8")
    print(bridge / "Cargo.toml")


if __name__ == "__main__":
    main()
