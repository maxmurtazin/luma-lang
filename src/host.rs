//! Host-aware checking and evaluation of admitted UTF-8 bundles.
use crate::{error, lex, runtime, semantic, Diagnostic, Span};
pub use luma_host_api::{CallSite, Dispatch, Effect, Mode, Registry, Value};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 16;
const MAX_BYTES: usize = 32 * 1024;
const MAX_TOKENS: usize = 2048;
pub(crate) const MAX_STEPS: usize = 100_000;
const EMPTY_SPAN: Span = Span { start: 0, end: 0 };

#[derive(Debug)]
pub struct SourceBundle {
    pub entry: String,
    pub files: BTreeMap<String, String>,
}

/// The bounded checker consumes the admitted snapshot rather than the unchecked wire frame.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireSourceBundle {
    bundle_version: u32,
    entry: String,
    #[serde(deserialize_with = "unique_files")]
    files: BTreeMap<String, String>,
    source_digest: String,
}

impl WireSourceBundle {
    pub fn admit(self) -> Result<SourceBundle, runtime::Failure> {
        if self.bundle_version != 1 {
            return Err(failure(
                &self.entry,
                "UnsupportedSchemaVersion",
                "expected SourceBundle version 1",
            ));
        }
        let bundle = SourceBundle {
            entry: self.entry,
            files: self.files,
        };
        bundle.validate_shape()?;
        if luma_cad_contract::source_digest(&bundle.entry, &bundle.files) != self.source_digest {
            return Err(failure(
                &bundle.entry,
                "SourceDigestMismatch",
                "source digest does not match the bundle bytes",
            ));
        }
        Ok(bundle)
    }
}

fn unique_files<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error as _, MapAccess, Visitor};

    struct Files;

    impl<'de> Visitor<'de> for Files {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a source file map with unique paths")
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut files = BTreeMap::new();
            while let Some((path, source)) = map.next_entry::<String, String>()? {
                if files.insert(path, source).is_some() {
                    return Err(M::Error::custom("duplicate source path"));
                }
            }
            Ok(files)
        }
    }

    deserializer.deserialize_map(Files)
}

impl SourceBundle {
    pub(crate) fn entry(&self) -> &Path {
        Path::new(&self.entry)
    }

    pub(crate) fn source(&self, path: &Path) -> Option<&str> {
        self.files.get(&bundle_path(path)?).map(String::as_str)
    }

    fn validate_shape(&self) -> Result<(), runtime::Failure> {
        if !valid_path(&self.entry) || !self.files.contains_key(&self.entry) {
            return Err(failure(
                &self.entry,
                "InvalidSourceBundle",
                "the bundle must contain its canonical entry path",
            ));
        }
        if self.files.len() > MAX_FILES
            || self
                .files
                .values()
                .try_fold(0usize, |total, text| total.checked_add(text.len()))
                .unwrap_or(usize::MAX)
                > MAX_BYTES
        {
            return Err(failure(
                &self.entry,
                "SourceBundleLimit",
                "bundle exceeds 16 files or 32 KiB",
            ));
        }
        for file in self.files.keys() {
            if !valid_path(file) {
                return Err(failure(
                    file,
                    "InvalidSourceBundle",
                    "noncanonical source path",
                ));
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), runtime::Failure> {
        self.validate_shape()?;
        let mut tokens = 0;
        for (file, source) in &self.files {
            let count = lex(source)
                .map_err(|diagnostic| runtime::Failure {
                    diagnostics: vec![(file.clone(), diagnostic)],
                    runtime: false,
                })?
                .len()
                - 1;
            tokens += count;
            if tokens > MAX_TOKENS {
                return Err(failure(
                    file,
                    "SourceBundleLimit",
                    "bundle exceeds 2048 tokens",
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn bundle_path(path: &Path) -> Option<String> {
    Some(path.to_str()?.replace('\\', "/"))
}

fn valid_path(path: &str) -> bool {
    path.ends_with(".luma")
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

fn failure(file: &str, code: &'static str, message: &str) -> runtime::Failure {
    runtime::Failure {
        diagnostics: vec![(file.into(), error(code, EMPTY_SPAN, message))],
        runtime: false,
    }
}

pub struct CheckedProgram {
    entry: PathBuf,
    mode: Mode,
    registry: Registry,
    report: semantic::Report,
}

#[derive(Debug, Serialize)]
pub struct RecordedCall {
    pub operation: String,
    pub major: u32,
    pub effect: Effect,
    pub site: CallSite,
    pub arguments: Vec<Value>,
    pub result: Value,
}

#[derive(Debug, Serialize)]
pub struct ExecutionResult {
    pub outputs: BTreeMap<String, Value>,
    pub calls: Vec<RecordedCall>,
    pub diagnostic_output: String,
    pub expression_steps: usize,
}

pub fn check_with_host(
    bundle: &SourceBundle,
    mode: Mode,
    registry: &Registry,
) -> Result<CheckedProgram, runtime::Failure> {
    bundle.validate()?;
    let mut report = semantic::check_bundle(bundle, registry, mode);
    if !report.diagnostics.is_empty() || !report.runtime_gates.is_empty() {
        let mut diagnostics = std::mem::take(&mut report.diagnostics);
        diagnostics.append(&mut report.runtime_gates);
        return Err(runtime::Failure {
            diagnostics,
            runtime: false,
        });
    }
    Ok(CheckedProgram {
        entry: bundle.entry().into(),
        mode,
        registry: registry.clone(),
        report,
    })
}

impl CheckedProgram {
    pub fn execute(
        self,
        host: &dyn Dispatch,
        expression_budget: usize,
    ) -> Result<ExecutionResult, runtime::Failure> {
        if host.registry() != &self.registry {
            return Err(failure(
                &self.entry.to_string_lossy(),
                "HostRegistryMismatch",
                "checker and runtime must use the same sealed descriptors",
            ));
        }
        runtime::run_bundle(
            self.report,
            &self.entry,
            self.mode,
            &self.registry,
            host,
            expression_budget,
        )
    }
}

pub fn run_with_host(
    bundle: &SourceBundle,
    mode: Mode,
    host: &dyn Dispatch,
    expression_budget: usize,
) -> Result<ExecutionResult, runtime::Failure> {
    check_with_host(bundle, mode, host.registry())?.execute(host, expression_budget)
}

pub fn diagnostic_json(file: &str, diagnostic: &Diagnostic) -> serde_json::Value {
    serde_json::json!({
        "file": file,
        "code": diagnostic.code,
        "message": diagnostic.message,
        "bytes": { "start": diagnostic.span.start, "end": diagnostic.span.end },
    })
}
