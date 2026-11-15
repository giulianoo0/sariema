// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

/// Writes a GLB with this JSON chunk and binary chunk into the test target
/// directory and returns its path.
pub fn write_glb(name: &str, json: &str, bin: &[u8]) -> PathBuf {
    let mut json = json.as_bytes().to_vec();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut bin = bin.to_vec();
    bin.resize(bin.len().next_multiple_of(4), 0);
    let bin_chunk = if bin.is_empty() { 0 } else { 8 + bin.len() };
    let total = 12 + 8 + json.len() + bin_chunk;

    let mut glb = Vec::with_capacity(total);
    glb.extend(b"glTF");
    glb.extend(2u32.to_le_bytes());
    glb.extend((total as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(json);
    if !bin.is_empty() {
        glb.extend((bin.len() as u32).to_le_bytes());
        glb.extend(b"BIN\0");
        glb.extend(bin);
    }
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, glb).unwrap();
    path
}
