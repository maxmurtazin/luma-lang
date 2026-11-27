//! Wire adapter for the separately linked native Living Voxel CLI.
use super::{io_error, read, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn operate(provider: &Path, operation: &str, request: &Value) -> Result<Value> {
    let temp = tempfile::tempdir().map_err(io_error)?;
    let input = temp.path().join("request.json");
    fs::write(&input, request.to_string()).map_err(io_error)?;
    let outcome = call(
        provider,
        &[
            "operate".as_ref(),
            operation.as_ref(),
            "1".as_ref(),
            input.as_os_str(),
        ],
        &temp.path().join("response.json"),
    )?;
    Ok(outcome["result"].clone())
}

pub(super) fn decode(provider: &Path, bytes: &[u8], digest: &[u8; 32]) -> Result<Value> {
    if !bytes.starts_with(b"LVXGRID\0") || Sha256::digest(bytes)[..] != digest[..] {
        return Err(json!({"code":"CorruptArtifact"}));
    }
    let temp = tempfile::tempdir().map_err(io_error)?;
    let input = temp.path().join("source.lvx");
    fs::write(&input, bytes).map_err(io_error)?;
    let expected = hex::encode(digest);
    let outcome = call(
        provider,
        &["decode".as_ref(), input.as_os_str(), expected.as_ref()],
        &temp.path().join("response.json"),
    )?;
    let field = &outcome["result"]["field"];
    if !field.is_object() {
        return Err(json!({"code":"CorruptArtifact"}));
    }
    Ok(field.clone())
}

fn call(provider: &Path, args: &[&std::ffi::OsStr], response: &Path) -> Result<Value> {
    let mut child = Command::new(provider.canonicalize().map_err(io_error)?)
        .args(args)
        .env_remove("LUMA_CAD_PROJECT_KEY")
        .env_remove("LUMA_CAD_TEST_CRASH_AT")
        .stdin(Stdio::null())
        .stdout(File::create(response).map_err(io_error)?)
        .stderr(Stdio::null())
        .spawn()
        .map_err(io_error)?;
    let timeout = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(io_error)? {
            break status;
        }
        if Instant::now() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(json!({"code":"LimitExceeded","message":"Voxel CLI timed out"}));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let failure = || json!({"code":"VoxelProviderRejected"});
    let mut outcome: Value =
        serde_json::from_slice(&read(response, 16 * 1024 * 1024)?).map_err(|_| failure())?;
    if status.success() && outcome["outcome"] == "Success" && outcome["result"].is_object() {
        return Ok(outcome);
    }
    if outcome["outcome"] == "LimitExceeded" {
        return Err(json!({"code":"LimitExceeded","diagnostic":outcome["diagnostic"]}));
    }
    let diagnostic = outcome["diagnostic"].take();
    Err(if diagnostic["code"].is_string() {
        diagnostic
    } else {
        failure()
    })
}
