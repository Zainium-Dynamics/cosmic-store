// Copyright (c) 2026 Zainiumdynamics. All rights reserved.
// Designed for Zainium OS by Zainiumdynamics — https://zainiumdynamics.tech

use crate::{AppId, backend::BackendName};

/// Source priority — **Alpine musl is default** (highest), then Void musl.
pub fn priority(backend_name: BackendName, source_id: &str, _id: &AppId) -> i32 {
    match backend_name {
        BackendName::AlpineMusl => {
            let mut p = 20;
            if source_id == "alpine-musl" {
                p += 1;
            }
            p
        }
        BackendName::VoidMusl => {
            let mut p = 10;
            if source_id == "void-musl" {
                p += 1;
            }
            p
        }
    }
}
