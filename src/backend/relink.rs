// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// Repairs already-installed union packages whose ELF interpreter/RPATH or
// script shebangs predate a patcher fix (or were written by an earlier
// version of one). Walks the union tree and re-applies elfpatch,
// pcpatch, and shebangpatch to every file already there — the same three
// calls `extract.rs` makes on every newly-extracted file, just backfilled
// onto what's already installed. Invoked headlessly via
// `cosmic-store relink-union` — one store binary, no separate tool.

use std::path::Path;

/// (files_scanned, elf_relinked, pc_rewritten, scripts_checked)
pub type RelinkCounts = (usize, usize, usize, usize);

pub fn relink_union(union_root: &Path) -> RelinkCounts {
    let mut counts = (0, 0, 0, 0);
    walk(union_root, &union_root.to_string_lossy(), &mut counts);
    counts
}

fn walk(dir: &Path, union_root: &str, counts: &mut RelinkCounts) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            log::warn!("relink: skip {}: {err}", dir.display());
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_symlink() {
            continue;
        } else if file_type.is_dir() {
            walk(&path, union_root, counts);
        } else if file_type.is_file() {
            // Matches extract.rs: all three patchers run unconditionally on
            // every regular file — each no-ops on anything that isn't
            // actually its file type (ELF / `.pc` / `#!`-script).
            counts.0 += 1;

            let was_elf = super::elfpatch::is_elf(&path);
            match super::elfpatch::patch_one(&path) {
                Ok(()) if was_elf => counts.1 += 1,
                Ok(()) => {}
                Err(err) => log::warn!("relink: elfpatch FAILED {}: {err}", path.display()),
            }

            let was_pc = super::pcpatch::is_pc_file(&path);
            match super::pcpatch::patch_one(&path, union_root) {
                Ok(()) if was_pc => counts.2 += 1,
                Ok(()) => {}
                Err(err) => log::warn!("relink: pcpatch FAILED {}: {err}", path.display()),
            }

            if let Err(err) = super::shebangpatch::patch_one(&path) {
                log::warn!("relink: shebangpatch FAILED {}: {err}", path.display());
            } else {
                counts.3 += 1;
            }
        }
    }
}
