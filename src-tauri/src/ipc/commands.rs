// src-tauri/src/ipc/commands.rs
//
// The renderer-facing Tauri commands, using the exact invoke names the
// frontend calls. Every command is a thin delegation into
// `SessionManager`/`NotificationHub`/`ProfileStore`/`SettingsStore` — no
// lock is held across an `.await` point. Most bodies have no `.await` at
// all (`async fn` is required by Tauri for commands that take `State`);
// the exceptions (`summarize_text`, `generate_codex_image`,
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
// 웹 원격 — 페어링 승인·클라이언트 관리·화면 스냅샷 응답.
mod web_remote;
// tailscale serve 대행(웹 원격 HTTPS). 상태 정본은 tailscaled다.
mod tailscale;
mod persistence;
mod session;
// pub: contract 테스트(src-tauri/tests/contract_fixtures.rs)가
// `agent_office_lib::ipc::commands::settings::GetAppSettingsResult`에 닿아야
// 한다. 기존 `pub(crate) use settings::*;` 글롭 재수출은 그대로 두고 모듈
// 자체만 승격 — 로직 변경 없음.
pub mod settings;
// 확인 요청 대사 TTS(리라이트+합성). 외부 API 두 곳을 호출하는 유일한 도메인.
mod tts;
// 동료 대화 — 상태 스냅샷·감사 로그 열람.
mod talk;
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
pub(crate) use web_remote::*;
pub(crate) use tailscale::*;
pub(crate) use persistence::*;
pub(crate) use session::*;
pub(crate) use settings::*;
pub(crate) use talk::*;
pub(crate) use tts::*;
pub(crate) use usage::*;

#[cfg(test)]
mod tests;
