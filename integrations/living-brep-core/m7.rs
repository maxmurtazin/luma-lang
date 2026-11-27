//! Qualify a captured pair for M7 and verify the provider's native result.
use crate::{artifact, encoding_error, Failure, Result, OUTPUT_BYTES};
use brep_core::{
    decode_native, BrepKernel, Frame3, KernelContext, ProfileDefinition, SourceSegment,
    StandaloneKernel, ValidatedBrep, K1_SOLID,
};
use luma::cad_engine::{PlanarOperand, PlanarPlan};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const REQUEST_BYTES: usize = 64 * 1024;
const REPLY_BYTES: usize = 2 * OUTPUT_BYTES + REQUEST_BYTES;
const EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;

pub(crate) fn run(
    provider: &str,
    bundle: &str,
    publications: [(&str, &ValidatedBrep); 2],
    context: &mut KernelContext,
) -> Result<Value> {
    let bundle = crate::artifact::read_luma_bundle(bundle)?;
    let plan = luma::cad::prepare_planar_boolean(&bundle, 100_000)
        .map_err(|problem| Failure("M7PlanRejected", json!(format!("{problem:?}"))))?;
    let [a, b] = plan.operands.map(|role| match role {
        PlanarOperand::Left => publications[0],
        PlanarOperand::Right => publications[1],
    });
    let request = serde_json::to_vec(&fixture(&plan, [a.1, b.1])?).map_err(encoding_error)?;
    if request.len() > REQUEST_BYTES {
        return Err(Failure("ResourceLimit", json!("M7 input exceeds 64 KiB")));
    }
    let provider = Path::new(provider).canonicalize().map_err(io_error)?;
    if !provider.is_file() {
        return Err(Failure(
            "InvalidM7Provider",
            json!("Expected a qualified M7 executable"),
        ));
    }
    let fingerprint = sha_file(&provider)?;
    let (status, manifest) = invoke(
        &provider,
        &[Path::new("--capabilities")],
        context,
        REQUEST_BYTES,
    )?;
    let expected: Value =
        serde_json::from_str(include_str!("m7-contract.json")).map_err(encoding_error)?;
    let expected = expected.as_object().ok_or_else(invalid_reply)?;
    let target = StandaloneKernel.capabilities().build_target;
    if status != 0
        || expected.iter().any(|(key, value)| manifest[key] != *value)
        || manifest["build_target"] != target
        || !manifest["supported_targets"]
            .as_array()
            .is_some_and(|targets| targets.iter().any(|item| item == target))
        || manifest["build_fingerprint"] != fingerprint
    {
        return Err(Failure(
            "InvalidM7Provider",
            json!("Provider does not match the qualified M7 contract and target"),
        ));
    }

    let mut input = tempfile::NamedTempFile::new().map_err(io_error)?;
    input.write_all(&request).map_err(io_error)?;
    input.flush().map_err(io_error)?;
    let (status, reply) = invoke(
        &provider,
        &[Path::new("--with-native"), input.path()],
        context,
        REPLY_BYTES,
    )?;
    if sha_file(&provider)? != fingerprint {
        return Err(Failure(
            "InvalidM7Provider",
            json!("M7 executable changed during evaluation"),
        ));
    }
    if status != 0 {
        let code = match status {
            2 => "UnsupportedM7Operation",
            3 => "M7Unresolved",
            4 if reply["outcome"] == "Cancelled" => "Cancelled",
            4 => "ResourceLimit",
            _ => "M7ProviderFailure",
        };
        return Err(Failure(
            code,
            json!({
                "provider_outcome":reply["outcome"],"provider_code":reply["code"],"diagnostics":reply["diagnostics"]
            }),
        ));
    }
    let request_digest = artifact::hex(&Sha256::digest(&request));
    validate_reply(&reply, &request_digest, a.1, context)?;
    Ok(json!({
        "schema":"luma.cad.m7-result/1", "operation_id":plan.operation.provider_id(),
        "operation_version":1,"result_key":plan.key,"source_digest":bundle.source_digest(),"request_sha256":request_digest,
        "operands":[{"operation":a.0,"geometry_sha256":artifact::hex(&a.1.blob_digest())},
                    {"operation":b.0,"geometry_sha256":artifact::hex(&b.1.blob_digest())}],
        "provider":{"contract_commit":include_str!("brep-pin.txt").trim(),"build_fingerprint":fingerprint,
                    "extrusion_backend_fingerprint":manifest["extrusion_backend_fingerprint"],"target":target},
        "result":reply
    }))
}

fn fixture(plan: &PlanarPlan, [a, b]: [&ValidatedBrep; 2]) -> Result<Value> {
    let a_profile = a.profile();
    let b_profile = b.profile();
    if !same_frame(a_profile.frame, b_profile.frame) || a.signed_height_m() != b.signed_height_m() {
        return Err(Failure(
            "UnsupportedM7Operands",
            json!("Expected one common frame and signed height"),
        ));
    }
    let mut request = json!({"frame":a_profile.frame,"signed_height_m":a.signed_height_m(),"operation":plan.operation});
    let a_disk = disk(a_profile);
    let b_disk = disk(b_profile);
    match (a_disk, b_disk) {
        (Some(a), Some(b)) => request["disks"] = json!([a, b]),
        (Some(disk), None) => {
            request["mixed"] = json!({"polygon_m": polygon(b_profile)?,"disk":disk,"first":"Disk"})
        }
        (None, Some(disk)) => {
            request["mixed"] =
                json!({"polygon_m": polygon(a_profile)?,"disk":disk,"first":"Polygon"})
        }
        (None, None) => {
            let bodies = [a_profile,b_profile].map(|profile| json!({
                "vertices":profile.vertices,"outer_loop":profile.outer_loop,"hole_loops":profile.hole_loops
            }));
            request["profiles"] = json!(bodies);
        }
    }
    Ok(request)
}

fn same_frame(a: Frame3, b: Frame3) -> bool {
    a.origin_m == b.origin_m && a.x_axis == b.x_axis && a.y_axis == b.y_axis && a.z_axis == b.z_axis
}

fn disk(profile: &ProfileDefinition) -> Option<Value> {
    match profile.outer_loop.segments.as_slice() {
        [SourceSegment::Circle {
            center_m, radius_m, ..
        }] if profile.hole_loops.is_empty() => {
            Some(json!({"center_m":center_m,"radius_m":radius_m}))
        }
        _ => None,
    }
}

fn polygon(profile: &ProfileDefinition) -> Result<Vec<[f64; 2]>> {
    let unsupported = || {
        Failure(
            "UnsupportedM7Operands",
            json!("Mixed disk input requires a straight polygon without holes"),
        )
    };
    if !profile.hole_loops.is_empty() {
        return Err(unsupported());
    }
    profile
        .outer_loop
        .segments
        .iter()
        .map(|segment| {
            let SourceSegment::Line { from_vertex, .. } = segment else {
                return Err(unsupported());
            };
            profile
                .vertices
                .iter()
                .find(|vertex| vertex.source_id == *from_vertex)
                .map(|vertex| vertex.uv_m)
                .ok_or_else(unsupported)
        })
        .collect()
}

fn validate_reply(
    reply: &Value,
    request_digest: &str,
    source: &ValidatedBrep,
    context: &mut KernelContext,
) -> Result<()> {
    if reply["outcome"] != "Success"
        || reply["request_sha256"] != request_digest
        || reply["validation"]["level"] != "L3K1Solid"
        || reply["validation"]["complete"] != true
        || reply["native"]["format"] != "brep-cbor-v1"
    {
        return Err(invalid_reply());
    }
    let hash = reply["native"]["blob_digest"]
        .as_str()
        .and_then(|text| decode_hex(text, 32))
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(invalid_reply)?;
    let bytes = reply["native"]["bytes_hex"]
        .as_str()
        .and_then(|text| decode_hex(text, OUTPUT_BYTES))
        .ok_or_else(invalid_reply)?;
    let shape = decode_native(&bytes, hash, context).map_err(|_| invalid_reply())?;
    let model = artifact::outcome(StandaloneKernel.accept(shape, K1_SOLID, context))?;
    if model.signed_height_m() != source.signed_height_m()
        || !same_frame(model.profile().frame, source.profile().frame)
        || json!(artifact::interval(model.volume_m3())) != reply["volume_m3"]
    {
        return Err(invalid_reply());
    }
    Ok(())
}

fn decode_hex(value: &str, limit: usize) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2)
        || value.len() / 2 > limit
        || !crate::lower_hex(value, value.len())
    {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|idx| u8::from_str_radix(&value[idx..idx + 2], 16).ok())
        .collect()
}

fn sha_file(path: &Path) -> Result<String> {
    let file = File::open(path).map_err(io_error)?;
    let mut hash = Sha256::new();
    if io::copy(&mut file.take(EXECUTABLE_BYTES + 1), &mut hash).map_err(io_error)?
        > EXECUTABLE_BYTES
    {
        return Err(Failure(
            "ResourceLimit",
            json!("M7 executable exceeds the size limit"),
        ));
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

fn invoke(
    provider: &Path,
    args: &[&Path],
    ctx: &KernelContext,
    limit: usize,
) -> Result<(i32, Value)> {
    let mut stdout = tempfile::tempfile().map_err(io_error)?;
    let mut child = Command::new(provider)
        .args(args)
        .env_remove("LUMA_CAD_PROJECT_KEY")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(stdout.try_clone().map_err(io_error)?)
        .spawn()
        .map_err(io_error)?;
    let status = loop {
        let result = child.try_wait();
        match result {
            Ok(Some(status)) => break status.code().unwrap_or(5),
            Ok(None) => {}
            Err(problem) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io_error(problem));
            }
        }
        let stop = if ctx.cancellation.is_cancelled() {
            Some("Cancelled")
        } else if ctx
            .budget
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
            || stdout
                .metadata()
                .is_ok_and(|metadata| metadata.len() > limit as u64)
        {
            Some("ResourceLimit")
        } else {
            None
        };
        if let Some(code) = stop {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failure(
                code,
                json!("M7 provider exceeded its execution budget"),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    stdout.rewind().map_err(io_error)?;
    let mut bytes = Vec::new();
    stdout
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > limit {
        return Err(Failure(
            "ResourceLimit",
            json!("M7 reply exceeds the size limit"),
        ));
    }
    let reply = serde_json::from_slice(&bytes).map_err(encoding_error)?;
    Ok((status, reply))
}

fn invalid_reply() -> Failure {
    Failure(
        "InvalidM7Result",
        json!("M7 native result did not match the verified request"),
    )
}

fn io_error(problem: io::Error) -> Failure {
    Failure("M7Io", json!(problem.to_string()))
}
