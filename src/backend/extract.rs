// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// Shared download + tar extract into zexlib union (musl package trees).

use flate2::read::{GzDecoder, MultiGzDecoder};
use std::{
    error::Error,
    fs::{self, File},
    io::Read,
    path::Path,
    time::Duration,
};

use super::{
    install_root::{ensure_layout, map_to_union},
    receipt::ZexReceipt,
};

pub fn download(url: &str, dest: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    log::info!("downloading {url}");
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(180))
        .call()
        .map_err(|e| format!("HTTP GET {url}: {e}"))?;
    let mut reader = resp.into_reader();
    let tmp = dest.with_extension("part");
    let mut file = File::create(&tmp)?;
    std::io::copy(&mut reader, &mut file)?;
    file.sync_all()?;
    fs::rename(&tmp, dest)?;
    Ok(())
}

pub fn decompress_to_bytes(data: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    // zstd
    if data.len() >= 4 && data[0..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        let mut dec = zstd::stream::read::Decoder::new(data)?;
        let mut out = Vec::new();
        dec.read_to_end(&mut out)?;
        return Ok(out);
    }
    // gzip (multi-member for Alpine APKINDEX / .apk)
    if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        let mut dec = MultiGzDecoder::new(data);
        let mut out = Vec::new();
        dec.read_to_end(&mut out)?;
        if out.is_empty() {
            let mut dec = GzDecoder::new(data);
            dec.read_to_end(&mut out)?;
        }
        return Ok(out);
    }
    Ok(data.to_vec())
}

/// Extract a tar (optionally gzip/zstd compressed) package into the union tree.
/// Skips metadata files (plist, PKGINFO, SIGN, and similar control files).
pub fn extract_archive_to_union(
    archive_path: &Path,
    receipt: &mut ZexReceipt,
) -> Result<usize, Box<dyn Error>> {
    ensure_layout()?;
    let data = fs::read(archive_path)?;
    let tar_bytes = decompress_to_bytes(&data)?;
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    let mut count = 0usize;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        let base = path.rsplit('/').next().unwrap_or(&path);

        // Skip package metadata / control
        if base.starts_with('.')
            || base == "props.plist"
            || base == "files.plist"
            || base == "PKGINFO"
            || base.starts_with(".PKGINFO")
            || base.starts_with(".SIGN")
            || base == "INSTALL"
            || base == "REMOVE"
            || base == "build"
            || base == "checksums"
            || base == "depends"
            || base == "sources"
            || base == "version"
            || base == "README"
        {
            continue;
        }

        if entry.header().entry_type().is_dir() {
            if let Some(target) = map_to_union(&path) {
                fs::create_dir_all(&target)?;
            }
            continue;
        }

        if !entry.header().entry_type().is_file() && !entry.header().entry_type().is_symlink() {
            continue;
        }

        let Some(target) = map_to_union(&path) else {
            continue;
        };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        if entry.header().entry_type().is_symlink() {
            if let Ok(Some(link)) = entry.link_name() {
                let _ = fs::remove_file(&target);
                #[cfg(unix)]
                std::os::unix::fs::symlink(link.as_ref(), &target)?;
                receipt.add_union_file(&target);
                count += 1;
            }
            continue;
        }

        {
            let mut out = File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(mode) = entry.header().mode() {
                let _ = fs::set_permissions(&target, fs::Permissions::from_mode(mode));
            }
        }

        // ELF-patch (PT_INTERP/RPATH) and pkg-config-patch (`.pc` path
        // variables) each newly-written file right away — no separate
        // staging root here (files land directly on their live union
        // destination), so this is the only "just written, before the
        // caller sees success" window there is. `patch_one` in each
        // module no-ops on anything that isn't actually an ELF/`.pc` file
        // respectively, so calling both unconditionally on every regular
        // file is cheap and safe.
        //
        // Both are fatal (`?`), not logged-and-ignored — a binary that
        // fails ELF-patching is exactly as broken as one that fails to
        // write to disk (it simply won't run), and every other operation
        // in this loop already aborts the whole extraction via `?` on
        // failure; silently downgrading only these two to a warning would
        // report "installed successfully" for a package that doesn't
        // actually work. Matches `zex`'s own installer, which treats the
        // identical patch failure as fatal.
        super::elfpatch::patch_one(&target)
            .map_err(|e| format!("ELF patch failed for {}: {e}", target.display()))?;
        // `union_root()`, not the bare `ZEXLIB_UNION` constant — it
        // resolves the `ZAINIUM_ZEXLIB_UNION` env override the same way
        // `map_to_union` above already did to compute `target` itself.
        // Hardcoding the constant here would write a `.pc` claiming the
        // *default* prefix even when the file actually landed under a
        // test-overridden root, silently mismatching the real path.
        let union_root = super::install_root::union_root();
        super::pcpatch::patch_one(&target, &union_root.to_string_lossy())
            .map_err(|e| format!("pkg-config patch failed for {}: {e}", target.display()))?;
        // Shebang scripts (`#!/bin/sh`, `#!/usr/bin/env ...`) — exec()
        // resolves that path literally with no fallback, and Zainium has
        // no `/bin` or `/usr/bin` at all, so an unpatched script fails
        // with ENOENT despite installing "successfully". Live-confirmed
        // in zex's own identical fix.
        super::shebangpatch::patch_one(&target)
            .map_err(|e| format!("shebang patch failed for {}: {e}", target.display()))?;

        receipt.add_union_file(&target);
        count += 1;
    }
    Ok(count)
}
