# Syssur — Conversation & Architecture Handoff

* **Previous Conversation:** [Original Design & Scaffolding Conversation](conversation://c28e7f08-5162-40c5-a0b9-84b53fe55e55)
* **Full Architecture Plan:** [SYSSUR_PLAN.md](../SYSSUR_PLAN.md)

---

## 1. Locked-In Architectural Decisions

| Area | Decision |
| :--- | :--- |
| **Canonical Source of Truth** | **Hybrid:** Central DB (SQLite in Phase 1 behind `ProjectStore` trait, architected for PostgreSQL in Phase 2) + per-file `loro` CRDT live sync, materialized as **pure `.sysml` files** into Git (zero `.syssur-view.json` sidecar files). |
| **Modeling Language & Semantics** | **Native SysML v2** via [`Open-MBEE/sysml-toolkit`](https://github.com/Open-MBEE/sysml-toolkit) (`sysmlv2-syntax` + `sysmlv2-model` pinned at `c8d256b`) wrapped behind `syssur-lang`, including cross-file semantic validation and constraint evaluation (`check_constraints`). |
| **Element Identity** | SysML v2 **Qualified Names** (`ElementId`, e.g., `RoverSystem::Rover::powertrain`), with atomic `RenameElement` in `syssur-cmd` (no synthetic UUID comments in `.sysml`). |
| **Composability Rules** | Native SysML v2 packages, `import`, subsetting (`:>`), redefinition (`:>>`), and port conjugation (`~`)—no `.toml`/lockfile package manager overhead. Separates **Definition Graph** from dynamic **Contextual Instance Tree** (`InstancePath = Vec<ElementId>`), scopes diagrams via `view` / `expose` filters, and visualizes `^` / `:>` / `:>>` origins and mirrored `~` port glyphs. |
| **Deterministic Auto-Layout** | `syssur-layout`: **Canonical AST-span pre-sorter** (byte offset) -> `dagre` compound bounding boxes -> border port pinning -> dedicated **orthogonal A\* channel router** (zero persisted coordinates, 100% deterministic across OS/WASM). |
| **Concurrency & Sync** | One `loro::LoroDoc` per `.sysml` file (`syssur-collab`), multiplexed over `axum` WebSockets (`ewebsock` on client for Native + WASM) using bidirectional `VersionVector` `Hello` handshakes for seamless offline-to-online merge, plus Git snapshot commits (`SyncMessage::CommitSnapshot`). |
| **GUI & Scaling** | Pure Rust `eframe` (`egui` + `wgpu`) + `egui_dock` (VS Code / Cameo-style dockable panels), SolidCraft-inspired top ribbon & bottom command/history bar, SysON-inspired synchronized views, `egui::Painter` with `rstar` R-tree frustum culling + 3-tier LOD rendering, HiDPI auto-scaling + Settings window, and `i18n` (`en` / `fr`). |

---

## 2. Implemented Workspace Status (All Verified)

### Workspace Crates (`/home/ratal/workspace/syssur/crates/`)
* `syssur-model` — `ElementId`, `InstancePath`, `DefinitionGraph`, `InstanceNode` (multiplicity `[n]` & subsetting `:>`), `ViewFilter`, `PortCompatibility`, `ConstraintVerdictItem`.
* `syssur-lang` — `SysmlEngine` wrapping `sysmlv2-syntax` + `sysmlv2-model` (with `ScalarValues` stub), semantic diagnostics, constraint verdicts, and canonical formatting.
* `syssur-layout` — Canonical AST-span pre-sorter, `dagre` compound layout, border port pinning, orthogonal A\* router.
* `syssur-cmd` — Typed `SyssurCommand`s (`CreatePackage`, `CreatePartDef`, `CreatePartUsage`, `CreatePort`, `CreateConnection`, `CreateInterface`, `CreateRequirement`, `LinkSatisfy`, `RenameElement`) + `CommandHistory` undo/redo.
* `syssur-collab` — Per-file `loro` `FileDoc` + `ProjectCollab`, `SyncMessage::{Hello, Delta, CommitSnapshot, CommitResult, Presence}`.
* `syssur-store` — Async `ProjectStore` trait, `SqliteStore` (`sqlx`, ANSI-compatible DDL ready for PostgreSQL), pure `.sysml` `materialize_to_dir`, and `commit_git_snapshot`.
* `syssur-server` — `axum` HTTP/WebSocket room hub with bidirectional `VersionVector` handshake, SQLite persistence, and Git commits (`SYSSUR_DB`, `SYSSUR_GIT_DIR`, `SYSSUR_ADDR`).
* `syssur-app` — Desktop & Browser/WASM GUI (`Trunk.toml` + `index.html`) featuring:
  * Top ribbon (`Scope` from `views.sysml`, `General` / `Interconnection`, `Wire: connect | interface`, quick add, `Connect` / `Disconnect` + auto-reconnect, `Commit Git Snapshot`, `Settings`).
  * Bottom history/command bar (`Undo`, `Redo`, `Cmd:`, `Diagnostics`, `History`).
  * Dockable tabs: **Model Explorer** (Views + Definition Graph + Instance Tree), **SysML v2 Diagram** (`rstar` frustum culling, 3-tier LOD, drag-to-connect port wiring with green/amber/red compatibility checks, right-click block context menu), **`.sysml` Code** (live CRDT editor + severity-colored diagnostics), **Traceability** (requirements, `satisfy` links, live `Satisfied` / `Violated` verdicts).
* `examples/rover-system/` — `power.sysml`, `chassis.sysml`, `vehicle.sysml`, `views.sysml`.

### Last Verified Checks
* `cargo test --workspace`: **12 / 12 tests passed** (0 warnings).
* `cargo check -p syssur-app --target wasm32-unknown-unknown`: **passed**.

---

## 3. Suggested Next Steps
1. **Start Phase 2 (`syssur-mcp` + PostgreSQL + RBAC):** Scaffold the MCP server over `syssur-cmd` and add PostgreSQL `ProjectStore` + OIDC/JWT + package-level `black-box`/`white-box` RBAC.
2. **UI/UX & `syssur-app` Refinements:** Persist user settings (`AppSettings`), enhance `.sysml` syntax highlighting / multi-file cross-package `import` editing, or add `syssur-cli` for CI validation and SysML v2 / RDF export.
3. **Browser WASM Live Run:** Test `trunk serve` on `crates/syssur-app` alongside `syssur-server`.
