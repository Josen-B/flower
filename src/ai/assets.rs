//! Fixed, build-pinned asset inventory; manifest never supplies filesystem paths.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    model: serde_json::Value,
    runtime: serde_json::Value,
    files: Vec<Asset>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    path: String,
    size: u64,
    sha256: String,
}

pub fn verify(bundle: &Path) -> Result<PathBuf, String> {
    let pinned: Manifest = serde_json::from_str(include_str!("../../data/ai/manifest.json"))
        .map_err(|_| "AI asset inventory invalid")?;
    if !bundle.is_absolute() {
        return Err("AI bundle path must be absolute".into());
    }
    let bundle = bundle.canonicalize().map_err(|_| "AI bundle unavailable")?;
    let manifest_path = bundle.join("manifest.json");
    let manifest_meta =
        std::fs::symlink_metadata(&manifest_path).map_err(|_| "AI manifest unavailable")?;
    if !manifest_meta.is_file()
        || manifest_meta.file_type().is_symlink()
        || manifest_meta.len() != include_str!("../../data/ai/manifest.json").len() as u64
    {
        return Err("AI manifest mismatch".into());
    }
    let manifest = std::fs::read(manifest_path).map_err(|_| "AI manifest unavailable")?;
    if manifest != include_str!("../../data/ai/manifest.json").as_bytes()
        || pinned.schema_version != 1
        || !pinned.model.is_object()
        || !pinned.runtime.is_object()
        || pinned.files.len() != 8
    {
        return Err("AI manifest mismatch".into());
    }
    for asset in pinned.files {
        // Only names embedded at build time, never untrusted manifest paths.
        if asset.path.contains(['/', '\\', ':']) || asset.path.starts_with('.') {
            return Err("AI asset inventory invalid".into());
        }
        let path = bundle.join(&asset.path);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| "AI asset missing")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != asset.size
        {
            return Err("AI asset size invalid".into());
        }
        let mut file = std::fs::File::open(path).map_err(|_| "AI asset unavailable")?;
        let mut digest = Sha256::new();
        let mut block = [0u8; 65536];
        loop {
            let n = file.read(&mut block).map_err(|_| "AI asset unreadable")?;
            if n == 0 {
                break;
            }
            digest.update(&block[..n]);
        }
        if format!("{:x}", digest.finalize()) != asset.sha256 {
            return Err("AI asset checksum invalid".into());
        }
    }
    Ok(bundle)
}

/// Process-wide: invoke only inside the dedicated inference helper.
#[cfg(windows)]
pub fn restrict_dll_search() -> Result<(), String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetDefaultDllDirectories(flags: u32) -> i32;
    }
    // System32 only: excludes cwd, PATH and writable application directories.
    // ORT itself is loaded using its already verified absolute path.
    if unsafe { SetDefaultDllDirectories(0x00000800) } == 0 {
        Err("AI runtime search policy failed".into())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
pub fn restrict_dll_search() -> Result<(), String> {
    Err("AI runtime requires Windows".into())
}
