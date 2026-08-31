// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// Void Linux musl packages → /overlayer/zexlib/union
// Does NOT require xbps on the system.

use cosmic::widget;
use std::{
    collections::HashMap,
    error::Error,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{
    Backend, Package,
    extract::{download, extract_archive_to_union},
    install_root::{cache_dir, ensure_layout},
    receipt::{self, ZexReceipt, origin},
};
use crate::app_id::AppId;
use crate::app_info::AppInfo;
use crate::appstream_cache::AppstreamCache;
use crate::operation::{Operation, OperationKind};

const DEFAULT_REPO: &str = "https://repo-default.voidlinux.org/current/musl";
/// Void musl packages use arch id `x86_64-musl` (repodata + .xbps suffix).
const ARCH: &str = "x86_64-musl";
const SOURCE_ID: &str = "void-musl";
const SOURCE_NAME: &str = "Void Linux (musl)";

#[derive(Clone, Debug)]
struct VoidPkg {
    name: String,
    version: String,
    pkgver: String,
    filename: String,
    short_desc: String,
    homepage: String,
    #[allow(dead_code)]
    run_depends: Vec<String>,
    #[allow(dead_code)]
    shasum: String,
}

#[derive(Debug)]
pub struct VoidMusl {
    repo_url: String,
    packages: HashMap<String, VoidPkg>,
    appstream_caches: Vec<AppstreamCache>,
}

impl VoidMusl {
    pub fn new(locale: &str) -> Result<Self, Box<dyn Error>> {
        let repo_url =
            std::env::var("ZAINIUM_VOID_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string());
        Ok(Self {
            repo_url,
            packages: HashMap::new(),
            appstream_caches: vec![AppstreamCache {
                source_id: SOURCE_ID.to_string(),
                source_name: SOURCE_NAME.to_string(),
                locale: locale.to_string(),
                ..Default::default()
            }],
        })
    }

    fn repodata_url(&self) -> String {
        format!("{}/{}-repodata", self.repo_url.trim_end_matches('/'), ARCH)
    }

    fn package_url(&self, filename: &str) -> String {
        format!("{}/{}", self.repo_url.trim_end_matches('/'), filename)
    }

    fn cache_repodata_path(&self) -> PathBuf {
        cache_dir().join(format!("void-{ARCH}-repodata"))
    }

    fn refresh_index(&mut self, force: bool) -> Result<(), Box<dyn Error>> {
        ensure_layout()?;
        let path = self.cache_repodata_path();
        if force || !path.is_file() {
            download(&self.repodata_url(), &path)?;
        }
        self.packages = parse_void_repodata(&path)?;
        log::info!(
            "void-musl: loaded {} packages from {}",
            self.packages.len(),
            path.display()
        );
        self.rebuild_appstream_cache();
        Ok(())
    }

    fn rebuild_appstream_cache(&mut self) {
        let mut infos = HashMap::new();
        let mut pkgnames = HashMap::new();
        for (name, pkg) in &self.packages {
            let id = AppId::new(name);
            let info = Arc::new(AppInfo {
                source_id: SOURCE_ID.to_string(),
                source_name: SOURCE_NAME.to_string(),
                origin_opt: Some(origin::VOID.to_string()),
                name: pkg.name.clone(),
                summary: pkg.short_desc.clone(),
                description: if pkg.homepage.is_empty() {
                    pkg.short_desc.clone()
                } else {
                    format!("{}\n\n{}", pkg.short_desc, pkg.homepage)
                },
                pkgnames: vec![pkg.name.clone()],
                package_paths: vec![pkg.filename.clone()],
                ..Default::default()
            });
            let mut set = std::collections::HashSet::new();
            set.insert(id.clone());
            pkgnames.insert(name.clone(), set);
            infos.insert(id, info);
        }
        self.appstream_caches[0].infos = infos;
        self.appstream_caches[0].pkgnames = pkgnames;
    }

    fn install_one(
        &self,
        name: &str,
        mut progress: impl FnMut(f32),
    ) -> Result<(), Box<dyn Error>> {
        let pkg = self
            .packages
            .get(name)
            .ok_or_else(|| format!("package not in void-musl index: {name}"))?;
        progress(0.05);
        let dest = cache_dir().join(&pkg.filename);
        if !dest.is_file() {
            download(&self.package_url(&pkg.filename), &dest)?;
        }
        progress(0.45);
        let mut receipt = ZexReceipt::new(&pkg.name, &pkg.version, origin::VOID);
        let n = extract_archive_to_union(&dest, &mut receipt)?;
        if n == 0 {
            return Err(format!("void package {name} extracted 0 files").into());
        }
        progress(0.9);
        receipt.save()?;
        progress(1.0);
        log::info!(
            "installed {} {} via void-musl ({} files)",
            pkg.name,
            pkg.version,
            n
        );
        Ok(())
    }

    fn uninstall_one(&self, name: &str) -> Result<(), Box<dyn Error>> {
        ZexReceipt::load(name)?.uninstall()
    }
}

impl Backend for VoidMusl {
    fn load_caches(&mut self, refresh: bool) -> Result<(), Box<dyn Error>> {
        if let Err(err) = self.refresh_index(refresh) {
            log::error!("void-musl index load failed: {err}");
            self.rebuild_appstream_cache();
        }
        Ok(())
    }

    fn info_caches(&self) -> &[AppstreamCache] {
        &self.appstream_caches
    }

    fn installed(&self) -> Result<Vec<Package>, Box<dyn Error>> {
        let mut packages = Vec::new();
        for receipt in receipt::list_installed()? {
            if receipt.metadata.origin != origin::VOID
                && receipt.metadata.origin != "void"
                && receipt.metadata.origin != "tier1-void"
            {
                continue;
            }
            let name = &receipt.metadata.name;
            let id = AppId::new(name);
            let info = self.appstream_caches[0]
                .infos
                .get(&id)
                .cloned()
                .unwrap_or_else(|| {
                    Arc::new(AppInfo {
                        source_id: SOURCE_ID.to_string(),
                        source_name: SOURCE_NAME.to_string(),
                        origin_opt: Some(origin::VOID.to_string()),
                        name: name.clone(),
                        summary: format!("Installed from Void musl ({})", receipt.metadata.version),
                        pkgnames: vec![name.clone()],
                        ..Default::default()
                    })
                });
            packages.push(Package {
                id,
                icon: widget::icon::from_name("package-x-generic")
                    .size(128)
                    .handle(),
                info,
                version: receipt.metadata.version.clone(),
                extra: HashMap::new(),
            });
        }
        Ok(packages)
    }

    fn updates(&self) -> Result<Vec<Package>, Box<dyn Error>> {
        let mut updates = Vec::new();
        for receipt in receipt::list_installed()? {
            if receipt.metadata.origin != origin::VOID && receipt.metadata.origin != "tier1-void" {
                continue;
            }
            let Some(remote) = self.packages.get(&receipt.metadata.name) else {
                continue;
            };
            if remote.version != receipt.metadata.version
                && remote.pkgver != receipt.metadata.version
            {
                let id = AppId::new(&receipt.metadata.name);
                if let Some(info) = self.appstream_caches[0].infos.get(&id) {
                    updates.push(Package {
                        id,
                        icon: widget::icon::from_name("package-x-generic")
                            .size(128)
                            .handle(),
                        info: info.clone(),
                        version: remote.version.clone(),
                        extra: HashMap::new(),
                    });
                }
            }
        }
        Ok(updates)
    }

    fn file_packages(&self, path: &str) -> Result<Vec<Package>, Box<dyn Error>> {
        let p = Path::new(path);
        if !p.is_file() {
            return Err(format!("not a file: {path}").into());
        }
        let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("pkg.xbps");
        let name = fname
            .trim_end_matches(".xbps")
            .rsplit_once('-')
            .map(|(n, _)| n)
            .unwrap_or(fname)
            .to_string();
        Ok(vec![Package {
            id: AppId::new(&name),
            icon: widget::icon::from_name("package-x-generic")
                .size(128)
                .handle(),
            info: Arc::new(AppInfo {
                source_id: SOURCE_ID.to_string(),
                source_name: SOURCE_NAME.to_string(),
                name: name.clone(),
                summary: format!("Local xbps {fname}"),
                pkgnames: vec![name],
                package_paths: vec![path.to_string()],
                ..Default::default()
            }),
            version: String::new(),
            extra: HashMap::from([("local_path".into(), path.to_string())]),
        }])
    }

    fn operation(
        &self,
        op: &Operation,
        mut f: Box<dyn FnMut(f32) + 'static>,
    ) -> Result<(), Box<dyn Error>> {
        match &op.kind {
            OperationKind::Install | OperationKind::Update => {
                let n = op.package_ids.len().max(1) as f32;
                for (i, id) in op.package_ids.iter().enumerate() {
                    let base = i as f32 / n;
                    if let Some(info) = op.infos.get(i) {
                        if let Some(local) = info.package_paths.first() {
                            if local.ends_with(".xbps") && Path::new(local).is_file() {
                                let mut receipt = ZexReceipt::new(id.raw(), "local", origin::VOID);
                                extract_archive_to_union(Path::new(local), &mut receipt)?;
                                receipt.save()?;
                                f(base + 1.0 / n);
                                continue;
                            }
                        }
                    }
                    self.install_one(id.raw(), |p| f(base + p / n))?;
                }
                Ok(())
            }
            OperationKind::Uninstall { .. } => {
                for id in &op.package_ids {
                    self.uninstall_one(id.raw())?;
                }
                f(1.0);
                Ok(())
            }
            OperationKind::RepositoryAdd(_) | OperationKind::RepositoryRemove(_, _) => {
                Err("void-musl: set ZAINIUM_VOID_REPO".into())
            }
        }
    }
}

fn parse_void_repodata(path: &Path) -> Result<HashMap<String, VoidPkg>, Box<dyn Error>> {
    let data = fs::read(path)?;
    let tar_bytes = super::extract::decompress_to_bytes(&data)?;
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    let mut index_bytes = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        if name.ends_with("index.plist") || name == "index.plist" {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            index_bytes = Some(buf);
            break;
        }
    }
    let index_bytes =
        index_bytes.ok_or_else(|| "void repodata: index.plist not found".to_string())?;
    parse_index_plist(&index_bytes)
}

fn parse_index_plist(bytes: &[u8]) -> Result<HashMap<String, VoidPkg>, Box<dyn Error>> {
    let value = plist::Value::from_reader(std::io::Cursor::new(bytes))?;
    let dict = value
        .as_dictionary()
        .ok_or_else(|| "index.plist root is not a dictionary".to_string())?;

    let mut packages = HashMap::new();
    for (name, pkg_val) in dict {
        let Some(pkg_dict) = pkg_val.as_dictionary() else {
            continue;
        };
        let pkgver = plist_string(pkg_dict, "pkgver").unwrap_or_default();
        let version = pkgver
            .strip_prefix(&format!("{name}-"))
            .unwrap_or(&pkgver)
            .to_string();
        let filename = plist_string(pkg_dict, "filename")
            .unwrap_or_else(|| format!("{pkgver}.{ARCH}.xbps"));
        let short_desc = plist_string(pkg_dict, "short_desc").unwrap_or_default();
        let homepage = plist_string(pkg_dict, "homepage").unwrap_or_default();
        let shasum = plist_string(pkg_dict, "shasum").unwrap_or_default();
        let run_depends = plist_string_array(pkg_dict, "run_depends");

        packages.insert(
            name.clone(),
            VoidPkg {
                name: name.clone(),
                version,
                pkgver,
                filename,
                short_desc,
                homepage,
                run_depends,
                shasum,
            },
        );
    }
    Ok(packages)
}

fn plist_string(dict: &plist::Dictionary, key: &str) -> Option<String> {
    dict.get(key).and_then(|v| match v {
        plist::Value::String(s) => Some(s.clone()),
        _ => v.as_string().map(|s| s.to_string()),
    })
}

fn plist_string_array(dict: &plist::Dictionary, key: &str) -> Vec<String> {
    let Some(val) = dict.get(key) else {
        return Vec::new();
    };
    match val {
        plist::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_string().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}
