# Cosmic Store

**Designed for Zainium OS by Zainiumdynamics**  
Website: https://zainiumdynamics.tech

Copyright (c) 2026 Zainiumdynamics. All rights reserved.

App store for Zainium OS.

## Sources (musl only)

| Priority | Source |
|----------|--------|
| **Default** | **Alpine Linux musl** |
| 2 | Void Linux musl |

No Flatpak · no KISS · no PackageKit. Packages install into zexlib (not system PMs).

## Paths

| What | Path |
|------|------|
| Store binary | `/overlayer/syshub/bin/cosmic-store` |
| App packages | `/overlayer/zexlib/union/` |
| Receipts | `/overlayer/zexlib/registry/installed/<name>.toml` |

App ID: `org.cosmic.Store`

## Build / test

```bash
cargo build --release --bin cosmic-store
cargo run --bin zex_host_test   # host only, /tmp — not zairoot
just status
```
