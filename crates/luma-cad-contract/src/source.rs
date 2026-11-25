use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// LCAN/1 identity for the SourceBundle v1 entry and exact UTF-8 contents.
/// The transport digest field and any checkout path are outside the preimage.
pub fn source_digest(entry: &str, files: &BTreeMap<String, String>) -> String {
    let mut payload = String::from(r#"{"bundle_version":{"$u64":"1"},"entry":"#);
    quote(&mut payload, entry);
    payload.push_str(r#","files":{"#);
    for (index, (path, source)) in files.iter().enumerate() {
        if index != 0 {
            payload.push(',');
        }
        quote(&mut payload, path);
        payload.push(':');
        quote(&mut payload, source);
    }
    payload.push_str("}}");
    let mut hash = Sha256::new();
    hash.update(b"luma.cad.source\0LCAN/1\0");
    hash.update((payload.len() as u64).to_be_bytes());
    hash.update(payload.as_bytes());
    format!("sha256:{:x}", hash.finalize())
}

fn quote(payload: &mut String, text: &str) {
    use std::fmt::Write as _;
    payload.push('"');
    for character in text.chars() {
        match character {
            '"' => payload.push_str(r#"\""#),
            '\\' => payload.push_str(r#"\\"#),
            '\0'..='\u{1f}' => write!(payload, "\\u{:04x}", u32::from(character)).unwrap(),
            _ => payload.push(character),
        }
    }
    payload.push('"');
}
