"""Compose the pinned Core F-Rep and Living Voxel sphere sampler."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("core_checkout", type=Path)
    parser.add_argument("voxel_checkout", type=Path)
    parser.add_argument("destination", type=Path, help="new directory for the isolated build")
    args = parser.parse_args()
    providers = json.loads((ROOT / "luma-cad.lock.json").read_text(encoding="utf-8"))["providers"]
    pins = {name: providers[name]["commit"] for name in ("core", "voxel")}
    checkouts = {"core": args.core_checkout.resolve(), "voxel": args.voxel_checkout.resolve()}
    for name, checkout in checkouts.items():
        head = subprocess.check_output(["git", "-C", checkout, "rev-parse", "HEAD"], text=True).strip()
        if head != pins[name]:
            parser.error(f"{checkout} must be at {pins[name]}")
    destination = args.destination.resolve()
    destination.mkdir()
    for name, checkout in checkouts.items():
        target = destination / name
        subprocess.run(["git", "clone", "--quiet", "--no-checkout", checkout, target], check=True)
        subprocess.run(["git", "-C", target, "checkout", "--quiet", "--detach", pins[name]], check=True)
    subprocess.run([sys.executable, "-B", ROOT / "tools/cad/prepare-core-project.py",
                    destination / "core"], check=True)
    native = destination / "native"
    native.mkdir()
    integration = ROOT / "integrations/living-voxel-core"
    shutil.copyfile(integration / "cli.rs", native / "cli.rs")
    shutil.copyfile(ROOT / "integrations/living-voxel-core/Cargo.lock", native / "Cargo.lock")
    (native / "provider-pins.json").write_text(json.dumps(pins) + "\n", encoding="utf-8")
    (native / "build.rs").write_text(
        'fn main() { println!("cargo:rustc-env=CAD_VOXEL_TARGET={}", std::env::var("TARGET").unwrap()); }\n',
        encoding="utf-8",
    )
    (native / "Cargo.toml").write_text("""[package]
name = "luma-cad-voxel"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "luma-cad-voxel"
path = "cli.rs"

[dependencies]
living-frep = { path = "../core/crates/living-frep" }
living-symbolic = { path = "../core/crates/living-symbolic" }
voxel-core = { path = "../voxel/crates/voxel-core" }
serde = { version = "=1.0.228", features = ["derive"] }
serde_json = "=1.0.145"
tempfile = "=3.27.0"
""", encoding="utf-8")
    project = destination / "project"
    project.mkdir()
    (project / "voxel-pin.txt").write_text(pins["voxel"] + "\n", encoding="utf-8")
    for file in ("project.rs", "project_test.rs", "blocks.rs", "paged.rs", "provider.rs", "adaptive.rs"):
        shutil.copyfile(integration / file, project / file)
    shutil.copyfile(ROOT / "examples/cad/voxel-sphere.json", project / "sphere.json")
    shutil.copyfile(ROOT / "examples/cad/voxel-adaptive-sphere.json", project / "adaptive-sphere.json")
    shutil.copyfile(ROOT / "examples/cad/voxel-sample-command.json", project / "sample-command.json")
    shutil.copyfile(integration / "project.Cargo.lock", project / "Cargo.lock")
    (project / "Cargo.toml").write_text("""[package]
name = "luma-cad-voxel-project"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "luma-cad-voxel-project"
path = "project.rs"

[[test]]
name = "project"
path = "project_test.rs"

[features]
checkpoint-fault-injection = ["living-cad-viewer/checkpoint-fault-injection"]

[dependencies]
living-cad-viewer = { path = "../core/crates/living-cad-viewer", features = ["luma-cad"] }
living-voxel-pilot = { path = "../core/integrations/living-voxel-pilot" }
hir-transaction = { path = "../core/crates/hir-transaction" }
serde = "=1.0.228"
serde_json = "=1.0.145"
sha2 = "=0.10.9"
hex = "=0.4.3"
tempfile = "=3.20.0"
""" + "luma = { path = " + json.dumps(ROOT.as_posix()) + " }\n", encoding="utf-8")
    print(project / "Cargo.toml")
    print(native / "Cargo.toml")


if __name__ == "__main__":
    main()
