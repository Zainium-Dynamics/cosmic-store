// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// ZEX PACKAGE RECEIPT — one TOML file per installed package:
//   /overlayer/zexlib/registry/installed/<name>.toml

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::install_root::{
    classify_union_path, ensure_layout, registry_installed_dir, union_root,
};

/// Origin tier strings used in receipts.
pub mod origin {
    pub const VOID: &str = "tier1-void";
    pub const ALPINE: &str = "tier2-alpine";
    pub const ZEX: &str = "zex";
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ReceiptMetadata {
    pub name: String,
    pub version: String,
    pub origin: String,
    pub install_time: String,
    pub installed_by: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ReceiptFiles {
    #[serde(default)]
    pub bin: Vec<String>,
    #[serde(default)]
    pub lib: Vec<String>,
    #[serde(default)]
    pub sbin: Vec<String>,
    #[serde(default)]
    pub etc: Vec<String>,
    #[serde(default)]
    pub share: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ReceiptServices {
    #[serde(default)]
    pub daemons: Vec<String>,
}

/// Full ZEX PACKAGE RECEIPT document.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ZexReceipt {
    pub metadata: ReceiptMetadata,
    pub files: ReceiptFiles,
    #[serde(default)]
    pub services: ReceiptServices,
}

impl ZexReceipt {
    pub fn new(name: impl Into<String>, version: impl Into<String>, origin: impl Into<String>) -> Self {
        Self {
            metadata: ReceiptMetadata {
                name: name.into(),
                version: version.into(),
                origin: origin.into(),
                install_time: now_rfc3339(),
                installed_by: whoami(),
            },
            files: ReceiptFiles::default(),
            services: ReceiptServices::default(),
        }
    }

    pub fn path_for(name: &str) -> PathBuf {
        registry_installed_dir().join(format!("{name}.toml"))
    }

    pub fn load(name: &str) -> Result<Self, Box<dyn Error>> {
        let path = Self::path_for(name);
        let text = fs::read_to_string(&path)?;
        let receipt: Self = toml::from_str(&text)?;
        Ok(receipt)
    }

    pub fn load_path(path: &Path) -> Result<Self, Box<dyn Error>> {
        let text = fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self) -> Result<PathBuf, Box<dyn Error>> {
        ensure_layout()?;
        let path = Self::path_for(&self.metadata.name);
        let body = format!(
            "# ZEX PACKAGE RECEIPT\n\
             # Package: {}\n\
             # Path: {}\n\
             # Copyright (c) 2026 Zainiumdynamics. All rights reserved.\n\
             # Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech\n\n{}",
            self.metadata.name,
            path.display(),
            toml::to_string_pretty(self)?
        );
        // Atomic-ish write
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, body)?;
        fs::rename(&tmp, &path)?;
        Ok(path)
    }

    pub fn add_union_file(&mut self, abs: &Path) {
        let s = abs.to_string_lossy().into_owned();
        match classify_union_path(abs).unwrap_or("share") {
            "bin" => {
                if !self.files.bin.contains(&s) {
                    self.files.bin.push(s);
                }
            }
            "sbin" => {
                if !self.files.sbin.contains(&s) {
                    self.files.sbin.push(s);
                }
            }
            "lib" => {
                if !self.files.lib.contains(&s) {
                    self.files.lib.push(s);
                }
            }
            "etc" => {
                if !self.files.etc.contains(&s) {
                    self.files.etc.push(s);
                }
            }
            _ => {
                if !self.files.share.contains(&s) {
                    self.files.share.push(s);
                }
            }
        }
    }

    /// All absolute paths owned by this package.
    pub fn all_files(&self) -> Vec<&str> {
        let mut v = Vec::new();
        v.extend(self.files.bin.iter().map(String::as_str));
        v.extend(self.files.lib.iter().map(String::as_str));
        v.extend(self.files.sbin.iter().map(String::as_str));
        v.extend(self.files.etc.iter().map(String::as_str));
        v.extend(self.files.share.iter().map(String::as_str));
        v
    }

    /// Uninstall: delete listed files then remove receipt.
    pub fn uninstall(&self) -> Result<(), Box<dyn Error>> {
        for p in self.all_files() {
            let path = Path::new(p);
            if path.is_file() || path.is_symlink() {
                if let Err(err) = fs::remove_file(path) {
                    log::warn!("failed to remove {}: {}", p, err);
                }
            }
        }
        // Best-effort: remove empty parent dirs under union (not union root itself)
        for p in self.all_files() {
            let mut parent = Path::new(p).parent();
            while let Some(dir) = parent {
                if dir == union_root() {
                    break;
                }
                if fs::remove_dir(dir).is_err() {
                    break;
                }
                parent = dir.parent();
            }
        }
        let receipt_path = Self::path_for(&self.metadata.name);
        if receipt_path.exists() {
            fs::remove_file(receipt_path)?;
        }
        Ok(())
    }
}

/// List all installed receipts.
pub fn list_installed() -> Result<Vec<ZexReceipt>, Box<dyn Error>> {
    let dir = registry_installed_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        match ZexReceipt::load_path(&path) {
            Ok(r) => out.push(r),
            Err(err) => log::warn!("skip broken receipt {:?}: {}", path, err),
        }
    }
    out.sort_by(|a, b| a.metadata.name.cmp(&b.metadata.name));
    Ok(out)
}

/// All files claimed by any installed receipt (for conflict checks).
pub fn all_owned_files() -> Result<BTreeSet<String>, Box<dyn Error>> {
    let mut set = BTreeSet::new();
    for r in list_installed()? {
        for f in r.all_files() {
            set.insert(f.to_string());
        }
    }
    Ok(set)
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Minimal RFC3339 UTC without chrono dependency on formatting edge cases
    // Prefer chrono if available in crate
    chrono_now().unwrap_or_else(|| format!("{secs}"))
}

fn chrono_now() -> Option<String> {
    use chrono::SecondsFormat;
    Some(chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "root".into())
}
