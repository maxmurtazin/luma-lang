//! Read-only Luma commands against one captured Living IR Elements catalog.
use elements_model::{Catalog, Error as ProviderError, Store, MAX_JSON_BYTES};
use luma::{
    host::{run_with_host, WireSourceBundle},
    host_api::{
        CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
        Value,
    },
};
use luma_cad_contract::SEALED_MODULES;
use serde_json::{json, Value as Json};
use std::{
    fs::File,
    io::Read,
    path::Path,
    process::ExitCode,
    sync::atomic::{AtomicUsize, Ordering},
};

const RESULTS: &str = "ElementSearchResult";
const PIN: &str = "ElementRevisionRef";
const DEFINITION: &str = "ElementDefinitionRef";
const MAX_CALLS: usize = 16;

struct CatalogHost {
    catalog: Catalog,
    exact_pin: Option<Json>,
    registry: Registry,
    calls: AtomicUsize,
}

impl CatalogHost {
    fn new(catalog: Catalog, exact_pin: Option<Json>) -> Result<Self, Error> {
        let opaque = |name: &str| Type::Opaque(name.into());
        let specs = [
            ("find", vec![Type::Text], RESULTS, Effect::ReadPinned),
            (
                "select",
                vec![opaque(RESULTS), Type::Number],
                PIN,
                Effect::Pure,
            ),
            ("pinned_revision", vec![], PIN, Effect::ReadPinned),
            ("resolve", vec![opaque(PIN)], DEFINITION, Effect::ReadPinned),
        ];
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            specs
                .into_iter()
                .map(|(name, parameters, result, effect)| Descriptor {
                    module: "elements".into(),
                    name: name.into(),
                    operation: format!("elements.{name}"),
                    major: 1,
                    parameters: parameters.into_iter().map(Parameter::Exact).collect(),
                    result: ResultType::Exact(opaque(result)),
                    effect,
                    modes: [Mode::Command].into(),
                }),
        )?;
        Ok(Self {
            catalog,
            exact_pin,
            registry,
            calls: AtomicUsize::new(0),
        })
    }
}

impl Dispatch for CatalogHost {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        _: &CallSite,
        args: &[Value],
    ) -> Result<Value, Error> {
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new("HostRegistryMismatch", &descriptor.operation));
        }
        descriptor.check_mode(mode)?;
        if self.calls.fetch_add(1, Ordering::Relaxed) >= MAX_CALLS {
            return Err(Error::new(
                "CatalogCommandLimit",
                "At most 16 Elements calls per command",
            ));
        }
        let (type_name, payload) = match (descriptor.name.as_str(), args) {
            ("find", [Value::Text(text)]) => {
                let request = json!({"text":text,"limit":16});
                let result = self
                    .catalog
                    .search_request(request.to_string().as_bytes())
                    .map_err(provider)?;
                (RESULTS, encode(result)?)
            }
            ("select", [Value::Opaque { type_name, payload }, Value::Number(index)])
                if type_name == RESULTS =>
            {
                if !(0.0..16.0).contains(index) || index.fract() != 0.0 {
                    return Err(Error::new(
                        "InvalidCatalogIndex",
                        "Expected a zero-based result index",
                    ));
                }
                let item = payload
                    .get("items")
                    .and_then(|items| items.get(*index as usize))
                    .ok_or_else(|| {
                        Error::new("MissingCatalogItem", "Index is outside the result page")
                    })?;
                (
                    PIN,
                    json!({
                        "source": item["source"], "definition_id": item["definition_id"],
                        "revision_id": item["revision_id"], "expected_revision_digest": item["revision_digest"]
                    }),
                )
            }
            ("pinned_revision", []) => (
                PIN,
                self.exact_pin.clone().ok_or_else(|| {
                    Error::new(
                        "MissingPackagePin",
                        "Supply an exact definition request to the catalog CLI",
                    )
                })?,
            ),
            ("resolve", [Value::Opaque { type_name, payload }]) if type_name == PIN => {
                let details = self
                    .catalog
                    .describe_exact(payload.to_string().as_bytes())
                    .map_err(provider)?;
                (DEFINITION, encode(details)?)
            }
            _ => return Err(Error::new("CatalogInputType", &descriptor.operation)),
        };
        Ok(Value::Opaque {
            type_name: type_name.into(),
            payload,
        })
    }
}

fn provider(error: ProviderError) -> Error {
    Error::new(error.code, error.to_string())
}

fn encode(value: impl serde::Serialize) -> Result<Json, Error> {
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| Error::new("CatalogEncoding", error.to_string()))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(Error::new(
            "CatalogResultLimit",
            "Elements result exceeds 1 MiB",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| Error::new("CatalogEncoding", error.to_string()))
}

fn read(path: &Path, limit: usize) -> Result<Vec<u8>, Json> {
    let io_error = |error: std::io::Error| json!({"code":"InputFile","message":error.to_string()});
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > limit {
        return Err(json!({"code":"InputLimit"}));
    }
    Ok(bytes)
}

fn execute() -> Result<Json, Json> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [database, bundle, exact @ ..] = args.as_slice() else {
        return Err(
            json!({"code":"Usage","message":"luma-cad-elements CATALOG.sqlite COMMAND.json [EXACT_PIN.json]"}),
        );
    };
    let invalid_json =
        |error: serde_json::Error| json!({"code":"InvalidInput","message":error.to_string()});
    let bundle: WireSourceBundle =
        serde_json::from_slice(&read(Path::new(bundle), 256 * 1024)?).map_err(invalid_json)?;
    let diagnostics = |error: luma::runtime::Failure| {
        let items: Vec<_> = error
            .diagnostics
            .into_iter()
            .map(|(file, diagnostic)| {
                json!({
                    "file": file, "code": diagnostic.code, "message": diagnostic.message,
                    "start": diagnostic.span.start, "end": diagnostic.span.end
                })
            })
            .collect();
        json!({"code":"CatalogCommandRejected","diagnostics":items})
    };
    let bundle = bundle.admit().map_err(diagnostics)?;
    let provider_error =
        |error: ProviderError| json!({"code":error.code,"path":error.path,"message":error.message});
    let catalog = Store::open_read_only(Path::new(database))
        .and_then(|store| store.catalog())
        .map_err(provider_error)?;
    let exact = match exact {
        [] => None,
        [path] => {
            let input = read(Path::new(path), 8192)?;
            catalog.describe_exact(&input).map_err(provider_error)?;
            Some(serde_json::from_slice(&input).map_err(invalid_json)?)
        }
        _ => return Err(json!({"code":"Usage"})),
    };
    let host = CatalogHost::new(catalog, exact)
        .map_err(|error| json!({"code":error.code,"message":error.message}))?;
    let execution = run_with_host(&bundle, Mode::Command, &host, 100_000).map_err(diagnostics)?;
    Ok(json!({
        "elements_revision": include_str!("luma-elements-pin.txt").trim(),
        "command_digest": bundle.source_digest(),
        "outputs": execution.outputs
    }))
}

fn main() -> ExitCode {
    let (status, envelope) = match execute() {
        Ok(result) => (ExitCode::SUCCESS, json!({"result":result})),
        Err(error) => (ExitCode::from(2), json!({"error":error})),
    };
    println!("{envelope}");
    status
}
