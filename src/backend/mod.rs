// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech
//
// Zainium App Store backends (musl only):
//   Alpine Linux musl (DEFAULT) · Void Linux musl
// Removed: Flatpak · PackageKit · KISS · Redox pkgar · rpm-ostree
// Install: /overlayer/zexlib/union  |  Receipts: registry/installed/*.toml
// No system package managers (xbps/apk) on the OS.

use cosmic::widget;
use futures::StreamExt;
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    fmt,
    sync::Arc,
    time::Instant,
};

use crate::app_id::AppId;
use crate::app_info::AppInfo;
use crate::appstream_cache::AppstreamCache;
use crate::gstreamer::GStreamerCodec;
use crate::operation::Operation;

pub mod alpine_musl;
pub mod elfpatch;
pub mod extract;
pub mod install_root;
pub mod pcpatch;
pub mod receipt;
pub mod relink;
pub mod shebangpatch;
pub mod void_musl;

/// Available backend types — Alpine is default (highest priority).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BackendName {
    /// DEFAULT — Alpine Linux musl
    AlpineMusl,
    /// Void Linux musl
    VoidMusl,
}

impl BackendName {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendName::AlpineMusl => "alpine-musl",
            BackendName::VoidMusl => "void-musl",
        }
    }

    pub fn origin_tier(&self) -> &'static str {
        match self {
            BackendName::AlpineMusl => receipt::origin::ALPINE,
            BackendName::VoidMusl => receipt::origin::VOID,
        }
    }

    pub fn is_default(&self) -> bool {
        matches!(self, BackendName::AlpineMusl)
    }
}

impl fmt::Display for BackendName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for BackendName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "alpine-musl" | "alpine" | "tier2-alpine" | "tier1-alpine" => {
                Ok(BackendName::AlpineMusl)
            }
            "void-musl" | "void" | "tier1-void" | "tier2-void" => Ok(BackendName::VoidMusl),
            "kiss" | "tier3-kiss" | "flatpak-user" | "flatpak-system" | "packagekit"
            | "pkgar" | "rpm-ostree" => Err(format!("removed backend: {s}")),
            _ => Err(format!("unknown backend name: {s}")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Package {
    pub id: AppId,
    pub icon: widget::icon::Handle,
    pub info: Arc<AppInfo>,
    pub version: String,
    pub extra: HashMap<String, String>,
}

pub trait Backend: fmt::Debug + Send + Sync {
    fn load_caches(&mut self, refresh: bool) -> Result<(), Box<dyn Error>>;
    fn info_caches(&self) -> &[AppstreamCache];
    fn installed(&self) -> Result<Vec<Package>, Box<dyn Error>>;
    fn updates(&self) -> Result<Vec<Package>, Box<dyn Error>>;
    fn file_packages(&self, path: &str) -> Result<Vec<Package>, Box<dyn Error>>;
    fn gstreamer_packages(
        &self,
        _gstreamer_codec: &GStreamerCodec,
    ) -> Result<Vec<Package>, Box<dyn Error>> {
        Ok(Vec::new())
    }
    fn operation(
        &self,
        op: &Operation,
        f: Box<dyn FnMut(f32) + 'static>,
    ) -> Result<(), Box<dyn Error>>;
}

pub type Backends = BTreeMap<BackendName, Arc<dyn Backend>>;

fn push_backend<F>(
    backends: &mut futures::stream::FuturesUnordered<
        tokio::sync::oneshot::Receiver<Option<(BackendName, Arc<dyn Backend>)>>,
    >,
    name: BackendName,
    locale: String,
    f: F,
) where
    F: FnOnce(&str) -> Result<Arc<dyn Backend>, Box<dyn Error>> + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let start = Instant::now();
        log::info!("adding {name} backend");
        _ = tx.send(match f(&locale) {
            Ok(backend) => {
                log::warn!("initialized {name} backend in {:?}", start.elapsed());
                Some((name, backend))
            }
            Err(err) => {
                log::error!("failed to load {name} backend: {err}");
                None
            }
        });
    });
    backends.push(rx);
}

/// Load backends — Alpine first (default), then Void. Both musl-only.
pub fn backends<'a>(
    locale: &'a str,
    refresh: bool,
) -> impl futures::Stream<Item = (BackendName, Arc<dyn Backend>)> + Send + Unpin + 'static {
    let mut backends = futures::stream::FuturesUnordered::new();
    let locale = locale.to_owned();

    // DEFAULT
    push_backend(
        &mut backends,
        BackendName::AlpineMusl,
        locale.clone(),
        |loc| Ok(Arc::new(alpine_musl::AlpineMusl::new(loc)?) as Arc<dyn Backend>),
    );
    push_backend(
        &mut backends,
        BackendName::VoidMusl,
        locale,
        |loc| Ok(Arc::new(void_musl::VoidMusl::new(loc)?) as Arc<dyn Backend>),
    );

    backends
        .map(move |value| async move {
            let (backend_name, mut backend) = value.ok()??;
            let start = Instant::now();
            let (tx, rx) = tokio::sync::oneshot::channel();
            tokio::task::spawn_blocking(move || {
                match Arc::get_mut(&mut backend).unwrap().load_caches(refresh) {
                    Ok(()) => {
                        log::info!(
                            "loaded {} backend caches in {:?}",
                            backend_name,
                            start.elapsed()
                        );
                    }
                    Err(err) => {
                        log::error!("failed to load {} backend caches: {}", backend_name, err);
                    }
                }
                _ = tx.send(backend);
            });
            Some((backend_name, rx.await.unwrap()))
        })
        .buffer_unordered(4)
        .chain(futures::stream::once(async {
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            {
                let start = Instant::now();
                unsafe {
                    libc::malloc_trim(0);
                }
                log::info!("trimmed allocations in {:?}", start.elapsed());
            }
            None
        }))
        .filter_map(|result| async move { result })
        .boxed()
}
