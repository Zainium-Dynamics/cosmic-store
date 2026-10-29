// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// Zainium store install layout (never syshub for store packages):
//   union:    /overlayer/zexlib/union/{bin,sbin,lib,share,etc,...}
//   receipts: /overlayer/zexlib/registry/installed/<name>.toml

use std::path::{Path, PathBuf};

/// App packages land here — not syshub.
pub const ZEXLIB_UNION: &str = "/overlayer/zexlib/union";

/// ZEX PACKAGE RECEIPTS directory (`<name>.toml`).
pub const ZEXLIB_REGISTRY_INSTALLED: &str = "/overlayer/zexlib/registry/installed";

/// Download / stage cache.
pub const ZEXLIB_CACHE: &str = "/overlayer/zexlib/union/var/cache/store";

/// Resolve union root (allow override for host testing).
pub fn union_root() -> PathBuf {
    PathBuf::from(
        std::env::var("ZAINIUM_ZEXLIB_UNION").unwrap_or_else(|_| ZEXLIB_UNION.to_string()),
    )
}

pub fn registry_installed_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("ZAINIUM_ZEXLIB_REGISTRY")
            .unwrap_or_else(|_| ZEXLIB_REGISTRY_INSTALLED.to_string()),
    )
}

pub fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var("ZAINIUM_ZEXLIB_CACHE").unwrap_or_else(|_| ZEXLIB_CACHE.to_string()))
}

/// Map a path from a foreign package (usually under `/usr` or relative `./usr`) into the union tree.
pub fn map_to_union(package_path: &str) -> Option<PathBuf> {
    let p = package_path.trim_start_matches("./");
    let p = p.trim_start_matches('/');

    // Strip common prefixes
    let rest = if let Some(r) = p.strip_prefix("usr/") {
        r
    } else if p.starts_with("bin/")
        || p.starts_with("sbin/")
        || p.starts_with("lib/")
        || p.starts_with("lib64/")
        || p.starts_with("share/")
        || p.starts_with("etc/")
        || p.starts_with("include/")
        || p.starts_with("libexec/")
        || p.starts_with("var/")
    {
        p
    } else {
        // Skip non-installable metadata at package root
        return None;
    };

    // lib64 → lib on Zainium
    let rest = rest
        .strip_prefix("lib64/")
        .map(|r| format!("lib/{r}"))
        .unwrap_or_else(|| rest.to_string());

    Some(union_root().join(rest))
}

/// Classify a union absolute path into a receipt `[files]` bucket.
pub fn classify_union_path(abs: &Path) -> Option<&'static str> {
    let union = union_root();
    let rel = abs.strip_prefix(&union).ok()?;
    let first = rel.components().next()?;
    match first.as_os_str().to_str()? {
        "bin" => Some("bin"),
        "sbin" => Some("sbin"),
        "lib" | "lib64" => Some("lib"),
        "etc" => Some("etc"),
        "share" => Some("share"),
        "include" | "libexec" | "var" => Some("share"), // bucket overflow → share
        _ => Some("share"),
    }
}

pub fn ensure_layout() -> std::io::Result<()> {
    let union = union_root();
    for sub in ["bin", "sbin", "lib", "libexec", "include", "share", "etc", "var"] {
        std::fs::create_dir_all(union.join(sub))?;
    }
    std::fs::create_dir_all(registry_installed_dir())?;
    std::fs::create_dir_all(cache_dir())?;
    Ok(())
}
