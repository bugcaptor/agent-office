// src-tauri/src/ipc/commands.rs
//
// The renderer-facing Tauri commands, using the exact invoke names the
// frontend calls. Every command is a thin delegation into
// `SessionManager`/`NotificationHub`/`ProfileStore`/`SettingsStore` — no
// lock is held across an `.await` point. Most bodies have no `.await` at
// all (`async fn` is required by Tauri for commands that take `State`);
// the exceptions (`summarize_text`, `generate_sprite_image`,
// `set_app_settings`) hold no lock when they yield.
//
// The `State<'_, AppState>` parameter is named `app_state` everywhere
// (not `state`) so it never collides with the `state: PersistedState`
// payload parameter on `save_state` -- Tauri's IPC argument binding matches
// JS argument keys to Rust parameter names, so a name collision there would
// silently break `save_state`'s payload mapping.
//
// This file is a parent hub only: the actual command bodies live in
// domain submodules below, split out for readability. Every submodule item
// referenced elsewhere in the crate (Tauri's `generate_handler!` in
// `lib.rs`, `control::settings_set`'s call to `apply_settings_effects`,
// and this module's own `tests` submodule) is re-exported here with
// `pub(crate) use <domain>::*;` so `crate::ipc::commands::<name>` keeps
// resolving exactly as it did before the split -- only the file each
// command's body lives in changed.
mod bot;
mod media;
mod misc;
mod persistence;
mod session;
// pub: contract 테스트(src-tauri/tests/contract_fixtures.rs)가
// `agent_office_lib::ipc::commands::settings::GetAppSettingsResult`에 닿아야
// 한다. 기존 `pub(crate) use settings::*;` 글롭 재수출은 그대로 두고 모듈
// 자체만 승격 — 로직 변경 없음.
pub mod settings;
mod usage;

// Re-imported here (in addition to each domain file's own scoped `use`s)
// purely so `commands/tests.rs`'s `use super::*;` keeps resolving the bare
// names (`State`, `AppState`, `CreateSessionRequest`, `PersistedState`, ...)
// it references -- glob re-exports below only surface each domain
// module's own public items, not the private `use` aliases inside them.
#[cfg(test)]
use tauri::State;
#[cfg(test)]
use crate::state::AppState;
#[cfg(test)]
use crate::types::*;

pub(crate) use bot::*;
pub(crate) use media::*;
pub(crate) use misc::*;
pub(crate) use persistence::*;
pub(crate) use session::*;
pub(crate) use settings::*;
pub(crate) use usage::*;

#[cfg(test)]
mod tests;
