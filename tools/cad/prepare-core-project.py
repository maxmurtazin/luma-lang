"""Compose the Luma project adapter in a dedicated checkout at the CAD Core pin."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
INTEGRATION = ROOT / "integrations/living-ir-core"
LOCK = ROOT / "luma-cad.lock.json"


def git(checkout, *arguments):
    return subprocess.check_output(["git", "-C", checkout, *arguments], text=True).strip()


def validate_checkout(checkout, expected):
    if git(checkout, "rev-parse", "HEAD") != expected or git(checkout, "status", "--porcelain"):
        raise ValueError(f"expected a clean provider checkout at {expected}")


def replace(text, before, after):
    if text.count(before) != 1:
        raise ValueError(f"unexpected provider source: {before[:55]}")
    return text.replace(before, after, 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("core_checkout", type=Path)
    parser.add_argument("--view", type=Path, help="pinned View checkout with web assets already built")
    args = parser.parse_args()
    core = args.core_checkout.resolve()
    lock = json.loads(LOCK.read_bytes())
    validate_checkout(core, lock["providers"]["core"]["commit"])
    if args.view:
        view = args.view.resolve()
        validate_checkout(view, lock["providers"]["view"]["commit"])
        subprocess.run(["node", view / "scripts/prepare-core-v2.mjs", core], check=True)

    crate = core / "crates/living-cad-viewer"
    cargo = crate / "Cargo.toml"
    project = crate / "src/checkpoint/project.rs"
    editor = crate / "src/editor.rs"
    patches = {
        cargo: replace(cargo.read_text(), "[dependencies]",
                       "[dependencies]\nluma = { path = "
                       + json.dumps(ROOT.as_posix(), ensure_ascii=False)
                       + ", optional = true }")
                       + '\nluma-cad = ["trusted-checkpoint", "dep:luma"]\n'
                         '\n[[bin]]\nname = "luma-cad-project"\n'
                         'path = "src/bin/luma-cad-project.rs"\nrequired-features = ["luma-cad"]\n',
        editor: editor.read_text().replace("fn set_root(", "pub(crate) fn set_root(", 1)
                                  .replace("fn manifest(", "pub(crate) fn manifest(", 1),
    }
    text = project.read_text()
    text = replace(text, "use super::{sqlite", '#[cfg(feature = "luma-cad")]\npub mod luma_cad;\n\nuse super::{sqlite')
    text = replace(text, "    parts: BTreeMap<String, PartDocument>,",
                   '''    parts: BTreeMap<String, PartDocument>,
    #[cfg(feature = "luma-cad")]
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    luma_recipes: BTreeMap<String, luma_cad::RecipeRecord>,
    #[cfg(feature = "luma-cad")]
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    luma_commands: BTreeMap<String, BTreeMap<String, String>>,''')
    text = replace(text, "            parts: BTreeMap::new(),",
                   '''            parts: BTreeMap::new(),
            #[cfg(feature = "luma-cad")]
            luma_recipes: BTreeMap::new(),
            #[cfg(feature = "luma-cad")]
            luma_commands: BTreeMap::new(),''')
    guard = '''                #[cfg(feature = "luma-cad")]
                if registry.luma_recipes.contains_key(&command.document_id) {
                    return Err(error("ManagedEntityConflict", "Edit the accepted recipe or submit a Luma override command"));
                }
'''
    text = replace(text, "                let cancel = Cancellation::from_shared_flag(context.cancellation_flag());\n                let revision = match &command.action",
                   guard + "                let cancel = Cancellation::from_shared_flag(context.cancellation_flag());\n                let revision = match &command.action")
    text = replace(text, "            |_| {},\n            context,",
                   """            |stage| {
                #[cfg(all(feature = "luma-cad", feature = "checkpoint-fault-injection"))]
                if std::env::var("LUMA_CAD_TEST_CRASH_AT").as_deref() == Ok(stage) {
                    std::process::exit(73);
                }
                let _ = stage;
            },
            context,""")
    patches[project] = text

    adapter = (INTEGRATION / "project.rs").read_text()
    cli = (INTEGRATION / "cli.rs").read_text()
    if not args.view:
        adapter = adapter.replace('feature = "living-view-v2"', "any()")
        cli = cli.replace('feature = "living-view-v2"', "any()")
    fingerprint = hashlib.sha256(LOCK.read_bytes() + adapter.encode())
    for path in sorted([*(ROOT / "src").glob("*.rs"), *(ROOT / "crates").glob("*/src/*.rs"),
                        ROOT / "Cargo.lock", Path(__file__).resolve()]):
        fingerprint.update(hashlib.sha256(path.read_bytes()).digest())
    module = crate / "src/checkpoint/project/luma_cad.rs"
    patches[module] = replace(adapter, "__LUMA_CAD_INTEGRATION_PIN__", f"sha256:{fingerprint.hexdigest()}")
    patches[crate / "src/bin/luma-cad-project.rs"] = cli
    for name in ("plate-geometry.json", "plate-width-command.json"):
        patches[module.parent / name] = (ROOT / "examples/cad" / name).read_text()
    for path, content in patches.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


if __name__ == "__main__":
    main()
