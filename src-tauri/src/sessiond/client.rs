// src-tauri/src/sessiond/client.rs
//
// 앱 쪽에서 sessiond에 접속해 Handoff/List/Adopt/Kill을 보내는 얇은 클라이언트.
// `SessionManager::handoff_all`/`adopt_detached`가 이 모듈만 안다 -- 프레이밍/
// fd 전달의 세부 사항은 protocol.rs에 위임한다.

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::protocol::{self, Message, SessionInfo};

/// `sockaddr_un.sun_path`의 실질 상한 -- 리눅스는 108바이트, macOS/BSD는
/// 104바이트(널 종료 포함)라 더 짧은 쪽을 기준으로 여유를 둔다.
const MAX_SAFE_SOCKET_PATH_LEN: usize = 90;

/// 소켓 기본 경로: `<app_data_dir>/sessiond.sock`. macOS의 `app_data_dir`은
/// `~/Library/Application Support/<bundle-id>/`처럼 길어지기 쉬운데,
/// 유닉스 도메인 소켓 경로는 커널 상한(~100바이트)을 넘으면 bind(2)가
/// `ENAMETOOLONG`으로 실패한다. 자연스러운 경로가 상한에 근접하면 짧고
/// `app_data_dir`에 결정적으로 대응하는 `/tmp` 경로로 폴백한다.
pub fn default_socket_path(app_data_dir: &Path) -> PathBuf {
    let natural = app_data_dir.join("sessiond.sock");
    if natural.as_os_str().len() < MAX_SAFE_SOCKET_PATH_LEN {
        return natural;
    }
    let mut h = sha1_smol::Sha1::new();
    h.update(app_data_dir.to_string_lossy().as_bytes());
    let digest = h.digest().to_string();
    std::env::temp_dir().join(format!("ao-{}.sock", &digest[..12]))
}

/// 데몬 stdio 리다이렉트 로그 기본 경로: `<app_data_dir>/sessiond.log`.
pub fn default_log_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("sessiond.log")
}

pub struct HandoffRequest {
    pub agent_id: String,
    pub session_id: String,
    pub pid: Option<i32>,
    pub pgid: Option<i32>,
    pub rows: u16,
    pub cols: u16,
    pub cwd: String,
    pub cleanup_paths: Vec<String>,
    /// 호출 성공/실패와 무관하게 이 fd의 소유권은 호출자에게 남는다 -- 데몬은
    /// SCM_RIGHTS로 받은 독립 사본을 쥔다. 호출자는 성공 후 이 fd를 닫아도
    /// 세션에 영향 없음(핸드오프 목적 그대로).
    pub master_fd: RawFd,
}

/// Adopt 성공 결과. `master_fd`는 호출자 소유 -- 다 쓰면 닫을 책임이 있다.
pub struct AdoptedSession {
    pub session_id: String,
    pub pid: Option<i32>,
    pub pgid: Option<i32>,
    pub rows: u16,
    pub cols: u16,
    pub cwd: String,
    pub cleanup_paths: Vec<String>,
    pub buffer: Vec<u8>,
    pub master_fd: RawFd,
}

pub struct Client {
    stream: UnixStream,
}

impl Client {
    fn fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    /// 연결 + Hello/HelloOk 프로토콜 버전 핸드셰이크까지 마친 클라이언트를
    /// 만든다. 버전 불일치는 Err -- 호출자는 해당 세션의 입양/핸드오프를
    /// 포기해야 한다(설계 문서 §프로토콜).
    pub fn connect(socket_path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(socket_path)?;
        let client = Client { stream };
        client.hello()?;
        Ok(client)
    }

    fn hello(&self) -> io::Result<()> {
        protocol::write_frame(self.fd(), &Message::Hello { proto: protocol::PROTO_VERSION }, None)?;
        match protocol::read_frame(self.fd())?.0 {
            Message::HelloOk { proto } if proto == protocol::PROTO_VERSION => Ok(()),
            Message::HelloOk { proto } => Err(io::Error::other(format!(
                "sessiond speaks proto {proto}, expected {}",
                protocol::PROTO_VERSION
            ))),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to Hello: {other:?}"))),
        }
    }

    pub fn handoff(&self, req: HandoffRequest) -> io::Result<()> {
        protocol::write_frame(
            self.fd(),
            &Message::Handoff {
                agent_id: req.agent_id,
                session_id: req.session_id,
                pid: req.pid,
                pgid: req.pgid,
                rows: req.rows,
                cols: req.cols,
                cwd: req.cwd,
                cleanup_paths: req.cleanup_paths,
            },
            Some(req.master_fd),
        )?;
        match protocol::read_frame(self.fd())?.0 {
            Message::HandoffOk => Ok(()),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to Handoff: {other:?}"))),
        }
    }

    pub fn list(&self) -> io::Result<Vec<SessionInfo>> {
        protocol::write_frame(self.fd(), &Message::List, None)?;
        match protocol::read_frame(self.fd())?.0 {
            Message::ListOk { sessions } => Ok(sessions),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to List: {other:?}"))),
        }
    }

    pub fn adopt(&self, agent_id: &str) -> io::Result<AdoptedSession> {
        protocol::write_frame(self.fd(), &Message::Adopt { agent_id: agent_id.to_string() }, None)?;
        let (msg, fd) = protocol::read_frame(self.fd())?;
        match msg {
            Message::AdoptOk {
                session_id,
                pid,
                pgid,
                rows,
                cols,
                cwd,
                cleanup_paths,
                buffer_b64,
                ..
            } => {
                let master_fd = fd.ok_or_else(|| io::Error::other("AdoptOk missing master fd"))?;
                use base64::Engine;
                let buffer = base64::engine::general_purpose::STANDARD
                    .decode(buffer_b64)
                    .map_err(|e| {
                        let _ = nix::unistd::close(master_fd);
                        io::Error::other(e)
                    })?;
                Ok(AdoptedSession { session_id, pid, pgid, rows, cols, cwd, cleanup_paths, buffer, master_fd })
            }
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to Adopt: {other:?}"))),
        }
    }

    pub fn kill(&self, agent_id: &str) -> io::Result<()> {
        protocol::write_frame(self.fd(), &Message::Kill { agent_id: agent_id.to_string() }, None)?;
        match protocol::read_frame(self.fd())?.0 {
            Message::KillOk => Ok(()),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to Kill: {other:?}"))),
        }
    }
}

/// 데몬을 자기 자신의 실행 파일로 스폰한다(`main.rs`의 `--sessiond` 분기가
/// 받는다). `setsid()`로 세션을 분리해 앱/터미널의 시그널이 전파되지
/// 않게 하고, stdio는 로그 파일로 리다이렉트한다. 스폰된 `Child`는 의도적으로
/// 버린다(daemonize) -- drop이 프로세스를 죽이지 않는다.
fn spawn_daemon(exe_path: &Path, socket_path: &Path, log_path: &Path) -> io::Result<()> {
    use std::os::unix::process::CommandExt;

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log_out = std::fs::OpenOptions::new().create(true).append(true).open(log_path)?;
    let log_err = log_out.try_clone()?;

    let mut cmd = std::process::Command::new(exe_path);
    cmd.arg("--sessiond").arg(socket_path);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(log_out);
    cmd.stderr(log_err);
    // safety: setsid()는 async-signal-safe하고 할당을 하지 않는다 -- fork 후
    // exec 전 자식에서 호출하기에 안전한 요건을 만족한다.
    unsafe {
        cmd.pre_exec(|| nix::unistd::setsid().map(|_| ()).map_err(io::Error::from));
    }
    cmd.spawn()?;
    Ok(())
}

/// 데몬에 연결을 시도하고, 없으면 스폰 후 ~2초 백오프로 재시도한다
/// (설계 문서 §핵심 3).
pub fn connect_or_spawn(socket_path: &Path, exe_path: &Path, log_path: &Path) -> io::Result<Client> {
    if let Ok(client) = Client::connect(socket_path) {
        return Ok(client);
    }
    spawn_daemon(exe_path, socket_path, log_path)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match Client::connect(socket_path) {
            Ok(client) => return Ok(client),
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_socket_and_log_paths_are_under_app_data_dir() {
        let dir = Path::new("/tmp/agent-office-app-data");
        assert_eq!(default_socket_path(dir), dir.join("sessiond.sock"));
        assert_eq!(default_log_path(dir), dir.join("sessiond.log"));
    }

    #[test]
    fn default_socket_path_falls_back_to_a_short_tmp_path_when_natural_path_is_too_long() {
        // 실제 macOS app_data_dir처럼 길어지기 쉬운 경로를 흉내낸다.
        let long_dir = Path::new(
            "/Users/some-very-long-login-name/Library/Application Support/com.bugcaptor.agent-office",
        );
        let fallback = default_socket_path(long_dir);
        assert!(
            fallback.as_os_str().len() < MAX_SAFE_SOCKET_PATH_LEN,
            "fallback path itself must stay under the safe limit: {fallback:?}"
        );
        assert!(fallback.starts_with(std::env::temp_dir()));

        // 결정적 -- 같은 app_data_dir은 항상 같은 소켓 경로로 귀결돼야
        // 데몬과 클라이언트가 서로를 찾을 수 있다.
        assert_eq!(fallback, default_socket_path(long_dir));
    }

    /// Client(connect/hello/handoff/list/adopt)를 실 `UnixListener` 배선
    /// (`daemon::run_daemon_inner`) 상대로 왕복 검증한다. 프로덕션 진입점
    /// `run_daemon`은 종료 조건에서 `process::exit`를 호출하므로 같은
    /// 테스트 프로세스 안에서 절대 직접 부르면 안 된다(다른 모든 테스트까지
    /// 함께 죽는다) -- 대신 `run_daemon_inner`에 채널 기반의 무해한 종료
    /// 훅을 주입한다(`spawn_daemon`의 "현재 실행 파일 재실행" 자체 배선은
    /// 수동 검증 항목으로 남긴다).
    #[test]
    fn client_round_trips_handoff_list_adopt_against_a_real_daemon_thread() {
        // macOS/BSD sockaddr_un.sun_path 상한(~104바이트) 안에 들어가도록
        // 짧은 경로를 쓴다.
        let dir = std::env::temp_dir().join(format!("ao-sc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket_path = dir.join("s.sock");
        let _ = std::fs::remove_file(&socket_path);

        let (shutdown_tx, _shutdown_rx) = std::sync::mpsc::channel::<()>();
        let hook: super::super::daemon::ShutdownHook = std::sync::Arc::new(move || {
            let _ = shutdown_tx.send(());
        });
        let daemon_socket = socket_path.clone();
        std::thread::spawn(move || {
            let _ = super::super::daemon::run_daemon_inner(daemon_socket, Duration::from_secs(60), hook);
        });

        // 데몬이 소켓을 bind할 때까지 대기.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !socket_path.exists() {
            assert!(Instant::now() < deadline, "daemon never created the socket file");
            std::thread::sleep(Duration::from_millis(10));
        }

        let client = Client::connect(&socket_path).expect("handshake must succeed");

        let (master_read, master_write) = nix::unistd::pipe().unwrap();
        client
            .handoff(HandoffRequest {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(4242),
                pgid: Some(4242),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
                master_fd: master_read,
            })
            .unwrap();
        let _ = nix::unistd::close(master_read);

        let sessions = client.list().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent_id, "a1");

        nix::unistd::write(master_write, b"queued").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let adopted = client.adopt("a1").unwrap();
        assert_eq!(adopted.session_id, "s1");
        assert_eq!(adopted.pid, Some(4242));
        assert_eq!(adopted.buffer, b"queued");

        nix::unistd::write(master_write, b"more").unwrap();
        let mut buf = [0u8; 16];
        let n = nix::unistd::read(adopted.master_fd, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"more");

        let _ = nix::unistd::close(adopted.master_fd);
        let _ = nix::unistd::close(master_write);
        drop(client);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
