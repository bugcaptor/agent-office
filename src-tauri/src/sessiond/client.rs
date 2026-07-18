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
    /// 종료 직전 xterm 화면 스냅샷(원본 UTF-8 바이트) -- 데몬은 핸드오프
    /// *이후* 출력만 링버퍼에 담으므로, 이게 없으면 재입양 후 종료 전
    /// 화면이 사라진다. `handoff()`가 base64로 인코딩해 실어 보낸다.
    pub snapshot: Vec<u8>,
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
    /// Handoff 때 실어 보낸 종료 직전 화면 스냅샷(원본 바이트, base64
    /// 디코딩 완료) -- 호출자(`SessionManager::adopt_one`)가 `snapshot ++
    /// buffer` 순으로 이어붙여 initial_output을 구성한다.
    pub snapshot: Vec<u8>,
    pub master_fd: RawFd,
}

pub struct Client {
    stream: UnixStream,
    /// Hello로 협상된 프로토콜 버전(1..=PROTO_VERSION). v2 RPC는 이 값이 2
    /// 이상일 때만 쓸 수 있다 -- 구데몬(proto 1)과는 v1 메시지만 주고받는다.
    proto: u32,
}

impl Client {
    fn fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    /// 이 연결이 협상한 프로토콜 버전. 브로커(v2) 기능은 `>= 2`에서만 유효.
    pub fn proto(&self) -> u32 {
        self.proto
    }

    /// v2 브로커 RPC 진입 가드. 협상된 proto가 2 미만이면(구데몬 등) 즉시
    /// Err -- 호출자(BrokerPtyFactory)는 이걸 보고 in-process 폴백을 탄다.
    fn require_v2(&self) -> io::Result<()> {
        if self.proto >= 2 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "sessiond negotiated proto {} (broker mode needs >= 2)",
                self.proto
            )))
        }
    }

    /// 연결 + Hello 협상까지 마친 클라이언트를 만든다. 협상은 additive:
    /// 신앱은 Hello{PROTO_VERSION}을 먼저 보내고, 데몬이 자기 상한으로 낮춰
    /// HelloOk{p}(1..=PROTO_VERSION)를 답하면 그 p로 확정한다. **구데몬(proto 1)은
    /// Hello{2}에 Error로 답하되 연결은 유지하므로, Error를 받으면 같은 연결에서
    /// Hello{1}로 1회 재시도해 p=1로 협상한다** -- 앱 업데이트 직후 구데몬이 쥔
    /// v1 핸드오프 세션을 잃지 않게 하는 하위호환 경로.
    pub fn connect(socket_path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(socket_path)?;
        let mut client = Client { stream, proto: 0 };
        client.proto = client.negotiate()?;
        Ok(client)
    }

    fn negotiate(&self) -> io::Result<u32> {
        protocol::write_frame(self.fd(), &Message::Hello { proto: protocol::PROTO_VERSION }, None)?;
        match protocol::read_frame(self.fd())?.0 {
            Message::HelloOk { proto } if (1..=protocol::PROTO_VERSION).contains(&proto) => Ok(proto),
            Message::HelloOk { proto } => Err(io::Error::other(format!(
                "sessiond negotiated unexpected proto {proto} (max {})",
                protocol::PROTO_VERSION
            ))),
            // 구데몬: Hello{2}를 못 알아듣고 Error로 답하지만 연결은 살아 있다.
            // 같은 연결에서 v1 Hello로 한 번만 재시도한다.
            Message::Error { .. } => self.negotiate_v1_retry(),
            other => Err(io::Error::other(format!("unexpected reply to Hello: {other:?}"))),
        }
    }

    fn negotiate_v1_retry(&self) -> io::Result<u32> {
        protocol::write_frame(self.fd(), &Message::Hello { proto: 1 }, None)?;
        match protocol::read_frame(self.fd())?.0 {
            Message::HelloOk { proto: 1 } => Ok(1),
            Message::HelloOk { proto } => Err(io::Error::other(format!(
                "sessiond answered v1 Hello with unexpected proto {proto}"
            ))),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to v1 Hello retry: {other:?}"))),
        }
    }

    pub fn handoff(&self, req: HandoffRequest) -> io::Result<()> {
        let snapshot_b64 = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(&req.snapshot)
        };
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
                snapshot_b64,
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
                snapshot_b64,
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
                let snapshot = base64::engine::general_purpose::STANDARD
                    .decode(snapshot_b64)
                    .map_err(|e| {
                        let _ = nix::unistd::close(master_fd);
                        io::Error::other(e)
                    })?;
                Ok(AdoptedSession {
                    session_id,
                    pid,
                    pgid,
                    rows,
                    cols,
                    cwd,
                    cleanup_paths,
                    buffer,
                    snapshot,
                    master_fd,
                })
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

    // ── 프로토콜 v2: 상시 브로커 모드 RPC ───────────────────────────────

    /// 데몬에 세션 하나를 spawn하라고 지시하고 자식 pid를 돌려받는다.
    pub fn spawn_broker(&self, req: SpawnBrokerRequest) -> io::Result<Option<i32>> {
        self.require_v2()?;
        protocol::write_frame(
            self.fd(),
            &Message::Spawn {
                agent_id: req.agent_id,
                session_id: req.session_id,
                shell: req.shell,
                args: req.args,
                env: req.env,
                rows: req.rows,
                cols: req.cols,
                cwd: req.cwd,
                cleanup_paths: req.cleanup_paths,
            },
            None,
        )?;
        match protocol::read_frame(self.fd())?.0 {
            Message::SpawnOk { pid } => Ok(pid),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to Spawn: {other:?}"))),
        }
    }

    /// 재접속용 메타데이터+최신 스냅샷 회수(백로그는 data conn 담당).
    pub fn attach(&self, agent_id: &str) -> io::Result<AttachedMeta> {
        self.require_v2()?;
        protocol::write_frame(self.fd(), &Message::Attach { agent_id: agent_id.to_string() }, None)?;
        match protocol::read_frame(self.fd())?.0 {
            Message::AttachOk { rows, cols, snapshot_b64, exit, .. } => {
                use base64::Engine;
                let snapshot = base64::engine::general_purpose::STANDARD
                    .decode(snapshot_b64)
                    .unwrap_or_default();
                Ok(AttachedMeta {
                    rows,
                    cols,
                    snapshot,
                    exit: exit.map(|e| (e.exit_code, e.signal)),
                })
            }
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to Attach: {other:?}"))),
        }
    }

    pub fn resize(&self, agent_id: &str, rows: u16, cols: u16) -> io::Result<()> {
        self.require_v2()?;
        protocol::write_frame(
            self.fd(),
            &Message::Resize { agent_id: agent_id.to_string(), rows, cols },
            None,
        )?;
        match protocol::read_frame(self.fd())?.0 {
            Message::ResizeOk => Ok(()),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to Resize: {other:?}"))),
        }
    }

    /// 자식 종료까지 블로킹 후 `(exit_code, signal)`을 돌려준다.
    pub fn wait(&self, agent_id: &str) -> io::Result<(Option<i32>, Option<i32>)> {
        self.require_v2()?;
        protocol::write_frame(self.fd(), &Message::Wait { agent_id: agent_id.to_string() }, None)?;
        match protocol::read_frame(self.fd())?.0 {
            Message::WaitOk { exit_code, signal } => Ok((exit_code, signal)),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to Wait: {other:?}"))),
        }
    }

    pub fn update_snapshot(&self, agent_id: &str, snapshot: &[u8]) -> io::Result<()> {
        self.require_v2()?;
        let snapshot_b64 = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(snapshot)
        };
        protocol::write_frame(
            self.fd(),
            &Message::UpdateSnapshot { agent_id: agent_id.to_string(), snapshot_b64 },
            None,
        )?;
        match protocol::read_frame(self.fd())?.0 {
            Message::UpdateSnapshotOk => Ok(()),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to UpdateSnapshot: {other:?}"))),
        }
    }

    /// DataAttach를 보내고 DataAttachOk를 확인한 뒤, 이 연결을 raw 양방향
    /// 바이트 스트림으로 전환해 소유권째 돌려준다. 이후 프레이밍은 없다 --
    /// 반환된 스트림에서 read하면 백로그+라이브 PTY 출력이, write하면 그대로
    /// PTY master 입력이 된다. `Client`를 소비한다(더는 프레임 RPC 불가).
    pub fn into_data_stream(self, agent_id: &str) -> io::Result<UnixStream> {
        self.require_v2()?;
        protocol::write_frame(self.fd(), &Message::DataAttach { agent_id: agent_id.to_string() }, None)?;
        match protocol::read_frame(self.fd())?.0 {
            Message::DataAttachOk => Ok(self.stream),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply to DataAttach: {other:?}"))),
        }
    }
}

/// `spawn_broker` 인자 묶음(대응하는 v2 Spawn 메시지의 필드들).
pub struct SpawnBrokerRequest {
    pub agent_id: String,
    pub session_id: String,
    pub shell: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub rows: u16,
    pub cols: u16,
    pub cwd: String,
    pub cleanup_paths: Vec<String>,
}

/// `attach` 결과(재접속용 메타 + 최신 스냅샷 + 종료 정보). session_id는 List가
/// 이미 주므로 여기 없다.
pub struct AttachedMeta {
    pub rows: u16,
    pub cols: u16,
    pub snapshot: Vec<u8>,
    /// 자식이 이미 종료했으면 `Some((exit_code, signal))`.
    pub exit: Option<(Option<i32>, Option<i32>)>,
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

    /// 신앱(proto 2 클라이언트)이 구데몬(proto 1만 아는)을 만났을 때 협상이
    /// 재시도로 v1에 안착하는지 검증한다. 구데몬 시뮬레이션: Hello{1}만 수락하고
    /// Hello{2}엔 Error로 답하되 **연결은 유지**하며, List엔 v1 세션을 담아
    /// 응답하고 v2/기타 메시지엔 Error. 앱 업데이트 직후 구데몬이 쥔 v1 핸드오프
    /// 세션을 잃지 않게 하는 하위호환 경로의 회귀 방지.
    #[test]
    fn connect_retries_v1_hello_against_an_old_daemon_and_negotiates_proto_1() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!("ao-oldd-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket_path = dir.join("s.sock");
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let fd = stream.as_raw_fd();
            loop {
                let msg = match protocol::read_frame(fd) {
                    Ok((m, _)) => m,
                    Err(_) => return,
                };
                match msg {
                    Message::Hello { proto: 1 } => {
                        let _ = protocol::write_frame(fd, &Message::HelloOk { proto: 1 }, None);
                    }
                    Message::List => {
                        let _ = protocol::write_frame(
                            fd,
                            &Message::ListOk {
                                sessions: vec![SessionInfo {
                                    agent_id: "a1".into(),
                                    session_id: "s1".into(),
                                    pid: Some(1),
                                    rows: 24,
                                    cols: 80,
                                    cwd: "/tmp".into(),
                                    exited: false,
                                    buffered_bytes: 0,
                                    broker: false,
                                }],
                            },
                            None,
                        );
                    }
                    // 구데몬은 Hello{2}/v2 메시지를 모른다 -- Error로 답하되 연결은 유지.
                    _ => {
                        let _ = protocol::write_frame(
                            fd,
                            &Message::Error { message: "unsupported".into() },
                            None,
                        );
                    }
                }
            }
        });

        let client = Client::connect(&socket_path).expect("negotiation must retry down to v1");
        assert_eq!(client.proto(), 1, "must negotiate proto 1 against an old daemon");

        // (a) v1 메서드는 p=1에서 정상 동작한다.
        let sessions = client.list().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent_id, "a1");
        assert!(!sessions[0].broker);

        // (c) v2 래퍼는 p=1에서 네트워크로 나가지 않고 즉시 Err(폴백 유도).
        assert!(client.attach("a1").is_err());
        assert!(client.resize("a1", 30, 100).is_err());
        assert!(client.wait("a1").is_err());
        assert!(client.update_snapshot("a1", b"x").is_err());
        assert!(client
            .spawn_broker(SpawnBrokerRequest {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                shell: "/bin/sh".into(),
                args: vec![],
                env: vec![],
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
            })
            .is_err());

        drop(client);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
                snapshot: b"SCREEN-BEFORE-QUIT".to_vec(),
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
        assert_eq!(
            adopted.snapshot, b"SCREEN-BEFORE-QUIT",
            "snapshot sent at handoff must round-trip through the daemon to adopt"
        );

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
