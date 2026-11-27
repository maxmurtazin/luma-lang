"""Compose the pinned provider's transient evaluator with the Core CAS host."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
INTEGRATION = ROOT / "integrations/living-ir-2d"


def replace(text, before, after):
    if text.count(before) != 1:
        raise ValueError(f"unexpected pinned provider source near {before!r}")
    return text.replace(before, after, 1)


def prepare_provider(source, destination, expected):
    pin = subprocess.check_output(
        ["git", "-C", source, "rev-parse", "HEAD"], text=True
    ).strip()
    if pin != expected:
        raise ValueError(f"expected Living IR 2D checkout at {expected}")
    subprocess.run(["git", "clone", "--quiet", "--no-checkout", source, destination], check=True)
    subprocess.run(["git", "-C", destination, "checkout", "--quiet", "--detach", pin], check=True)
    runtime = destination / "crates/lir2d-runtime/src/lib.rs"
    text = replace(runtime.read_text(encoding="utf-8"), "impl Runtime {\n",
                   "mod luma_embedded;\n\nimpl Runtime {\n")
    text = replace(text, "        snapshot.validate()?;\n        if snapshot.parameters()",
                   """        Self::new_local(project_id, snapshot, sources)
    }

    fn new_local(
        project_id: ObjectId,
        snapshot: DocumentSnapshot,
        sources: FixtureSources,
    ) -> Result<Self, ErrorEnvelope> {
        snapshot.validate()?;
        if snapshot.parameters()""")
    runtime.write_text(text, encoding="utf-8")
    runtime.with_name("luma_embedded.rs").write_bytes((INTEGRATION / "prepare.rs").read_bytes())
    # Core enables serde_json/arbitrary_precision. Numbers buffered by an
    # untagged enum must be decoded through Number before conversion to SI f64.
    scalars = destination / "crates/lir2d-types/src/scalars.rs"
    text = replace(scalars.read_text(encoding="utf-8"),
                   "        Self::try_from(f64::deserialize(deserializer)?).map_err(de::Error::custom)",
                   """        let number = serde_json::Number::deserialize(deserializer)?;
        let value = number.as_f64().ok_or_else(|| de::Error::custom("expected a finite number"))?;
        Self::try_from(value).map_err(de::Error::custom)""")
    scalars.write_text(text, encoding="utf-8")



def compose_core(crate, provider, cargo, project):
    dependencies = "\n".join(
        f"{name} = {{ path = {json.dumps((provider / 'crates' / name).as_posix())}, optional = true }}"
        for name in ("lir2d-runtime", "lir2d-types")
    )
    cargo = replace(cargo, "[dependencies]", f"[dependencies]\n{dependencies}")
    cargo = replace(cargo, "[features]", """[features]
luma-embedded-2d = ["luma-cad", "dep:lir2d-runtime", "dep:lir2d-types"]""")
    project = replace(project, "use super::{sqlite", """#[cfg(feature = "luma-embedded-2d")]
mod luma_embedded_2d;

use super::{sqlite""")
    project = replace(project, "    assemblies: BTreeMap<String, String>,", """    assemblies: BTreeMap<String, String>,
    #[cfg(feature = "luma-embedded-2d")]
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    embedded_2d: BTreeMap<String, luma_embedded_2d::Embedded2dV1>,""")
    project = replace(project, "            assemblies: BTreeMap::new(),", """            assemblies: BTreeMap::new(),
            #[cfg(feature = "luma-embedded-2d")]
            embedded_2d: BTreeMap::new(),""")
    project = replace(project,
                      "        let document_revision = apply(&mut registry)?;\n        if previous != Some(document_revision) {",
                      """        #[cfg(feature = "luma-embedded-2d")]
        let previous_2d = registry.embedded_2d.get(document).map(|state| state.revision);
        let document_revision = apply(&mut registry)?;
        let changed = previous != Some(document_revision);
        #[cfg(feature = "luma-embedded-2d")]
        let changed = changed || previous_2d != registry.embedded_2d.get(document).map(|state| state.revision);
        if changed {""")
    sketch = crate / "src/sketch.rs"
    sketch_text = replace(sketch.read_text(encoding="utf-8"), "fn write(document: &mut Value, sketch: &Sketch)",
                          "pub(crate) fn write(document: &mut Value, sketch: &Sketch)")
    module = crate / "src/checkpoint/project/luma_embedded_2d.rs"
    return {crate / "Cargo.toml": cargo, crate / "src/checkpoint/project.rs": project, sketch: sketch_text,
            module: (INTEGRATION / "project.rs").read_text(encoding="utf-8"),
            module.with_suffix("") / "branch.rs": (INTEGRATION / "branch.rs").read_text(encoding="utf-8")}


def compose_cli(cli, enabled):
    match = '    match (action.as_str(), args) {\n'
    if enabled:
        arms = (INTEGRATION / "cli.rs").read_text(encoding="utf-8")
    else:
        arms = """        ("2d-register" | "2d-apply" | "2d-inspect" | "2d-solve-report" | "2d-luma"
          | "2d-propose-branch" | "2d-accept-branch", _) =>
            Err(error("UnsupportedEmbedded2d", "Core host was built without the registered 2D adapter")),
"""
    return replace(cli, match, match + arms)
