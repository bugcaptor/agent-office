//! Claude Code output-style preparation for a verified, observed VS Code session.
//! This module never changes IDE settings or transcripts; it only creates our own
//! deterministic style file for the user to select in Claude Code.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::session::manager::SessionManager;
use crate::types::AgentProfile;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IdePersonaContext {
    pub session_id: String,
    pub provider: String,
    pub source_session_id: String,
    pub cwd: String,
    pub personality_prompt: String,
    pub style: Option<IdePersonaStyle>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IdePersonaStyle {
    pub name: String,
    pub path: String,
    pub content: String,
    pub exists: bool,
}

pub(crate) fn get(
    manager: &SessionManager,
    profile: &AgentProfile,
) -> Result<Option<IdePersonaContext>, String> {
    get_with_config_root(manager, profile, None)
}

fn get_with_config_root(
    manager: &SessionManager,
    profile: &AgentProfile,
    config_root: Option<&Path>,
) -> Result<Option<IdePersonaContext>, String> {
    let Some(connection) = manager.observed_persona_connection(&profile.id) else {
        return Ok(None);
    };
    if !connection.verified_vscode {
        return Ok(None);
    }
    if profile
        .cwd
        .as_deref()
        .filter(|cwd| !cwd.is_empty())
        .is_none_or(|cwd| !super::same_directory(cwd, &connection.cwd))
    {
        return Err("ide-persona-stale-cwd".into());
    }
    let prompt = profile.personality_prompt.clone().unwrap_or_default();
    let style = match connection.provider.as_str() {
        "claude" => Some(style_for(profile, &prompt, config_root)?),
        "codex" => None,
        _ => return Ok(None),
    };
    Ok(Some(IdePersonaContext {
        session_id: connection.session_id,
        provider: connection.provider,
        source_session_id: connection.source_session_id,
        cwd: connection.cwd,
        personality_prompt: prompt,
        style,
    }))
}

pub(crate) fn prepare(
    manager: &SessionManager,
    profile: &AgentProfile,
    session_id: &str,
    expected_prompt: &str,
) -> Result<IdePersonaContext, String> {
    prepare_with_config_root(manager, profile, session_id, expected_prompt, None)
}

fn prepare_with_config_root(
    manager: &SessionManager,
    profile: &AgentProfile,
    session_id: &str,
    expected_prompt: &str,
    config_root: Option<&Path>,
) -> Result<IdePersonaContext, String> {
    let context =
        get_with_config_root(manager, profile, config_root)?.ok_or("ide-persona-not-connected")?;
    if context.session_id != session_id {
        return Err("ide-persona-session-stale".into());
    }
    if context.personality_prompt != expected_prompt {
        return Err("ide-persona-prompt-stale".into());
    }
    if context.personality_prompt.trim().is_empty() {
        return Err("ide-persona-prompt-empty".into());
    }
    if context.provider != "claude" {
        return Err("ide-persona-provider-unsupported".into());
    }
    let style = context
        .style
        .as_ref()
        .ok_or("ide-persona-provider-unsupported")?;
    // Recheck immediately before the only mutation. The manager lock is held
    // only while taking this snapshot, never while filesystem I/O runs.
    let current =
        get_with_config_root(manager, profile, config_root)?.ok_or("ide-persona-not-connected")?;
    if current.session_id != session_id {
        return Err("ide-persona-session-stale".into());
    }
    if current.personality_prompt != expected_prompt {
        return Err("ide-persona-prompt-stale".into());
    }
    write_style(style)?;
    Ok(IdePersonaContext {
        style: Some(IdePersonaStyle {
            exists: true,
            ..style.clone()
        }),
        ..context
    })
}

fn style_for(
    profile: &AgentProfile,
    prompt: &str,
    config_root: Option<&Path>,
) -> Result<IdePersonaStyle, String> {
    validate(profile, prompt)?;
    let mut h = Sha256::new();
    h.update(profile.id.as_bytes());
    h.update([0]);
    h.update(profile.name.as_bytes());
    h.update([0]);
    h.update(prompt.as_bytes());
    let digest = format!("{:x}", h.finalize());
    let name = format!("Agent Office — {} ({})", profile.name.trim(), &digest[..12]);
    let content = format!(
        "---\nname: {}\ndescription: {}\nkeep-coding-instructions: true\n---\n\n{}\n\nMaintain this character's requested speaking style in your responses. Preserve all coding, safety, project, and user instructions; do not weaken, replace, or reinterpret them.\n",
        serde_json::to_string(&name).map_err(|_| "ide-persona-invalid")?,
        serde_json::to_string("Agent Office character voice. Keeps Claude Code coding instructions.").map_err(|_| "ide-persona-invalid")?,
        prompt.trim(),
    );
    let root = config_root
        .map(Path::to_path_buf)
        .or_else(crate::agent_paths::claude_config_dir_from_env)
        .ok_or("ide-persona-home-unavailable")?;
    let path = root.join("output-styles").join(format!(
        "agent-office-{}-{}.md",
        safe_id(&profile.id),
        &digest[..16]
    ));
    let exists = existing_style_matches(&path, &content)?;
    Ok(IdePersonaStyle {
        name,
        path: path.to_string_lossy().into_owned(),
        content,
        exists,
    })
}

fn write_style(style: &IdePersonaStyle) -> Result<(), String> {
    let path = Path::new(&style.path);
    let parent = path.parent().ok_or("ide-persona-path-invalid")?;
    validate_style_directory(parent, true)?;
    if existing_style_matches(path, &style.content)? {
        return Ok(());
    }
    if fs::symlink_metadata(path).is_ok() {
        return Err("ide-persona-style-conflict".into());
    }
    let temp = temporary_path(parent, path.file_name().ok_or("ide-persona-path-invalid")?);
    let write_result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| "ide-persona-style-create-failed")?;
        file.write_all(style.content.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| "ide-persona-style-write-failed".to_string())?;
        // hard_link is create-new at the destination: it cannot replace a user file.
        fs::hard_link(&temp, path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "ide-persona-style-conflict".to_string()
            } else {
                "ide-persona-style-create-failed".to_string()
            }
        })?;
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    write_result
}

fn validate_style_directory(parent: &Path, create: bool) -> Result<(), String> {
    let root = parent.parent().ok_or("ide-persona-path-invalid")?;
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("ide-persona-path-symlink".into());
    }
    if create {
        fs::create_dir_all(parent).map_err(|_| "ide-persona-style-dir-failed")?;
    }
    match fs::symlink_metadata(parent) {
        Ok(meta) if meta.file_type().is_symlink() => Err("ide-persona-path-symlink".into()),
        Ok(meta) if !meta.file_type().is_dir() => Err("ide-persona-style-conflict".into()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("ide-persona-style-read-failed".into()),
    }
}

fn existing_style_matches(path: &Path, content: &str) -> Result<bool, String> {
    let parent = path.parent().ok_or("ide-persona-path-invalid")?;
    validate_style_directory(parent, false)?;
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => return Err("ide-persona-path-symlink".into()),
        Ok(meta) if !meta.file_type().is_file() => return Err("ide-persona-style-conflict".into()),
        Ok(_) => {
            Ok(fs::read(path).map_err(|_| "ide-persona-style-read-failed")? == content.as_bytes())
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err("ide-persona-style-read-failed".into())
        }
        Err(_) => Ok(false),
    }
}

fn temporary_path(parent: &Path, name: &std::ffi::OsStr) -> PathBuf {
    parent.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        uuid::Uuid::new_v4()
    ))
}

fn validate(profile: &AgentProfile, prompt: &str) -> Result<(), String> {
    if profile.id.is_empty()
        || profile.id.len() > 128
        || profile.name.trim().is_empty()
        || profile.name.len() > 160
        || prompt.len() > 16_000
        || prompt.contains('\0')
    {
        return Err("ide-persona-invalid".into());
    }
    Ok(())
}
fn safe_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::observer::ObserverRuntime;
    use crate::session::external::{ExternalDetachReason, ObservedFocusTarget};
    use crate::session::pty_factory::fake::FakePtyFactory;
    use crate::session_events::types::AgentEventProfile;
    use crate::state::{fake::RecordingEvents, AppEvents, SessionRegistry};

    fn profile(name: &str, prompt: &str) -> AgentProfile {
        serde_json::from_value(serde_json::json!({
            "id": "agent-1", "name": name, "role": "dev", "seed": "seed",
            "createdAt": 1, "deskIndex": 0, "cwd": "/tmp/project",
            "personalityPrompt": prompt
        }))
        .unwrap()
    }

    fn manager() -> Arc<SessionManager> {
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            std::time::Duration::from_millis(3_000),
        ));
        let observer = Arc::new(ObserverRuntime::new(hub.clone(), vec![]));
        let (factory, _) = FakePtyFactory::new();
        Arc::new(SessionManager::new(
            Arc::new(factory),
            observer,
            registry,
            events as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| None),
        ))
    }

    fn attach(manager: &SessionManager, provider: &str, cwd: &str, vscode: bool) -> String {
        manager
            .attach_observed_with_focus(
                "agent-1",
                provider,
                "source-1",
                cwd,
                "owner",
                None,
                AgentEventProfile {
                    name: "Ada".into(),
                    role: None,
                },
                vscode.then(|| ObservedFocusTarget::VsCode { cwd: cwd.into() }),
            )
            .unwrap()
            .session_id
    }

    #[test]
    fn existing_style_is_idempotent_but_user_content_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voice.md");
        let style = IdePersonaStyle {
            name: "voice".into(),
            path: path.to_string_lossy().into_owned(),
            content: "---\nname: \"voice\"\n---\nhello".into(),
            exists: false,
        };
        write_style(&style).unwrap();
        write_style(&style).unwrap();
        fs::write(&path, "user edit").unwrap();
        assert_eq!(
            write_style(&style).err().as_deref(),
            Some("ide-persona-style-conflict")
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "user edit");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_style_path_is_rejected() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "keep").unwrap();
        let path = dir.path().join("voice.md");
        symlink(&target, &path).unwrap();
        let style = IdePersonaStyle {
            name: "voice".into(),
            path: path.to_string_lossy().into_owned(),
            content: "new".into(),
            exists: false,
        };
        assert_eq!(
            write_style(&style).err().as_deref(),
            Some("ide-persona-path-symlink")
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "keep");
    }

    #[test]
    fn name_is_json_quoted_and_prompt_change_gets_a_new_deterministic_file() {
        let first_profile = profile("Ada\n---\nname: injected", "Speak warmly");
        let second_profile = profile("Ada\n---\nname: injected", "Speak briefly");
        let root = tempfile::tempdir().unwrap();
        let first = style_for(&first_profile, "Speak warmly", Some(root.path())).unwrap();
        let second = style_for(&second_profile, "Speak briefly", Some(root.path())).unwrap();
        assert!(first.content.contains("name: \"Agent Office"));
        assert!(first.content.contains("\\n---\\nname: injected"));
        assert!(first.content.contains("keep-coding-instructions: true"));
        assert_ne!(first.path, second.path);
    }

    #[test]
    fn verified_vscode_claude_prepares_only_its_style_file() {
        let cwd = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let manager = manager();
        let sid = attach(&manager, "claude", cwd.path().to_str().unwrap(), true);
        let mut p = profile("Ada", "Speak warmly");
        p.cwd = Some(cwd.path().to_string_lossy().into_owned());
        let before = std::fs::read_dir(cwd.path()).unwrap().count();
        let got = get_with_config_root(&manager, &p, Some(config.path()))
            .unwrap()
            .unwrap();
        assert!(!got.style.unwrap().exists);
        let prepared =
            prepare_with_config_root(&manager, &p, &sid, "Speak warmly", Some(config.path()))
                .unwrap();
        assert!(prepared.style.unwrap().exists);
        assert_eq!(std::fs::read_dir(cwd.path()).unwrap().count(), before);
    }

    #[test]
    fn provider_and_connection_gates_reject_unsupported_or_unverified_sessions() {
        let cwd = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let mut p = profile("Ada", "voice");
        p.cwd = Some(cwd.path().to_string_lossy().into_owned());
        let codex = manager();
        let sid = attach(&codex, "codex", cwd.path().to_str().unwrap(), true);
        assert!(get_with_config_root(&codex, &p, Some(config.path()))
            .unwrap()
            .unwrap()
            .style
            .is_none());
        assert_eq!(
            prepare_with_config_root(&codex, &p, &sid, "voice", Some(config.path()))
                .err()
                .as_deref(),
            Some("ide-persona-provider-unsupported")
        );
        let cli = manager();
        attach(&cli, "claude", cwd.path().to_str().unwrap(), false);
        assert!(get_with_config_root(&cli, &p, Some(config.path()))
            .unwrap()
            .is_none());
    }

    #[test]
    fn prepare_rejects_stale_session_prompt_cwd_and_empty_prompt() {
        let cwd = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let manager = manager();
        let sid = attach(&manager, "claude", cwd.path().to_str().unwrap(), true);
        let mut p = profile("Ada", "voice");
        p.cwd = Some(cwd.path().to_string_lossy().into_owned());
        assert_eq!(
            prepare_with_config_root(&manager, &p, "old", "voice", Some(config.path()))
                .err()
                .as_deref(),
            Some("ide-persona-session-stale")
        );
        assert_eq!(
            prepare_with_config_root(&manager, &p, &sid, "other", Some(config.path()))
                .err()
                .as_deref(),
            Some("ide-persona-prompt-stale")
        );
        p.cwd = Some(other.path().to_string_lossy().into_owned());
        assert_eq!(
            get_with_config_root(&manager, &p, Some(config.path()))
                .err()
                .as_deref(),
            Some("ide-persona-stale-cwd")
        );
        p.cwd = Some(cwd.path().to_string_lossy().into_owned());
        p.personality_prompt = Some(" ".into());
        assert_eq!(
            prepare_with_config_root(&manager, &p, &sid, " ", Some(config.path()))
                .err()
                .as_deref(),
            Some("ide-persona-prompt-empty")
        );
    }

    #[test]
    fn detach_removes_context_and_old_session_id_cannot_prepare() {
        let cwd = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let manager = manager();
        let sid = attach(&manager, "claude", cwd.path().to_str().unwrap(), true);
        let mut p = profile("Ada", "voice");
        p.cwd = Some(cwd.path().to_string_lossy().into_owned());
        manager.detach_external("agent-1", ExternalDetachReason::Detach);
        assert!(get_with_config_root(&manager, &p, Some(config.path()))
            .unwrap()
            .is_none());
        assert_eq!(
            prepare_with_config_root(&manager, &p, &sid, "voice", Some(config.path()))
                .err()
                .as_deref(),
            Some("ide-persona-not-connected")
        );
    }
}
