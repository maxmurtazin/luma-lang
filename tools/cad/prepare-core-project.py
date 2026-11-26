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


def compose_job_commit(project, sqlite):
    project = replace(project, "    checkpoint: [u8; 32],\n}",
                      """    checkpoint: [u8; 32],
    #[cfg(feature = "luma-cad")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub luma_jobs: Vec<luma_cad_jobs::Delivery>,
    #[cfg(feature = "luma-cad")]
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub luma_dependent_jobs: BTreeMap<String, Vec<luma_cad_jobs::Delivery>>,
}""")
    project = replace(project, """    ) -> Result<Receipt, Error> {
        uuid(document)?;
        if request.len() != 32""",
        """    ) -> Result<Receipt, Error> {
        self.commit_with((document, request, expected), command, context, apply, |_, _| Ok(()))
    }

    fn commit_with(
        &mut self,
        (document, request, expected): (&str, &str, u64),
        command: &impl Serialize,
        context: &PreparationContext,
        apply: impl FnOnce(&mut Registry) -> Result<u64, Error>,
        stage: impl FnOnce(&mut rusqlite::Transaction<'_>, &mut Receipt) -> Result<(), Error>,
    ) -> Result<Receipt, Error> {
        uuid(document)?;
        if request.len() != 32""")
    project = replace(project, "        let receipt = Receipt {", "        let mut receipt = Receipt {")
    project = replace(project, """            checkpoint: Sha256::digest(&after).into(),
        };
        let bytes = seal(&self.key, domain.as_bytes(), &receipt)?;""",
        """            checkpoint: Sha256::digest(&after).into(),
            #[cfg(feature = "luma-cad")]
            luma_jobs: Vec::new(),
            #[cfg(feature = "luma-cad")]
            luma_dependent_jobs: BTreeMap::new(),
        };
        let key = &self.key;""")
    project = replace(project, """        self.db.commit_command(
            super::CommandActor::Owner,""", """        self.db.commit_command_with(
            super::CommandActor::Owner,""")
    project = replace(project, """            &after,
            &bytes,
            |stage| {""",
        """            &after,
            |tx| {
                stage(tx, &mut receipt)?;
                seal(key, domain.as_bytes(), &receipt)
            },
            |stage| {""")
    sqlite = replace(sqlite, """    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_command(""",
        """    #[cfg(feature = "luma-cad")]
    pub(super) fn luma_connection(&self) -> &Connection {
        &self.0
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_command(""")
    sqlite = replace(sqlite, """    ) -> Result<(), Error> {
        fault("command-before-begin");
        let tx = self""",
        """    ) -> Result<(), Error> {
        self.commit_command_with(actor, document, request, before, after,
            |_| Ok(receipt.to_vec()), fault, context)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_command_with(
        &mut self,
        actor: super::CommandActor<'_>,
        document: &str,
        request: &str,
        before: &[u8],
        after: &[u8],
        make_receipt: impl FnOnce(&mut rusqlite::Transaction<'_>) -> Result<Vec<u8>, Error>,
        fault: impl Fn(&str),
        context: &hir_transaction::PreparationContext,
    ) -> Result<(), Error> {
        fault("command-before-begin");
        let mut tx = self""")
    sqlite = replace(sqlite, '        fault("command-before-receipt");',
                     '''        let receipt = make_receipt(&mut tx)?;
        fault("command-after-luma-outbox");
        fault("command-before-receipt");''')
    sqlite = replace(sqlite, "&Sha256::digest(receipt)[..]", "&Sha256::digest(&receipt)[..]")
    sqlite = replace(sqlite, '        if Self::schema(&db)? != Self::schema(&expected)? {',
        """        let actual = {
            let observed = db.unchecked_transaction().map_err(failure)?;
            #[cfg(feature = "luma-cad")]
            if multi_part && luma_cad_jobs::schema_present(&observed).map_err(outbox_schema_error)? {
                let fixture = expected.unchecked_transaction().map_err(failure)?;
                luma_cad_jobs::install_in(&fixture).map_err(outbox_schema_error)?;
                fixture.commit().map_err(failure)?;
            }
            let schema = Self::schema(&observed)?;
            observed.commit().map_err(failure)?;
            schema
        };
        if actual != Self::schema(&expected)? {""")
    sqlite = replace(sqlite, 'pub(super) struct Database(Connection);',
        """#[cfg(feature = "luma-cad")]
fn outbox_schema_error(_: luma_cad_jobs::Error) -> Error {
    error("UnsupportedCheckpointSchema", "Invalid CAD outbox schema ledger")
}

pub(super) struct Database(Connection);""")
    return project, sqlite


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
    sqlite = crate / "src/checkpoint/sqlite.rs"
    patches = {
        cargo: replace(cargo.read_text(), "[dependencies]",
                       "[dependencies]\nluma = { path = "
                       + json.dumps(ROOT.as_posix(), ensure_ascii=False)
                       + ", optional = true }"
                       + "\nluma-cad-jobs = { path = "
                       + json.dumps((ROOT / "crates/luma-cad-jobs").as_posix(), ensure_ascii=False)
                       + ", optional = true }")
                       + '\nluma-cad = ["trusted-checkpoint", "dep:luma", "dep:luma-cad-jobs"]\n'
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
                if registry.luma_recipes.get(&command.document_id).is_some_and(|record| record.is_managed()) {
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
    # Core bounds a checkpoint to 129 authenticated snapshots. Account for
    # their validation separately from the operation being prepared.
    text = replace(text,
        "    let mut store = facade.restore_checkpoint_history(&part.snapshots, context)?;",
        """    #[cfg(feature = "luma-cad")]
    let replay = context.for_checkpoint_replay(part.snapshots.len().min(129));
    #[cfg(feature = "luma-cad")]
    let context = &replay;
    let mut store = facade.restore_checkpoint_history(&part.snapshots, context)?;""")
    overlay = core / "crates/hir-transaction/src/overlay.rs"
    patches[overlay] = replace(overlay.read_text(),
        "    pub fn with_observer(mut self, observer: Observer) -> Self {",
        """    /// Account for retained checkpoint replay without charging the incoming operation.
    /// Keep its deadline, cancellation, and per-definition size limits.
    pub fn for_checkpoint_replay(&self, retained: usize) -> Self {
        let mut limits = self.limits;
        limits.max_work = limits.max_work
            .min(PreparationLimits::default().max_work)
            .saturating_mul(retained);
        Self {
            limits,
            flag: self.flag.clone(),
            work: AtomicUsize::new(0),
            started: self.started,
            observer: self.observer.clone(),
        }
    }

    pub fn with_observer(mut self, observer: Observer) -> Self {""")
    patches[project], patches[sqlite] = compose_job_commit(text, sqlite.read_text())

    adapter = (INTEGRATION / "project.rs").read_text()
    cli = (INTEGRATION / "cli.rs").read_text()
    if not args.view:
        adapter = adapter.replace('feature = "living-view-v2"', "any()")
        cli = cli.replace('feature = "living-view-v2"', "any()")
    fingerprint = hashlib.sha256(LOCK.read_bytes() + adapter.encode())
    for path in sorted([*(ROOT / "src").glob("*.rs"), *(ROOT / "crates").glob("*/src/*.rs"),
                        ROOT / "Cargo.lock", ROOT / "crates/luma-cad-jobs/Cargo.lock",
                        ROOT / "crates/luma-cad-jobs/src/queue.sql", Path(__file__).resolve()]):
        fingerprint.update(hashlib.sha256(path.read_bytes()).digest())
    module = crate / "src/checkpoint/project/luma_cad.rs"
    for name in ("jobs.rs", "bindings.rs"):
        content = (INTEGRATION / name).read_text()
        patches[module.parent / "luma_cad" / name] = content
        fingerprint.update(hashlib.sha256(content.encode()).digest())
    patches[module] = replace(adapter, "__LUMA_CAD_INTEGRATION_PIN__", f"sha256:{fingerprint.hexdigest()}")
    patches[crate / "src/bin/luma-cad-project.rs"] = cli
    for name in ("plate-geometry.json", "plate-width-command.json"):
        patches[module.parent / name] = (ROOT / "examples/cad" / name).read_text()
    for path, content in patches.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


if __name__ == "__main__":
    main()
