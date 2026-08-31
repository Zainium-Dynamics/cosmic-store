// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// pcpatch.rs — Rewrites pkg-config `.pc` file path variables for
// extracted Alpine/Void packages.
//
// Alpine/Void `-dev` packages ship pkg-config `.pc` metadata with the
// `prefix=`/`libdir=`/`includedir=` variables baked in against their own
// FHS layout (`prefix=/usr`), which doesn't resolve once the file lands
// under `/overlayer/zexlib/union/`. Left unpatched, any tool invoking
// `pkg-config --cflags`/`--libs` against a store-installed dependency
// gets headers/lib paths that don't exist on disk. Same fix, same
// reasoning, as zex's own `src/cmd/install/pcpatch.rs` — this module
// never existed here at all before now.
//
// Plain-text rewrite, not an ELF concern — nothing for `oxipatch` (see
// `elfpatch.rs`) to do here.

use std::path::Path;

/// Roots this rewrite recognizes and will always replace with
/// [`super::install_root::ZEXLIB_UNION`] — `/usr/local` before the
/// shorter `/usr` so it isn't matched as a `/usr` prefix with `/local/...`
/// left dangling.
const KNOWN_ROOTS: &[&str] = &["/usr/local", "/usr", "/overlayer/syshub", "/overlayer/zexlib/union"];

pub fn is_pc_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pc"))
        .unwrap_or(false)
}

/// Rewrite one `.pc` file's variable-definition lines in place.
///
/// Only touches `identifier=value` lines whose value is a literal
/// absolute path under a [`KNOWN_ROOTS`] entry — comments, the
/// `Name:`/`Version:`/`Libs:`/`Cflags:`/... metadata section, and
/// variable-interpolated values (`${prefix}/lib`) are left untouched.
/// No-op if `path` doesn't have a `.pc` extension.
pub fn patch_one(path: &Path, target_root: &str) -> std::io::Result<()> {
    if !is_pc_file(path) {
        return Ok(());
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        // Not valid UTF-8 (or unreadable) — not a real .pc file, skip.
        return Ok(());
    };
    let mut changed = false;

    let rewritten: Vec<String> = content
        .lines()
        .map(|line| {
            let Some(eq) = line.find('=') else {
                return line.to_string();
            };
            let key = &line[..eq];
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return line.to_string();
            }
            let value = &line[eq + 1..];
            if value.starts_with('$') {
                return line.to_string();
            }
            for root in KNOWN_ROOTS {
                if let Some(rest) = value.strip_prefix(root) {
                    if rest.is_empty() || rest.starts_with('/') {
                        if value != format!("{target_root}{rest}") {
                            changed = true;
                        }
                        return format!("{key}={target_root}{rest}");
                    }
                }
            }
            line.to_string()
        })
        .collect();

    if changed {
        let mut out = rewritten.join("\n");
        if content.ends_with('\n') {
            out.push('\n');
        }
        std::fs::write(path, out)?;
    }
    Ok(())
}
