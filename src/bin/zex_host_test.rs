// Host-only integration test for Zainium package backends.
// Uses /tmp — NEVER zairoot.
//
//   cargo run --bin zex_host_test
//
// Copyright (c) 2026 Zainiumdynamics — https://zainiumdynamics.tech

#[path = "../app_id.rs"]
mod app_id;
#[path = "../app_info.rs"]
mod app_info;
#[path = "../appstream_cache.rs"]
mod appstream_cache;
#[path = "../backend/mod.rs"]
mod backend;
#[path = "../gstreamer.rs"]
mod gstreamer;
#[path = "../operation.rs"]
mod operation;
#[path = "../stats.rs"]
mod stats;

use std::{error::Error, fs, path::PathBuf, sync::Arc};

use app_id::AppId;
use app_info::AppInfo;
use backend::{
    Backend, BackendName,
    alpine_musl::AlpineMusl,
    install_root::{cache_dir, ensure_layout, registry_installed_dir, union_root},
    receipt::{ZexReceipt, origin},
    void_musl::VoidMusl,
};
use operation::{Operation, OperationKind};

fn setup_host_env() {
    let base = PathBuf::from("/tmp/zainium-store-host-test");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    unsafe {
        std::env::set_var("ZAINIUM_ZEXLIB_UNION", base.join("union"));
        std::env::set_var(
            "ZAINIUM_ZEXLIB_REGISTRY",
            base.join("registry/installed"),
        );
        std::env::set_var("ZAINIUM_ZEXLIB_CACHE", base.join("cache"));
    }
    println!("HOST TEST ROOT: {}", base.display());
    println!("  union:    {}", union_root().display());
    println!("  registry: {}", registry_installed_dir().display());
    println!("  cache:    {}", cache_dir().display());
}

fn install_via(
    backend: &dyn Backend,
    backend_name: BackendName,
    pkg: &str,
) -> Result<(), Box<dyn Error>> {
    let op = Operation {
        kind: OperationKind::Install,
        backend_name,
        package_ids: vec![AppId::new(pkg)],
        infos: vec![Arc::new(AppInfo {
            name: pkg.to_string(),
            pkgnames: vec![pkg.to_string()],
            ..Default::default()
        })],
    };
    backend.operation(
        &op,
        Box::new(|p| print!("\r  progress {:.0}%   ", p * 100.0)),
    )?;
    println!();
    Ok(())
}

fn count_receipt_files(name: &str) -> Result<usize, Box<dyn Error>> {
    Ok(ZexReceipt::load(name)?.all_files().len())
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    setup_host_env();
    ensure_layout()?;

    let mut failures = Vec::new();

    println!("\n=== [1/2] Alpine Linux musl (DEFAULT) ===");
    match (|| -> Result<(), Box<dyn Error>> {
        let mut alpine = AlpineMusl::new("en")?;
        alpine.load_caches(true)?;
        let ncat = alpine.info_caches()[0].infos.len();
        println!("  catalog packages: {ncat}");
        if ncat == 0 {
            return Err("alpine catalog empty".into());
        }
        let candidates = ["tree", "hello", "lz4", "xz", "file"];
        let mut installed_name = None;
        for c in candidates {
            if alpine.info_caches()[0].infos.contains_key(&AppId::new(c)) {
                println!("  installing {c}…");
                match install_via(&alpine, BackendName::AlpineMusl, c) {
                    Ok(()) => {
                        installed_name = Some(c);
                        break;
                    }
                    Err(e) => println!("  skip {c}: {e}"),
                }
            }
        }
        let name = installed_name.ok_or("no alpine candidate")?;
        let n = count_receipt_files(name)?;
        let r = ZexReceipt::load(name)?;
        assert_eq!(r.metadata.origin, origin::ALPINE);
        println!("  OK {name} files={n} origin={}", r.metadata.origin);
        let op = Operation {
            kind: OperationKind::Uninstall { purge_data: false },
            backend_name: BackendName::AlpineMusl,
            package_ids: vec![AppId::new(name)],
            infos: vec![],
        };
        alpine.operation(&op, Box::new(|_| {}))?;
        println!("  OK uninstalled {name}");
        Ok(())
    })() {
        Ok(()) => println!("ALPINE: PASS"),
        Err(e) => {
            println!("ALPINE: FAIL — {e}");
            failures.push(format!("alpine: {e}"));
        }
    }

    println!("\n=== [2/2] Void Linux musl ===");
    match (|| -> Result<(), Box<dyn Error>> {
        let mut voidb = VoidMusl::new("en")?;
        voidb.load_caches(true)?;
        let ncat = voidb.info_caches()[0].infos.len();
        println!("  catalog packages: {ncat}");
        if ncat == 0 {
            return Err("void catalog empty".into());
        }
        let candidates = ["tree", "lz4", "xz", "which", "file"];
        let mut installed_name = None;
        for c in candidates {
            if voidb.info_caches()[0].infos.contains_key(&AppId::new(c)) {
                println!("  installing {c}…");
                match install_via(&voidb, BackendName::VoidMusl, c) {
                    Ok(()) => {
                        installed_name = Some(c);
                        break;
                    }
                    Err(e) => println!("  skip {c}: {e}"),
                }
            }
        }
        let name = installed_name.ok_or("no void candidate")?;
        let n = count_receipt_files(name)?;
        let r = ZexReceipt::load(name)?;
        assert_eq!(r.metadata.origin, origin::VOID);
        println!("  OK {name} files={n} origin={}", r.metadata.origin);
        let op = Operation {
            kind: OperationKind::Uninstall { purge_data: false },
            backend_name: BackendName::VoidMusl,
            package_ids: vec![AppId::new(name)],
            infos: vec![],
        };
        voidb.operation(&op, Box::new(|_| {}))?;
        println!("  OK uninstalled {name}");
        Ok(())
    })() {
        Ok(()) => println!("VOID: PASS"),
        Err(e) => {
            println!("VOID: FAIL — {e}");
            failures.push(format!("void: {e}"));
        }
    }

    println!("\n========== SUMMARY ==========");
    if failures.is_empty() {
        println!("ALL PASS — Alpine (default) · Void musl (host /tmp only, no zairoot)");
        Ok(())
    } else {
        for f in &failures {
            eprintln!("FAIL: {f}");
        }
        Err(failures.join("; ").into())
    }
}
