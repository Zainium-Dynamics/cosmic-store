// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// elfpatch.rs — Rewrites ELF `PT_INTERP`/`RPATH` for extracted binaries.
//
// Alpine (`.apk`) and Void (`.xbps`) packages are built against their own
// distro's FHS layout: their binaries carry `PT_INTERP` pointing at
// `/lib/ld-musl-x86_64.so.1` and `RPATH`/`RUNPATH` entries under
// `/usr/lib`. Neither resolves once `extract::extract_archive_to_union`
// has remapped the file into `/overlayer/zexlib/union/`, so without this,
// packages install but their binaries can't actually run.
//
// Most extracted binaries are musl, but not all of them are — a package
// can legitimately bundle a glibc-linked binary (proprietary blobs,
// codecs, etc.) even when it otherwise ships musl content, and Flatpak
// payloads are glibc exclusively (Flathub runtimes never ship a musl
// build). Forcing the musl interpreter onto a glibc binary doesn't
// relocate it, it breaks it — the linker can't resolve glibc's `libc.so.6`
// symbol versioning against musl's libc at all. So the interpreter/RPATH
// target is picked per file, based on what the binary already declares
// it needs, not assumed to always be musl:
//   - musl  → `/overlayer/syshub/<triple>/lib/ld-musl-x86_64.so.1`, RPATH
//             gets both the union lib dir and syshub's own lib dir.
//   - glibc → `/overlayer/zexlib/union/lib/ld-linux-x86-64.so.2` — glibc's
//             interpreter and RPATH both live under the union lib dir,
//             never syshub (syshub is the musl base toolchain only).
//
// Built on `oxipatch`, the same pure-Rust patchelf-equivalent crate `zex`
// uses, at the same pinned revision — not reimplemented here.
//
// Must run on each file immediately after it's written and before the
// caller returns success — `extract_archive_to_union` writes files
// directly onto their live union destination (no separate staging root
// the way zex's installer has one), so there's no later "still mutable"
// window to patch in.

use std::io::Read;
use std::path::Path;

use oxipatch::elf::ElfFile;

/// Which C library a binary was actually built against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Libc {
    Musl,
    Glibc,
}

/// Canonical musl dynamic linker for this OS — same value as zex's own
/// `ZEX_INTERP` (confirmed against a real syshub-toolchain-built binary).
const MUSL_INTERP: &str = "/overlayer/syshub/x86_64-zainium-linux-musl/lib/ld-musl-x86_64.so.1";

/// Canonical glibc dynamic linker location — glibc is installed into the
/// union tree as a regular userland package (like any other store
/// dependency), not into syshub, so its interpreter lives here.
const GLIBC_INTERP: &str = "/overlayer/zexlib/union/lib/ld-linux-x86-64.so.2";

/// Where store-installed packages' shared libraries live — musl app libs
/// and the glibc runtime (loader + `libc.so.6` + friends) both land here.
const UNION_LIB_RPATH: &str = "/overlayer/zexlib/union/lib";

/// Where syshub's own immutable musl-base libraries live. Only relevant to
/// musl binaries — glibc binaries can't use musl's libc anyway, so it's
/// never added to a glibc binary's RPATH.
const SYSHUB_LIB_RPATH: &str = "/overlayer/syshub/lib";

pub fn is_elf(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).is_ok() && magic == [0x7f, b'E', b'L', b'F']
}

fn is_fhs_lib_entry(entry: &str) -> bool {
    entry == "/usr/lib" || entry == "/lib" || entry == "/usr/lib64" || entry == "/lib64"
        || entry.starts_with("/usr/lib/") || entry.starts_with("/lib/")
        || entry.starts_with("/usr/lib64/") || entry.starts_with("/lib64/")
}

/// Decide which C library `f` was actually linked against, from what's
/// still in the file *before* we touch anything:
///   1. Its current `PT_INTERP` string, if it has one — `ld-linux` is
///      always glibc, `ld-musl` is always musl, on every distro.
///   2. Otherwise its `DT_NEEDED` list — glibc's libc soname is always
///      `libc.so.6`; musl's is `libc.so` or `libc.musl-x86_64.so.1`.
///   3. Unrecognized → musl, since that's what every store source (Alpine,
///      Void-musl) ships by default.
fn detect_libc(f: &ElfFile, interp: Option<&str>, needed: &[String]) -> Libc {
    if let Some(interp) = interp {
        if interp.contains("ld-linux") {
            return Libc::Glibc;
        }
        if interp.contains("ld-musl") {
            return Libc::Musl;
        }
    }
    if needed.iter().any(|lib| lib == "libc.so.6") {
        return Libc::Glibc;
    }
    let _ = f; // reserved: only the interp/needed heuristics are used today
    Libc::Musl
}

/// Rewrite RPATH/RUNPATH entries that point at FHS lib dirs to the union
/// lib dir, keep everything else, dedup preserving order, and guarantee
/// every dir in `required` is present even if the binary shipped no RPATH
/// at all.
fn rebuild_rpath(existing: &str, required: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    for entry in existing.split(':').filter(|e| !e.is_empty()) {
        let mapped = if is_fhs_lib_entry(entry) { UNION_LIB_RPATH } else { entry };
        if !out.iter().any(|e| e == mapped) {
            out.push(mapped.to_string());
        }
    }
    for req in required {
        if !out.iter().any(|e| e == req) {
            out.push(req.to_string());
        }
    }
    out.join(":")
}

/// Patch one already-extracted file's interpreter and RPATH in place.
/// No-op (not even opened as ELF) if `path` isn't actually an ELF file.
pub fn patch_one(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let is_regular = path.symlink_metadata().map(|m| m.is_file()).unwrap_or(false);
    if !is_regular || !is_elf(path) {
        return Ok(());
    }

    let mut f = ElfFile::open(path)?;

    let current_interp = f.interpreter()?;
    let has_interp = current_interp.is_some();
    let needed = f.needed_libs()?;
    let has_needed = !needed.is_empty();

    // A file with neither an interpreter nor any DT_NEEDED entries is a
    // fully self-contained static-PIE binary — no dynamic linker ever
    // runs for it, so RPATH is genuinely unused. Skip entirely: zex's own
    // elfpatch.rs found growing the .dynamic section on one of these
    // (Chimera's apk-tools-static-bin) corrupts its self-relocation table
    // and turns a working binary into one that segfaults on every launch.
    if !has_interp && !has_needed {
        return Ok(());
    }

    let libc = detect_libc(&f, current_interp.as_deref(), &needed);
    let (target_interp, required_rpath): (&str, &[&str]) = match libc {
        Libc::Musl => (MUSL_INTERP, &[UNION_LIB_RPATH, SYSHUB_LIB_RPATH]),
        Libc::Glibc => (GLIBC_INTERP, &[UNION_LIB_RPATH]),
    };

    let existing_rpath = f.rpath()?.unwrap_or_default();
    let new_rpath = rebuild_rpath(&existing_rpath, required_rpath);
    let rpath_unchanged = existing_rpath == new_rpath;
    let interp_unchanged = if has_interp {
        current_interp.as_deref() == Some(target_interp)
    } else {
        true
    };

    // Nothing to do — skip the write entirely rather than rewriting the
    // .dynamic section for a no-op change, the same operation category
    // that corrupted a real static-PIE binary's self-relocation table
    // (see the comment above) — matches the identical fix in zex's own
    // `src/cmd/install/elfpatch.rs`.
    if rpath_unchanged && interp_unchanged {
        return Ok(());
    }

    if has_interp && !interp_unchanged {
        f.set_interpreter(target_interp)?;
    }
    if !rpath_unchanged {
        f.set_rpath(&new_rpath, true)?;
    }

    f.commit()?;
    f.write_in_place()?;
    Ok(())
}
