//! App-owned, read-only observation of an explicitly selected IDE transcript.
//! No shell, Node process, control server, or provider settings are involved.
pub(crate) mod transcripts;

use std::path::Path;
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use crate::persistence::settings_store::AppSettings;
use crate::session::external::{ObservedEventKind, ObservedFocusTarget};
use crate::session::manager::SessionManager;
use crate::session_events::types::AgentEventProfile;
use transcripts::{Candidate, EventFilter, TranscriptTail};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectResult {
    pub session_id: String,
}

pub(crate) fn discover(
    provider: Option<&str>,
    cwd: Option<&str>,
) -> Result<Vec<Candidate>, String> {
    if provider.is_some_and(|p| !matches!(p, "codex" | "claude")) {
        return Err("invalid-provider".into());
    }
    let codex = crate::agent_paths::codex_home_from_env()
        .ok_or("ide-home-unavailable")?
        .join("sessions");
    let claude = crate::agent_paths::claude_config_dir_from_env()
        .ok_or("ide-home-unavailable")?
        .join("projects");
    Ok(transcripts::discover(provider, cwd, &codex, &claude))
}

pub(crate) fn connect(
    manager: &Arc<SessionManager>,
    settings: &Arc<RwLock<AppSettings>>,
    agent_id: &str,
    profile_cwd: &str,
    profile: AgentEventProfile,
    file: &Path,
    provider: &str,
    source_session_id: &str,
) -> Result<ConnectResult, String> {
    let worker = Watcher::attach(
        manager,
        settings,
        agent_id,
        profile_cwd,
        profile,
        file,
        provider,
        source_session_id,
    )?;
    let session_id = worker.session_id.clone();
    // The worker owns its file handle and releases just its own logical session
    // on every exit path, including thread creation failure. Source IDEs are
    // never our child processes and are never signalled.
    std::thread::Builder::new()
        .name("ide-session-observer".into())
        .spawn(move || {
            let mut worker = worker;
            while worker.tick().is_ok() {
                std::thread::sleep(Duration::from_millis(500));
            }
        })
        .map_err(|_| "ide-watch-start-failed".to_string())?;
    Ok(ConnectResult { session_id })
}

struct Watcher {
    manager: Weak<SessionManager>,
    settings: Weak<RwLock<AppSettings>>,
    agent_id: String,
    session_id: String,
    owner_id: String,
    sequence: u64,
    tail: TranscriptTail,
    filter: EventFilter,
}

impl Watcher {
    fn attach(
        manager: &Arc<SessionManager>,
        settings: &Arc<RwLock<AppSettings>>,
        agent_id: &str,
        profile_cwd: &str,
        profile: AgentEventProfile,
        file: &Path,
        provider: &str,
        source_session_id: &str,
    ) -> Result<Self, String> {
        if !settings.read().unwrap().observer_enabled {
            return Err("observed-observer-disabled".into());
        }
        let candidate = transcripts::inspect(file, provider)?;
        if candidate.source != "vscode" {
            return Err("source-not-vscode".into());
        }
        if candidate.source_session_id != source_session_id {
            return Err("ide-candidate-changed".into());
        }
        if profile_cwd.is_empty() || !same_directory(profile_cwd, &candidate.cwd) {
            return Err("observed-cwd-mismatch".into());
        }
        let tail = TranscriptTail::from_candidate(&candidate)?;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let attached = manager.attach_observed_with_focus(
            agent_id,
            provider,
            source_session_id,
            &candidate.cwd,
            &owner_id,
            None,
            profile,
            Some(ObservedFocusTarget::VsCode {
                cwd: candidate.cwd.clone(),
            }),
        )?;
        Ok(Self {
            manager: Arc::downgrade(manager),
            settings: Arc::downgrade(settings),
            agent_id: agent_id.into(),
            session_id: attached.session_id,
            owner_id,
            sequence: 0,
            tail,
            filter: EventFilter::new(provider, source_session_id),
        })
    }

    fn tick(&mut self) -> Result<(), String> {
        let settings = self.settings.upgrade().ok_or("ide-app-stopped")?;
        if !settings.read().unwrap().observer_enabled {
            return Err("observed-observer-disabled".into());
        }
        let manager = self.manager.upgrade().ok_or("ide-app-stopped")?;
        // Validate ownership before touching the file, so manual detach stops
        // reading even a quiet transcript on the next tick.
        self.sequence += 1;
        manager.ingest_observed_event(
            &self.agent_id,
            &self.session_id,
            &self.owner_id,
            self.sequence,
            ObservedEventKind::Heartbeat,
            None,
        )?;
        for record in self.tail.read()? {
            if let Some(kind) = self.filter.take(&record) {
                self.sequence += 1;
                manager.ingest_observed_event(
                    &self.agent_id,
                    &self.session_id,
                    &self.owner_id,
                    self.sequence,
                    kind,
                    None,
                )?;
            }
        }
        Ok(())
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(manager) = self.manager.upgrade() {
            manager.detach_observed(&self.agent_id, &self.session_id, &self.owner_id);
        }
    }
}

fn same_directory(a: &str, b: &str) -> bool {
    let expand = |s: &str| {
        if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
            if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
            {
                return Path::new(&home).join(s.get(2..).unwrap_or(""));
            }
        }
        Path::new(s).to_path_buf()
    };
    let (a, b) = (expand(a), expand(b));
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests;
