//! Filesystem transactions for a trusted workspace directory.
use super::{Failure, Preview, Proposal, Snapshot, Workspace, MAX_FILES, MAX_SOURCE_BYTES};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const LOCK: &str = ".luma-edit.lock";
const JOURNAL: &str = ".luma-edit-journal-v1";
const MAX_JOURNAL: usize = MAX_SOURCE_BYTES * 12 + 8192;

pub struct DiskWorkspace {
    root: PathBuf,
    workspace: Workspace,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    changes: BTreeMap<String, Change>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    before: String,
    after: String,
}

impl DiskWorkspace {
    /// Open an explicit list of sibling modules and recover any interrupted commit.
    pub fn open(root: &Path, entry: String, modules: Vec<String>) -> Result<Self, Failure> {
        if modules.len() > MAX_FILES || !modules.iter().all(|name| super::valid_file(name)) {
            return Err(Failure::new(
                "invalid_file",
                "expected at most 16 valid sibling module names",
            ));
        }
        let unique: BTreeSet<_> = modules.iter().collect();
        if unique.len() != modules.len() {
            return Err(Failure::new("invalid_file", "duplicate module name"));
        }
        let root = root.canonicalize().map_err(storage)?;
        let _lock = lock(&root).map_err(storage)?;
        recover(&root).map_err(recovery_failure)?;
        let mut files = BTreeMap::new();
        let mut remaining = MAX_SOURCE_BYTES;
        for module in modules {
            let source = read(&root.join(&module), remaining).map_err(storage)?;
            remaining -= source.len();
            files.insert(module, source);
        }
        let workspace = Workspace::open(entry, files)?;
        Ok(Self { root, workspace })
    }

    pub fn snapshot(&self) -> &Snapshot {
        self.workspace.snapshot()
    }

    pub fn preview(&self, proposal: &Proposal) -> Result<Preview, Failure> {
        self.workspace.preview(proposal)
    }

    pub fn commit(&mut self, proposal: &Proposal) -> Result<&Snapshot, Failure> {
        self.commit_with(proposal, |_| Ok(()))
    }

    fn commit_with(
        &mut self,
        proposal: &Proposal,
        after_replace: impl Fn(usize) -> io::Result<()>,
    ) -> Result<&Snapshot, Failure> {
        let (preview, files, generation) = self.workspace.prepare(proposal)?;
        let _lock = lock(&self.root).map_err(storage)?;
        recover(&self.root).map_err(recovery_failure)?;

        // Validate the complete read set, including unchanged dependencies.
        for (module, base) in &self.workspace.files {
            let current = read(&self.root.join(module), MAX_SOURCE_BYTES);
            if !matches!(current, Ok(ref current) if current == base) {
                return Err(Failure::new(
                    "source_conflict",
                    format!("{module} changed or cannot be read; reopen the workspace"),
                ));
            }
        }

        let mut journal = Journal {
            version: 1,
            changes: BTreeMap::new(),
        };
        let mut staged = Vec::new();
        for (module, after) in &files {
            let before = &self.workspace.files[module];
            if before != after {
                let target = self.root.join(module);
                staged.push((target.clone(), stage(&target, after).map_err(storage)?));
                journal.changes.insert(
                    module.clone(),
                    Change {
                        before: before.clone(),
                        after: after.clone(),
                    },
                );
            }
        }
        if !staged.is_empty() {
            publish(&self.root, &journal, staged, after_replace)?;
        }
        self.workspace.files = files;
        self.workspace.generation = generation;
        self.workspace.snapshot = preview.candidate;
        Ok(self.workspace.snapshot())
    }
}

fn storage(error: io::Error) -> Failure {
    Failure::new("storage_error", error.to_string())
}

fn recovery_failure(error: io::Error) -> Failure {
    Failure::new("recovery_required", error.to_string())
}

fn regular(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::other("expected a regular file"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("reparse points are unsupported"));
        }
    }
    Ok(metadata)
}

fn read(path: &Path, limit: usize) -> io::Result<String> {
    regular(path)?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("file exceeds size limit"));
    }
    String::from_utf8(bytes).map_err(io::Error::other)
}

fn lock(root: &Path) -> io::Result<File> {
    let path = root.join(LOCK);
    match regular(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

fn temp(root: &Path) -> io::Result<tempfile::NamedTempFile> {
    tempfile::Builder::new()
        .prefix(".luma-edit-stage-")
        .tempfile_in(root)
}

fn stage(target: &Path, contents: &str) -> io::Result<tempfile::NamedTempFile> {
    let mut staged = temp(target.parent().unwrap())?;
    staged.write_all(contents.as_bytes())?;
    staged
        .as_file()
        .set_permissions(regular(target)?.permissions())?;
    staged.as_file().sync_all()?;
    Ok(staged)
}

fn sync_directory(root: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(root)?.sync_all()?;
    // Windows does not provide a portable directory fsync via std.
    #[cfg(not(unix))]
    let _ = root;
    Ok(())
}

fn publish(
    root: &Path,
    journal: &Journal,
    staged: Vec<(PathBuf, tempfile::NamedTempFile)>,
    after_replace: impl Fn(usize) -> io::Result<()>,
) -> Result<(), Failure> {
    let path = root.join(JOURNAL);
    let mut record = temp(root).map_err(storage)?;
    serde_json::to_writer(&mut record, journal).map_err(|error| storage(error.into()))?;
    record.as_file().sync_all().map_err(storage)?;
    record
        .persist_noclobber(&path)
        .map_err(|error| storage(error.error))?;

    let result = (|| {
        sync_directory(root)?;
        for (index, (target, new)) in staged.into_iter().enumerate() {
            new.persist(target).map_err(|error| error.error)?;
            after_replace(index)?;
        }
        sync_directory(root)?;
        fs::remove_file(path)?;
        Ok(())
    })();

    if let Err(error) = result {
        recover(root).map_err(recovery_failure)?;
        return Err(storage(error));
    }
    // Removing the journal is the commit point. Failure to flush the
    // directory after that point cannot be reported as a rolled-back commit.
    sync_directory(root).map_err(|error| Failure::new("commit_uncertain", error.to_string()))
}

fn recover(root: &Path) -> io::Result<()> {
    let path = root.join(JOURNAL);
    let contents = match read(&path, MAX_JOURNAL) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let journal: Journal = serde_json::from_str(&contents)?;
    if journal.version != 1 || journal.changes.is_empty() || journal.changes.len() > MAX_FILES {
        return Err(io::Error::other("invalid edit journal"));
    }
    let mut staged = Vec::new();
    for (module, change) in journal.changes {
        if !super::valid_file(&module) {
            return Err(io::Error::other("invalid module in edit journal"));
        }
        let target = root.join(module);
        let current = read(&target, MAX_SOURCE_BYTES)?;
        if current != change.before {
            if current != change.after {
                return Err(io::Error::other(format!(
                    "external change blocks recovery: {}",
                    target.display()
                )));
            }
            staged.push((target.clone(), stage(&target, &change.before)?));
        }
    }
    for (target, original) in staged {
        original.persist(target).map_err(|error| error.error)?;
    }
    sync_directory(root)?;
    fs::remove_file(path)?;
    sync_directory(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edits::{Operand, Operation, SCHEMA_VERSION};

    fn fixture() -> (tempfile::TempDir, DiskWorkspace) {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("main.luma"), "use lib\n").unwrap();
        fs::write(root.path().join("lib.luma"), "# library\r\n").unwrap();
        let workspace = open(root.path());
        (root, workspace)
    }

    fn open(root: &Path) -> DiskWorkspace {
        DiskWorkspace::open(
            root,
            "main.luma".into(),
            vec!["lib.luma".into(), "main.luma".into()],
        )
        .unwrap()
    }

    fn proposal(workspace: &DiskWorkspace) -> Proposal {
        Proposal {
            schema_version: SCHEMA_VERSION,
            expected_revision: workspace.snapshot().revision.clone(),
            operations: ["lib.luma", "main.luma"]
                .into_iter()
                .map(|file| Operation::CreateValue {
                    file: file.into(),
                    name: "added".into(),
                    value: Operand::Number { value: 1 },
                })
                .collect(),
        }
    }

    fn assert_originals(root: &Path) {
        assert_eq!(
            fs::read_to_string(root.join("lib.luma")).unwrap(),
            "# library\r\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("main.luma")).unwrap(),
            "use lib\n"
        );
        assert!(!root.join(JOURNAL).exists());
    }

    #[test]
    fn preview_and_injected_write_failure_leave_originals() {
        let (root, mut workspace) = fixture();
        let proposal = proposal(&workspace);
        workspace.preview(&proposal).unwrap();
        assert_originals(root.path());
        let error = workspace
            .commit_with(&proposal, |index| {
                assert_eq!(index, 0);
                Err(io::Error::other("injected publication failure"))
            })
            .unwrap_err();
        assert_eq!(error.code, "storage_error");
        assert_originals(root.path());
        assert_eq!(workspace.snapshot().revision, proposal.expected_revision);
        workspace.commit(&proposal).unwrap();
        assert_eq!(open(root.path()).snapshot().ir, workspace.snapshot().ir);
    }

    #[test]
    fn second_session_detects_stale_disk_snapshot() {
        let (root, mut first) = fixture();
        let mut second = open(root.path());
        first.commit(&proposal(&first)).unwrap();
        let error = second.commit(&proposal(&second)).unwrap_err();
        assert_eq!(error.code, "source_conflict");
        assert_eq!(open(root.path()).snapshot().ir, first.snapshot().ir);
    }

    #[test]
    fn interrupted_child() {
        let Some(path) = std::env::var_os("LUMA_TEST_INTERRUPTED_ROOT") else {
            return;
        };
        let mut workspace = open(Path::new(&path));
        let proposal = proposal(&workspace);
        let _ = workspace.commit_with(&proposal, |_| std::process::exit(77));
        panic!("expected interruption after first replacement");
    }

    #[test]
    fn reopen_rolls_back_interrupted_process() {
        let (root, _) = fixture();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "edits::disk::tests::interrupted_child"])
            .env("LUMA_TEST_INTERRUPTED_ROOT", root.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(77));
        assert!(root.path().join(JOURNAL).is_file());
        assert!(fs::read_to_string(root.path().join("lib.luma"))
            .unwrap()
            .contains("added"));
        open(root.path());
        assert_originals(root.path());
    }

    #[test]
    fn recovery_refuses_to_overwrite_external_content() {
        let (root, _) = fixture();
        let journal = Journal {
            version: 1,
            changes: BTreeMap::from([(
                "lib.luma".into(),
                Change {
                    before: "# library\r\n".into(),
                    after: "added = 1\r\n".into(),
                },
            )]),
        };
        fs::write(
            root.path().join(JOURNAL),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        fs::write(root.path().join("lib.luma"), "# external\n").unwrap();
        assert!(recover(root.path()).is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("lib.luma")).unwrap(),
            "# external\n"
        );
        assert!(root.path().join(JOURNAL).is_file());
    }
}
