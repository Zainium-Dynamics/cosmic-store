// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Cosmic Store does not use Flathub statistics.

use std::collections::HashMap;

use crate::app_id::AppId;

/// Monthly download stats — unused for Alpine/Void musl (no Flathub).
pub fn monthly_downloads(_id: &AppId) -> Option<u64> {
    let _: HashMap<AppId, u64> = HashMap::new();
    None
}
