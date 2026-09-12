//! GUI-free `agent-office serve` bootstrap. It is Unix-only because durable
//! sessions are owned by the session broker rather than this process.
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::notification::hub::{NotificationHub, SystemClock};
use crate::persistence::png_store::{PngStore, MAX_PORTRAIT_BYTES};
use crate::persistence::profile_store::ProfileStore;
use crate::persistence::settings_store::SettingsStore;
use crate::session::inject::{InjectGate, ManagerSink};
use crate::session::manager::SessionManager;
use crate::session::pty_factory::{PortablePtyFactory, PtyFactory};
use crate::state::{AppEvents, BotPromptArms, SessionRegistry};
use crate::webremote::pairing::ClientRecord;
use crate::webremote::protocol::ClientPermission;

/// Non-GUI services which must outlive a headless listener. Runtime tests use
/// this to create a real broker and web context without starting Tauri.
pub(crate) struct Context {
    pub web: Arc<crate::webremote::WebRemoteContext>,
    pub manager: Arc<SessionManager>,
    pub observer_server: Arc<crate::observer::server::ObserverServerState>,
    pub hub_notify: Arc<NotificationHub>,
}

pub fn maybe_run_server<I, S>(args: I) -> Option<i32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut it = args.into_iter();
    let _ = it.next();
    if !it
        .next()
        .map(|v| v.as_ref() == std::ffi::OsStr::new("serve"))
        .unwrap_or(false)
    {
        return None;
    }
    let mut data = None;
    let mut bind: IpAddr = "127.0.0.1".parse().expect("literal IP");
    let mut port = 47_373u16;
    let rest: Vec<_> = it.collect();
    let mut n = 0;
    while n < rest.len() {
        match rest[n].as_ref().to_string_lossy().as_ref() {
            "--help" | "-h" => {
                eprintln!(
                    "usage: agent-office serve --data-dir <dir> [--bind <ip>] [--port <port>]"
                );
                return Some(0);
            }
            "--data-dir" => {
                n += 1;
                data = rest.get(n).map(|v| PathBuf::from(v.as_ref()));
                if data.is_none() {
                    eprintln!("serve: --data-dir requires a directory");
                    return Some(2);
                }
            }
            "--bind" => {
                n += 1;
                bind = match rest
                    .get(n)
                    .and_then(|v| v.as_ref().to_str())
                    .and_then(|v| v.parse().ok())
                {
                    Some(v) => v,
                    None => {
                        eprintln!("serve: invalid --bind");
                        return Some(2);
                    }
                };
            }
            "--port" => {
                n += 1;
                port = match rest
                    .get(n)
                    .and_then(|v| v.as_ref().to_str())
                    .and_then(|v| v.parse().ok())
                {
                    Some(v) => v,
                    None => {
                        eprintln!("serve: invalid --port");
                        return Some(2);
                    }
                };
            }
            _ => {
                eprintln!("serve: unknown option");
                return Some(2);
            }
        }
        n += 1;
    }
    let Some(data_dir) = data else {
        eprintln!("serve: --data-dir is required");
        return Some(2);
    };
    #[cfg(not(unix))]
    {
        let _ = (data_dir, bind, port);
        eprintln!("serve: unsupported on this platform (the durable session broker requires Unix)");
        return Some(1);
    }
    #[cfg(unix)]
    match tokio::runtime::Runtime::new()
        .expect("Tokio runtime")
        .block_on(run(data_dir, SocketAddr::new(bind, port)))
    {
        Ok(()) => Some(0),
        Err(e) => {
            eprintln!("serve: {e}");
            Some(1)
        }
    }
}

#[cfg(unix)]
pub(crate) fn build_context(data_dir: PathBuf) -> Result<Context, String> {
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    // Build the durable output hub before the event chain, otherwise early
    // session/hook events silently miss the remote client.
    let web_hub =
        crate::webremote::host::WebRemoteHub::new_with_journal(data_dir.join("terminal-journal"));
    let registry = Arc::new(SessionRegistry::new());
    let arms = Arc::new(BotPromptArms::new());
    let remote_events: Arc<dyn AppEvents> = Arc::new(crate::webremote::host::WebRemoteEvents::new(
        web_hub.clone(),
    ));
    let recording: Arc<dyn AppEvents> = Arc::new(
        crate::session_events::recording_events::RecordingAppEvents::new(
            remote_events,
            Arc::new(crate::session_events::store::SessionEventStore::new(
                data_dir.join("session-events").join("v1"),
            )),
            arms.clone(),
        ),
    );
    let events: Arc<dyn AppEvents> = recording;
    let settings = Arc::new(RwLock::new(
        SettingsStore::new(data_dir.join("settings.json")).load().0,
    ));
    let hub_notify = Arc::new(NotificationHub::new(
        registry.clone(),
        events.clone(),
        Arc::new(SystemClock),
        Duration::from_millis(3_000),
    ));
    {
        let setting = settings
            .read()
            .map_err(|_| "settings lock poisoned".to_string())?;
        hub_notify.set_hold_duration(Duration::from_millis(setting.attention_hold_ms));
        hub_notify.set_lang(crate::i18n::ui_lang(&setting));
    }
    let observer = Arc::new(crate::observer::ObserverRuntime::production(
        hub_notify.clone(),
        data_dir.join("observer").join("claude"),
        crate::forwarder_executable_path(),
    ));
    let observer_server = Arc::new(crate::observer::server::ObserverServerState::default());
    observer_server.set_app_data_dir(data_dir.clone());
    let get_url = crate::make_observer_url_getter(settings.clone(), observer_server.clone());
    let fallback: Arc<dyn PtyFactory> = Arc::new(PortablePtyFactory);
    let factory: Arc<dyn PtyFactory> = Arc::new(
        crate::session::broker_pty::BrokerPtyFactory::new_strict(&data_dir, fallback),
    );
    let manager = Arc::new(
        SessionManager::new(
            factory,
            observer.clone(),
            registry.clone(),
            events,
            hub_notify.clone(),
            get_url,
        )
        .with_app_data_dir(data_dir.clone())
        .with_broker_mode(true),
    );
    let store = ProfileStore::new(data_dir.join("profiles.json"));
    let profiles = store.load();
    for profile in &profiles.agents {
        web_hub.share(&manager, &profile.id);
    }
    let known = profiles
        .agents
        .iter()
        .map(|profile| profile.id.clone())
        .collect();
    let _ = manager.adopt_detached(&known);
    let gate = Arc::new(InjectGate::new(
        Arc::new(ManagerSink::new(manager.clone())),
        arms,
    ));
    let web = Arc::new(crate::webremote::WebRemoteContext::new(
        crate::webremote::WebRemoteContextDeps {
            manager: manager.clone(),
            registry,
            store,
            settings,
            hub: web_hub,
            app_data_dir: data_dir.clone(),
            host_name: "Agent Office".into(),
            hub_notify: hub_notify.clone(),
            observer,
            observer_server: observer_server.clone(),
            live_usage: Arc::new(crate::usage::LiveUsageState::new()),
            portraits: Arc::new(PngStore::new(
                data_dir.join("portraits"),
                MAX_PORTRAIT_BYTES,
            )),
            gate,
        },
    ));
    web.enable_headless_server();
    Ok(Context {
        web,
        manager,
        observer_server,
        hub_notify,
    })
}

#[cfg(unix)]
fn owner_token(
    ctx: &crate::webremote::WebRemoteContext,
    data_dir: &Path,
) -> Result<String, String> {
    let path = data_dir.join("serve-token");
    let token = std::fs::read_to_string(&path)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            ctx.tokens
                .load()
                .into_iter()
                .find(|r| r.client_id == "serve-owner")
                .map(|r| r.token)
        })
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    write_owner_token(&path, &token).map_err(|e| e.to_string())?;
    ctx.tokens
        .insert(ClientRecord {
            client_id: "serve-owner".into(),
            name: "serve owner".into(),
            token: token.clone(),
            permission: ClientPermission::Input,
            created_at: crate::types::now_ms(),
        })
        .map_err(|e| e.to_string())?;
    Ok(token)
}

#[cfg(unix)]
fn write_owner_token(path: &Path, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    crate::httpapi::set_owner_only(path);
    Ok(())
}

/// Advisory flock is released with the file descriptor, so a crash cannot
/// leave a create_new-style stale lock behind.
#[cfg(unix)]
struct ServeLock {
    _file: std::fs::File,
}
#[cfg(unix)]
impl ServeLock {
    fn acquire(data_dir: &Path) -> Result<Self, String> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .open(data_dir.join("serve.lock"))
            .map_err(|e| e.to_string())?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("data directory is already served".into());
        }
        Ok(Self { _file: file })
    }
}

#[cfg(unix)]
async fn run(data_dir: PathBuf, bind: SocketAddr) -> Result<(), String> {
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let _lock = ServeLock::acquire(&data_dir)?;
    // Do this before mutating the serve-owner credential. A failed bind is a
    // failed launch, not a reason to rotate or otherwise touch authentication.
    let socket = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| e.to_string())?;
    let context = build_context(data_dir.clone())?;
    let _token = owner_token(&context.web, &data_dir)?;
    eprintln!(
        "agent-office serve listening on {bind}; token file: {}",
        data_dir.join("serve-token").display()
    );
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    let listener = crate::webremote::serve_headless_listener_on(context.web.clone(), socket);
    tokio::pin!(listener);
    loop {
        tokio::select! {
            result = &mut listener => return result.map_err(|e| e.to_string()),
            _ = ticker.tick() => context.hub_notify.flush_expired(),
            _ = shutdown_signal() => {
                // Persist accepted output before detaching the broker-owned PTYs.
                // A later client reconnect in this server lifetime then replays it;
                // an unclean process crash still has the documented offline gap.
                let _ = context.web.hub.flush_journal().await;
                context.observer_server.shutdown();
                context.manager.handoff_all(&HashMap::new(), &HashMap::new());
                return Ok(());
            }
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut interrupt = signal(SignalKind::interrupt()).expect("SIGINT handler");
    let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn serve_lock_has_no_stale_create_new_failure() {
        let dir = tempfile::tempdir().unwrap();
        let first = ServeLock::acquire(dir.path()).unwrap();
        assert!(ServeLock::acquire(dir.path()).is_err());
        drop(first);
        ServeLock::acquire(dir.path()).unwrap();
    }

    #[test]
    fn serve_argument_errors_do_not_enter_gui_bootstrap() {
        assert_eq!(
            maybe_run_server(["agent-office", "serve", "--help"]),
            Some(0)
        );
        assert_eq!(
            maybe_run_server(["agent-office", "serve", "--data-dir"]),
            Some(2)
        );
        assert_eq!(
            maybe_run_server([
                "agent-office",
                "serve",
                "--data-dir",
                "/tmp/x",
                "--port",
                "no"
            ]),
            Some(2)
        );
        assert_eq!(maybe_run_server(["agent-office", "ctl"]), None);
    }
}
