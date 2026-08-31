// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// shebangpatch.rs — Rewrites `#!` interpreter lines in extracted scripts.
//
// Root cause (confirmed independently in a sibling packaging pipeline —
// substrate/cosmic-zainium's `ci/build.sh` — and live-tested in zex's own
// identical `src/cmd/install/shebangpatch.rs`): `exec()` resolves a
// script's shebang path LITERALLY, with no search or fallback. Alpine/
// Void scripts ship `#!/bin/sh`, `#!/usr/bin/env bash`, etc. — all FHS
// paths that don't exist on Zainium (no `/bin`, no `/usr/bin` at all).
// Confirmed live: an unpatched script fails to exec with plain ENOENT
// even though it installed and looks completely normal. Neither
// `elfpatch.rs` (ELF only) nor `pcpatch.rs` (`.pc` only) catches this —
// a shebang script is neither. This module never existed here before.

use std::io::{Read, Write};
use std::path::Path;

const SYSHUB_BIN: &str = "/overlayer/syshub/bin";
const UNION_BIN: &str = "/overlayer/zexlib/union/bin";

const FHS_BIN_ROOTS: &[&str] = &[
    "/usr/local/bin/", "/usr/local/sbin/",
    "/usr/bin/", "/usr/sbin/",
    "/bin/", "/sbin/",
];

fn is_script(head: &[u8]) -> bool {
    head.starts_with(b"#!")
}

/// Returns the real path to `bin_name` if it exists under syshub (checked
/// first) or the union tree, `None` if it exists in neither.
fn find_existing(bin_name: &str) -> Option<String> {
    let syshub_path = format!("{SYSHUB_BIN}/{bin_name}");
    if Path::new(&syshub_path).exists() {
        return Some(syshub_path);
    }
    let union_path = format!("{UNION_BIN}/{bin_name}");
    if Path::new(&union_path).exists() {
        return Some(union_path);
    }
    None
}

/// Maps an FHS interpreter path to its real Zainium location — see the
/// identical, live-tested version in zex's own `shebangpatch.rs` for the
/// full reasoning (syshub preferred, union fallback, `sh` specifically
/// falls back to a real shell if `sh` itself isn't a real file — confirmed
/// live this actually happens: this environment has `bash` but no `sh`).
fn rewrite_interpreter(interp: &str) -> Option<String> {
    let base_name = FHS_BIN_ROOTS.iter().find_map(|root| interp.strip_prefix(root))?;
    if base_name.is_empty() {
        return None;
    }
    if let Some(found) = find_existing(base_name) {
        return Some(found);
    }
    if base_name == "sh" {
        for candidate in ["bash", "dash", "ash", "busybox"] {
            if let Some(found) = find_existing(candidate) {
                return Some(found);
            }
        }
    }
    Some(format!("{SYSHUB_BIN}/{base_name}"))
}

/// Rewrite one file's shebang line in place, if it has one that needs it.
/// No-op if `path` isn't a `#!`-script, or its interpreter isn't under a
/// recognized FHS bin root, or the shebang is already correct.
pub fn patch_one(path: &Path) -> std::io::Result<()> {
    let mut head = [0u8; 4096];
    let n = {
        let mut f = std::fs::File::open(path)?;
        f.read(&mut head)?
    };
    let head = &head[..n];
    if !is_script(head) {
        return Ok(());
    }
    let Some(nl) = head.iter().position(|&b| b == b'\n') else {
        return Ok(());
    };
    let Ok(first_line) = std::str::from_utf8(&head[..nl]) else {
        return Ok(());
    };

    let rest = &first_line[2..];
    let (interp, args) = rest.trim_start().split_once(' ').unwrap_or((rest.trim_start(), ""));

    let Some(new_interp) = rewrite_interpreter(interp) else {
        return Ok(());
    };
    let new_shebang = if args.is_empty() {
        format!("#!{new_interp}")
    } else {
        format!("#!{new_interp} {args}")
    };
    if new_shebang == first_line {
        return Ok(());
    }

    let full = std::fs::read(path)?;
    let old_nl = full.iter().position(|&b| b == b'\n').unwrap_or(full.len());
    let mut out = new_shebang.into_bytes();
    out.push(b'\n');
    out.extend_from_slice(&full[(old_nl + 1).min(full.len())..]);

    let mut out_f = std::fs::File::create(path)?;
    out_f.write_all(&out)?;
    Ok(())
}
