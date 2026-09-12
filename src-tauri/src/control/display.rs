//! 인증된 로컬 표시 전용 API. 이 모듈은 프로세스/PTY를 만들거나 입력을 주입하지 않는다.

use std::sync::Arc;

use axum::extract::{Json, State};
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::httpapi::{fail, ok};
use crate::types::AgentProfile;

use super::protocol::*;
use super::ControlContext;

const GENERATOR_REVISION: &str = "office-gen-0617d45";
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const MAX_SPRITE_DIMENSION: u32 = 256;
const MIN_SPRITE_DIMENSION: u32 = 16;
const MAX_PORTRAIT_DIMENSION: u32 = 4096;

pub(super) async fn capabilities() -> Json<serde_json::Value> {
    ok(DisplayCapabilities {
        protocol_version: 1,
        features: vec!["characters", "appearance", "portrait", "usage", "focus", "ide-session-focus"],
        generator_revision: GENERATOR_REVISION,
    })
}

pub(super) async fn characters(State(ctx): State<Arc<ControlContext>>) -> Json<serde_json::Value> {
    let characters = ctx
        .store
        .load()
        .agents
        .iter()
        .map(display_character)
        .collect();
    ok(DisplayCharactersResult { characters })
}

pub(super) async fn appearance(
    State(ctx): State<Arc<ControlContext>>,
    Json(params): Json<DisplayAppearanceParams>,
) -> Json<serde_json::Value> {
    let Some(profile) = ctx
        .store
        .load()
        .agents
        .into_iter()
        .find(|p| p.id == params.agent_id)
    else {
        return fail("unknown_agent");
    };
    let revision = appearance_revision(&profile);
    if params.revision != revision {
        return fail("appearance_revision_mismatch");
    }
    let (png_base64, frame_count) = match params.kind {
        DisplayAppearanceKind::Sprite => match ctx.sprite_store.load(&profile.id) {
            Ok(Some(encoded)) if valid_sprite_png(&encoded) => (Some(encoded), 4),
            Ok(Some(_)) | Err(_) => return fail("invalid_sprite"),
            Ok(None) => (None, 4),
        },
        DisplayAppearanceKind::Portrait => match ctx.portrait_store.load(&profile.id) {
            Ok(Some(encoded)) if valid_portrait_png(&encoded) => (Some(encoded), 1),
            Ok(Some(_)) | Err(_) => return fail("invalid_portrait"),
            Ok(None) => (None, 1),
        },
    };
    ok(DisplayAppearanceResult {
        agent_id: profile.id,
        revision,
        png_base64,
        mime_type: "image/png",
        frame_count,
    })
}

pub(super) async fn usage(State(ctx): State<Arc<ControlContext>>) -> Json<serde_json::Value> {
    let snapshot = crate::ipc::commands::load_usage_snapshot_body(
        &ctx.live_usage,
        chrono::Utc::now().timestamp_millis(),
    )
    .await;
    ok(snapshot)
}

pub(super) async fn focus(
    State(ctx): State<Arc<ControlContext>>,
    Json(params): Json<DisplayFocusParams>,
) -> Json<serde_json::Value> {
    if !ctx
        .store
        .load()
        .agents
        .iter()
        .any(|p| p.id == params.agent_id)
    {
        return fail("unknown_agent");
    }
    if let Err(_) = (ctx.focus_agent)(&params.agent_id, params.intent) {
        return fail("focus_unavailable");
    }
    ok(DisplayFocusResult {
        agent_id: params.agent_id,
        focused: true,
    })
}

fn display_character(profile: &AgentProfile) -> DisplayCharacter {
    DisplayCharacter {
        agent_id: profile.id.clone(),
        name: profile.name.clone(),
        role: profile.role.clone(),
        cwd: profile.cwd.clone(),
        revision: appearance_revision(profile),
        seed: profile.seed.clone(),
        archetype: profile.archetype.clone(),
        colors: profile.colors.clone(),
        portrait_updated_at: profile.portrait_updated_at,
        sprite_updated_at: profile.sprite_updated_at,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionSource<'a> {
    seed: &'a str,
    archetype: &'a Option<String>,
    colors: &'a Option<crate::types::ColorOverrides>,
    portrait_updated_at: Option<u64>,
    sprite_updated_at: Option<u64>,
}

fn appearance_revision(profile: &AgentProfile) -> String {
    let source = RevisionSource {
        seed: &profile.seed,
        archetype: &profile.archetype,
        colors: &profile.colors,
        portrait_updated_at: profile.portrait_updated_at,
        sprite_updated_at: profile.sprite_updated_at,
    };
    let bytes = serde_json::to_vec(&source).expect("appearance revision source is serializable");
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_sprite_png(encoded: &str) -> bool {
    let Some((width, height)) = png_ihdr(encoded, crate::persistence::png_store::MAX_SPRITE_BYTES) else {
        return false;
    };
    (MIN_SPRITE_DIMENSION..=MAX_SPRITE_DIMENSION).contains(&height)
        && width == height.saturating_mul(4)
}

fn valid_portrait_png(encoded: &str) -> bool {
    let Some((width, height)) = png_ihdr(encoded, crate::persistence::png_store::MAX_PORTRAIT_BYTES) else {
        return false;
    };
    (1..=MAX_PORTRAIT_DIMENSION).contains(&width)
        && (1..=MAX_PORTRAIT_DIMENSION).contains(&height)
}

fn png_ihdr(encoded: &str, max_bytes: usize) -> Option<(u32, u32)> {
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return None;
    };
    if bytes.len() > max_bytes
        || bytes.len() < 33
        || bytes[..8] != PNG_MAGIC
    {
        return None;
    }
    // PNG의 첫 chunk는 필수 IHDR(길이 13)이고, 폭/높이는 big-endian u32이다.
    if bytes[8..12] != [0, 0, 0, 13] || bytes[12..16] != *b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("four bytes"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("four bytes"));
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_changes_when_sprite_metadata_changes() {
        let mut profile: AgentProfile = serde_json::from_value(serde_json::json!({
            "id": "a", "name": "A", "role": "backend", "seed": "seed",
            "createdAt": 1, "deskIndex": 0
        }))
        .unwrap();
        let first = appearance_revision(&profile);
        profile.sprite_updated_at = Some(42);
        assert_ne!(first, appearance_revision(&profile));
        let first = appearance_revision(&profile);
        profile.portrait_updated_at = Some(43);
        assert_ne!(first, appearance_revision(&profile));
    }

    #[test]
    fn sprite_png_requires_a_four_frame_sheet() {
        let mut bytes = vec![0; 33];
        bytes[..8].copy_from_slice(&PNG_MAGIC);
        bytes[8..12].copy_from_slice(&13u32.to_be_bytes());
        bytes[12..16].copy_from_slice(b"IHDR");
        bytes[16..20].copy_from_slice(&64u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&16u32.to_be_bytes());
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        assert!(valid_sprite_png(&encoded));
    }

    #[test]
    fn portrait_png_requires_nonzero_bounded_dimensions() {
        let mut bytes = vec![0; 33];
        bytes[..8].copy_from_slice(&PNG_MAGIC);
        bytes[8..12].copy_from_slice(&13u32.to_be_bytes());
        bytes[12..16].copy_from_slice(b"IHDR");
        bytes[16..20].copy_from_slice(&240u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&320u32.to_be_bytes());
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        assert!(valid_portrait_png(&encoded));
        bytes[20..24].copy_from_slice(&0u32.to_be_bytes());
        let malformed = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert!(!valid_portrait_png(&malformed));
    }
}
