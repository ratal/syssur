# Syssur

**Syssur** is a multi-platform (Native Desktop + Browser/WASM) **Model-Based Systems Engineering (MBSE)** platform written in **Rust**, using **SysML v2** as a human- and LLM-readable *Everything-as-Code* foundation.

See [SYSSUR_PLAN.md](SYSSUR_PLAN.md) for the full architecture and phased roadmap.

## Workspace Crates

- `crates/syssur-model` — Core domain types (`ElementId`, `InstancePath`, `DefinitionGraph`, `InstanceTree`, `ViewFilter`, `TraceabilityGraph`)
- `crates/syssur-lang` — Adapter over `Open-MBEE/sysml-toolkit` (`sysmlv2-syntax`) for parsing, diagnostics, and span-anchored extraction
- `crates/syssur-layout` — 100% deterministic `dagre` layout (canonical AST-span pre-sorter) + border port pinning + orthogonal wire router
- `crates/syssur-cmd` — Unified command engine (`SyssurCommand`, span-anchored `.sysml` text edits, undo/redo history)
- `crates/syssur-collab` — Per-file `loro` CRDT collaboration and presence
- `crates/syssur-store` — Async `ProjectStore` trait with SQLite (`sqlx`) implementation (PostgreSQL-ready schema) and pure `.sysml` Git materializer
- `crates/syssur-server` — `axum` HTTP/WebSocket live sync hub
- `crates/syssur-app` — Pure Rust `egui` + `wgpu` (`eframe` + `egui_dock` + `rstar`) desktop & WASM graphical editor

## Quick Start

```bash
cargo test --workspace
cargo run -p syssur-app
```

## License

Syssur is dual-licensed under either:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
