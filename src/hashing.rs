//! SHA-256 helpers for evidence and fixture verification.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Hash raw bytes and return a lowercase hex string.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Hash a file by reading it fully and returning a lowercase hex string.
pub(crate) fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
