// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// Alpine Linux musl packages → /overlayer/zexlib/union
// Does NOT require apk on the system.

use cosmic::widget;
use std::{
    collections::HashMap,
    error::Error,
    fs,
    io::Read,
    path::Path,
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

/// Default: Alpine latest stable main + community (musl x86_64).
const DEFAULT_BRANCH: &str = "v3.21";
const ARCH: &str = "x86_64";
const SOURCE_ID: &str = "alpine-musl";
const SOURCE_NAME: &str = "Alpine Linux (musl)";

#[derive(Clone, Debug)]
struct AlpinePkg {
    name: String,
    version: String,
    description: String,
    url: String,
    depends: Vec<String>,
    /// Repository relative path e.g. main or community
    repo: String,
    size: u64,
}

#[derive(Debug)]
pub struct AlpineMusl {
    mirror: String,
    branch: String,
    packages: HashMap<String, AlpinePkg>,
    appstream_caches: Vec<AppstreamCache>,
}

impl AlpineMusl {
    pub fn new(locale: &str) -> Result<Self, Box<dyn Error>> {
        let mirror = std::env::var("ZAINIUM_ALPINE_MIRROR")
            .unwrap_or_else(|_| "https://dl-cdn.alpinelinux.org/alpine".into());
        let branch =
            std::env::var("ZAINIUM_ALPINE_BRANCH").unwrap_or_else(|_| DEFAULT_BRANCH.into());
        Ok(Self {
            mirror,
            branch,
            packages: HashMap::new(),
            appstream_caches: vec![AppstreamCache {
                source_id: SOURCE_ID.to_string(),
                source_name: SOURCE_NAME.to_string(),
                locale: locale.to_string(),
                ..Default::default()
            }],
        })
    }

    fn repos(&self) -> &[&str] {
        &["main", "community"]
    }

    fn index_url(&self, repo: &str) -> String {
        format!(
            "{}/{}/{}/{}/APKINDEX.tar.gz",
            self.mirror.trim_end_matches('/'),
            self.branch,
            repo,
            ARCH
        )
    }

    fn package_url(&self, pkg: &AlpinePkg) -> String {
        format!(
            "{}/{}/{}/{}/{}-{}.apk",
            self.mirror.trim_end_matches('/'),
            self.branch,
            pkg.repo,
            ARCH,
            pkg.name,
            pkg.version
        )
    }

    fn refresh_index(&mut self, force: bool) -> Result<(), Box<dyn Error>> {
        ensure_layout()?;
        let mut all = HashMap::new();
        for repo in self.repos() {
            let cache_path = cache_dir().join(format!("alpine-{}-{}-APKINDEX.tar.gz", self.branch, repo));
            if force || !cache_path.is_file() {
                download(&self.index_url(repo), &cache_path)?;
            }
            let pkgs = parse_apkindex_tar(&cache_path, repo)?;
            log::info!("alpine-musl: {} packages from {repo}", pkgs.len());
            for (k, v) in pkgs {
                all.insert(k, v);
            }
        }
        self.packages = all;
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
                origin_opt: Some(origin::ALPINE.to_string()),
                name: pkg.name.clone(),
                summary: pkg.description.clone(),
                description: if pkg.url.is_empty() {
                    pkg.description.clone()
                } else {
                    format!("{}\n\n{}", pkg.description, pkg.url)
                },
                pkgnames: vec![pkg.name.clone()],
                package_paths: vec![format!("{}-{}.apk", pkg.name, pkg.version)],
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
            .ok_or_else(|| format!("package not in alpine-musl index: {name}"))?;
        progress(0.05);
        let dest = cache_dir().join(format!("{}-{}.apk", pkg.name, pkg.version));
        if !dest.is_file() {
            download(&self.package_url(pkg), &dest)?;
        }
        progress(0.45);
        let mut receipt = ZexReceipt::new(&pkg.name, &pkg.version, origin::ALPINE);
        let n = extract_archive_to_union(&dest, &mut receipt)?;
        if n == 0 {
            return Err(format!("alpine package {name} extracted 0 files").into());
        }
        progress(0.9);
        receipt.save()?;
        progress(1.0);
        log::info!(
            "installed {} {} via alpine-musl ({} files)",
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

impl Backend for AlpineMusl {
    fn load_caches(&mut self, refresh: bool) -> Result<(), Box<dyn Error>> {
        if let Err(err) = self.refresh_index(refresh) {
            log::error!("alpine-musl index load failed: {err}");
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
            if receipt.metadata.origin != origin::ALPINE
                && receipt.metadata.origin != "tier2-alpine"
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
                        origin_opt: Some(origin::ALPINE.to_string()),
                        name: name.clone(),
                        summary: format!("Installed from Alpine ({})", receipt.metadata.version),
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
            if receipt.metadata.origin != origin::ALPINE
                && receipt.metadata.origin != "tier2-alpine"
            {
                continue;
            }
            let Some(remote) = self.packages.get(&receipt.metadata.name) else {
                continue;
            };
            if remote.version != receipt.metadata.version {
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
        // Allow installing a local .apk file
        let p = Path::new(path);
        if !p.is_file() {
            return Err(format!("not a file: {path}").into());
        }
        let fname = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("local.apk");
        let name = fname
            .trim_end_matches(".apk")
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
                summary: format!("Local apk {fname}"),
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
                    // Local path override via package_paths
                    if let Some(info) = op.infos.get(i) {
                        if let Some(local) = info.package_paths.first() {
                            if local.ends_with(".apk") && Path::new(local).is_file() {
                                let mut receipt =
                                    ZexReceipt::new(id.raw(), "local", origin::ALPINE);
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
                Err("alpine-musl: set ZAINIUM_ALPINE_MIRROR / ZAINIUM_ALPINE_BRANCH".into())
            }
        }
    }
}

/// Parse APKINDEX.tar.gz → package map.
fn parse_apkindex_tar(path: &Path, repo: &str) -> Result<HashMap<String, AlpinePkg>, Box<dyn Error>> {
    let data = fs::read(path)?;
    // Prefer streaming gzip→tar when file is .tar.gz
    // Alpine uses multi-member gzip; MultiGzDecoder is required.
    let index_text = if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        let dec = flate2::read::MultiGzDecoder::new(std::io::Cursor::new(&data));
        let mut archive = tar::Archive::new(dec);
        read_apkindex_from_tar(&mut archive)?
    } else {
        let tar_bytes = super::extract::decompress_to_bytes(&data)?;
        let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
        read_apkindex_from_tar(&mut archive)?
    };
    if index_text.is_empty() {
        return Err("APKINDEX not found in archive".into());
    }
    Ok(parse_apkindex_text(&index_text, repo))
}

fn read_apkindex_from_tar<R: Read>(
    archive: &mut tar::Archive<R>,
) -> Result<String, Box<dyn Error>> {
    let mut names = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        names.push(name.clone());
        let base = name.rsplit('/').next().unwrap_or(&name);
        if base == "APKINDEX" {
            let mut index_text = String::new();
            entry.read_to_string(&mut index_text)?;
            return Ok(index_text);
        }
    }
    Err(format!("APKINDEX not found; members={names:?}").into())
}

fn parse_apkindex_text(text: &str, repo: &str) -> HashMap<String, AlpinePkg> {
    let mut packages = HashMap::new();
    let mut cur: HashMap<char, String> = HashMap::new();
    let flush = |cur: &mut HashMap<char, String>, packages: &mut HashMap<String, AlpinePkg>| {
        let name = cur.get(&'P').cloned().unwrap_or_default();
        if name.is_empty() {
            cur.clear();
            return;
        }
        let version = cur.get(&'V').cloned().unwrap_or_default();
        let description = cur.get(&'T').cloned().unwrap_or_default();
        let url = cur.get(&'U').cloned().unwrap_or_default();
        let depends = cur
            .get(&'D')
            .map(|d| {
                d.split_whitespace()
                    .map(|s| s.split(['<', '>', '=', '~']).next().unwrap_or(s).to_string())
                    .filter(|s| !s.is_empty() && !s.starts_with('!'))
                    .collect()
            })
            .unwrap_or_default();
        let size = cur
            .get(&'S')
            .and_then(|s| s.parse().ok())
            .unwrap_or(0u64);
        packages.insert(
            name.clone(),
            AlpinePkg {
                name,
                version,
                description,
                url,
                depends,
                repo: repo.to_string(),
                size,
            },
        );
        cur.clear();
    };

    for line in text.lines() {
        if line.is_empty() {
            flush(&mut cur, &mut packages);
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            if let Some(c) = k.chars().next() {
                if k.len() == 1 {
                    cur.insert(c, v.to_string());
                }
            }
        }
    }
    flush(&mut cur, &mut packages);
    packages
}
