// src-tauri/src/sessiond/protocol.rs
//
// 앱<->sessiond UDS 프로토콜(버전 1, unix 전용). 프레이밍은 `u32 LE 길이 +
// JSON(serde)`. fd는 해당 프레임의 sendmsg에 SCM_RIGHTS 보조 데이터로
// 최대 1개 첨부한다(Handoff 요청, AdoptOk 응답만 fd를 동반).
//
// 길이 프리픽스는 평범한 write/read(순수 바이트, cmsg 없음)로 보내고,
// 본문은 항상 sendmsg/recvmsg로 주고받는다 — fd가 있든 없든 동일 경로를
// 타야 "본문의 첫 바이트와 함께"라는 SCM_RIGHTS 전달 규칙(리눅스 man
// unix(7): 스트림 소켓에서 ancillary data는 그 데이터의 첫 바이트를 읽는
// recvmsg 호출에서만 전달된다)을 항상 만족시킬 수 있다. buffer_b64(최대
// 512KB 링버퍼 기반, base64 팽창 포함 ~700KB)처럼 큰 프레임은 소켓 버퍼
// 한도 때문에 한 번의 sendmsg/recvmsg로 못 끝날 수 있어 양쪽 다 루프를
// 돈다 — fd는 언제나 첫 청크에만 실린다.

use std::io::{self, IoSlice, IoSliceMut};
use std::os::unix::io::RawFd;

use nix::sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags};
use serde::{Deserialize, Serialize};

// 프로토콜 버전. v1(=1)은 종료 시점 fd 핸드오프(Handoff/Adopt), v2(=2)는
// 상시 브로커 모드(Spawn/DataAttach/Attach/Resize/Wait/KillAll/UpdateSnapshot)를
// additive로 얹은 것이다. 규칙은 additive-only: 기존 메시지(Hello/Handoff/
// List/Adopt/Kill)는 의미 불변이고, 신규 필드는 전부 `#[serde(default)]`.
// 자세한 와이어 계약은 docs/session-broker-v2-design.md "프로토콜 v2 확정" 참조.
pub const PROTO_VERSION: u32 = 2;

/// 프레임이 실을 수 있는 최대 JSON 바이트 수. 512KB 링버퍼가 base64로
/// ~700KB까지 부푸는 것을 감안한 넉넉한 상한 — 이 이상은 손상된/악의적
/// 프레임으로 보고 즉시 거부한다(무한정 큰 할당 방지).
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Message {
    Hello {
        proto: u32,
    },
    HelloOk {
        proto: u32,
    },
    /// 앱 -> 데몬. 세션 하나를 데몬 테이블로 넘긴다. fd(마스터)가 동반된다.
    Handoff {
        agent_id: String,
        session_id: String,
        pid: Option<i32>,
        pgid: Option<i32>,
        rows: u16,
        cols: u16,
        cwd: String,
        cleanup_paths: Vec<String>,
        /// 종료 직전 xterm 화면(스크롤백 포함, 프론트 SerializeAddon
        /// 직렬화 결과의 UTF-8 바이트를 base64) — 데몬은 핸드오프 *이후*
        /// 출력만 링버퍼에 담으므로, 이게 없으면 재입양 후 종료 전 화면이
        /// 사라진다(실증에서 발견된 빈틈). `#[serde(default)]`는 이 필드
        /// 추가 전/후 빌드가 섞일 여지에 대비한 방어적 하위호환 — PROTO_VERSION
        /// 자체는 앱/데몬이 항상 함께 배포되므로 그대로 1을 유지한다.
        #[serde(default)]
        snapshot_b64: String,
    },
    HandoffOk,
    List,
    ListOk {
        sessions: Vec<SessionInfo>,
    },
    /// 앱 -> 데몬. 응답 AdoptOk에 fd(마스터)가 동반된다.
    Adopt {
        agent_id: String,
    },
    AdoptOk {
        agent_id: String,
        session_id: String,
        pid: Option<i32>,
        pgid: Option<i32>,
        rows: u16,
        cols: u16,
        cwd: String,
        cleanup_paths: Vec<String>,
        /// 데몬이 보관해 둔 미전달 출력(표준 base64).
        buffer_b64: String,
        /// Handoff 때 받은 종료 직전 화면 스냅샷을 그대로 되돌려준다(표준
        /// base64) — 입양 쪽이 `snapshot ++ buffer` 순으로 이어붙여
        /// initial_output을 구성한다. `#[serde(default)]`: Handoff와 동일한
        /// 이유의 방어적 하위호환.
        #[serde(default)]
        snapshot_b64: String,
    },
    Kill {
        agent_id: String,
    },
    KillOk,

    // ── 프로토콜 v2: 상시 브로커 모드(additive) ─────────────────────────
    //
    // v2에서는 데몬이 스폰부터 PTY와 자식을 소유하고 앱은 연결만 붙였다
    // 뗀다. control 연결(프레이밍 유지)로 아래 RPC를 주고받되, DataAttach만은
    // 응답 직후 그 연결을 raw 양방향 바이트 스트림으로 전환한다(프레이밍 없음).
    /// 앱 -> 데몬. 데몬이 openpty+spawn으로 세션을 새로 만들고 테이블에
    /// 등록한다(링버퍼는 스폰 시점부터 수집). fd는 동반하지 않는다 --
    /// v1 Handoff와 정반대로, 소유권이 처음부터 데몬에 있다.
    Spawn {
        agent_id: String,
        session_id: String,
        shell: String,
        #[serde(default)]
        args: Vec<String>,
        /// (key, value) 쌍 목록. 세션 env(관찰자 훅/설정 파일 경로 등)는
        /// 앱(SessionManager)이 이미 계산해 넘긴다 -- 데몬은 그대로 주입만 한다.
        #[serde(default)]
        env: Vec<(String, String)>,
        rows: u16,
        cols: u16,
        cwd: String,
        #[serde(default)]
        cleanup_paths: Vec<String>,
    },
    SpawnOk {
        pid: Option<i32>,
    },
    /// 앱 -> 데몬. 응답 `DataAttachOk` 직후 이 연결은 raw 양방향 바이트
    /// 스트림으로 전환된다: 데몬은 먼저 링버퍼 백로그를, 이어서 라이브 PTY
    /// 출력을 같은 스트림에 흘린다(이음새 없는 리플레이). 앱->데몬 방향 raw
    /// 바이트는 PTY master에 기록된다. 세션당 활성 data conn은 1개 --
    /// 새 DataAttach가 오면 기존 연결을 끊고 교체한다.
    DataAttach {
        agent_id: String,
    },
    DataAttachOk,
    /// 앱 -> 데몬. 재접속용 메타데이터+최신 스냅샷 회수(백로그는 data conn이
    /// 담당하므로 여기엔 버퍼가 없다). `exit`이 Some이면 이미 종료된 세션.
    Attach {
        agent_id: String,
    },
    AttachOk {
        rows: u16,
        cols: u16,
        pid: Option<i32>,
        /// 앱이 주기적으로 업로드한 최신 xterm 화면 스냅샷(표준 base64).
        #[serde(default)]
        snapshot_b64: String,
        /// 자식이 이미 종료했으면 Some(그 종료 정보).
        #[serde(default)]
        exit: Option<ExitStatusMsg>,
    },
    /// 앱 -> 데몬. 데몬이 PTY master를 resize(TIOCSWINSZ)한다.
    Resize {
        agent_id: String,
        rows: u16,
        cols: u16,
    },
    ResizeOk,
    /// 앱 -> 데몬. 자식 종료까지 블로킹 후 `WaitOk`. 연결별 독립 처리이므로
    /// waiter는 전용 control 연결을 하나 열어 이 요청만 보낸다.
    Wait {
        agent_id: String,
    },
    WaitOk {
        exit_code: Option<i32>,
        signal: Option<i32>,
    },
    /// 앱 -> 데몬. 브로커가 소유한 모든 자식을 SIGKILL하고 테이블을 비운다
    /// ("모두 종료"). 죽인 개수를 반환한다.
    KillAll,
    KillAllOk {
        killed: usize,
    },
    /// 앱 -> 데몬. 주기 스냅샷 업로드 -- 데몬은 세션당 최신 것만 보관한다.
    /// 앱 크래시 후에도 마지막 화면을 Attach로 복원할 수 있게 하는 것.
    UpdateSnapshot {
        agent_id: String,
        snapshot_b64: String,
    },
    UpdateSnapshotOk,

    /// 프로토콜 버전 불일치 등, 요청을 처리할 수 없을 때. 앱은 해당 세션의
    /// 입양/핸드오프를 포기한다(세션은 데몬에 그대로 남는다).
    Error {
        message: String,
    },
}

/// 자식 종료 정보(Attach 응답 전용). WaitOk는 필드를 평면으로 갖지만,
/// AttachOk는 "종료했는가"를 Option으로 감싸야 해 별도 구조체로 뺀다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitStatusMsg {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub agent_id: String,
    pub session_id: String,
    pub pid: Option<i32>,
    pub rows: u16,
    pub cols: u16,
    pub cwd: String,
    pub exited: bool,
    pub buffered_bytes: usize,
    /// v2 브로커 세션이면 true(데몬이 스폰부터 소유). v1 핸드오프 세션은
    /// false. `#[serde(default)]`: v1 데몬이 이 필드 없이 보낸 ListOk도 그대로
    /// 역직렬화되게 하는 additive 하위호환.
    #[serde(default)]
    pub broker: bool,
}

pub(crate) fn write_all_raw(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        match nix::unistd::write(fd, buf) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0")),
            Ok(n) => buf = &buf[n..],
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::from(e)),
        }
    }
    Ok(())
}

fn read_exact_raw(fd: RawFd, buf: &mut [u8]) -> io::Result<()> {
    let mut off = 0;
    while off < buf.len() {
        match nix::unistd::read(fd, &mut buf[off..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed",
                ))
            }
            Ok(n) => off += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::from(e)),
        }
    }
    Ok(())
}

/// 본문을 전송한다. `out_fd`가 있으면 첫 sendmsg 호출에만 SCM_RIGHTS로
/// 붙인다 — 그 뒤의 연속 전송(대형 프레임)은 평범한 write.
fn send_body(fd: RawFd, body: &[u8], out_fd: Option<RawFd>) -> io::Result<()> {
    let mut offset = 0usize;
    if let Some(f) = out_fd {
        let fds = [f];
        let cmsg = [ControlMessage::ScmRights(&fds)];
        loop {
            let iov = [IoSlice::new(&body[offset..])];
            match sendmsg::<()>(fd, &iov, &cmsg, MsgFlags::empty(), None) {
                Ok(0) if !body.is_empty() => {
                    return Err(io::Error::new(io::ErrorKind::WriteZero, "sendmsg returned 0"))
                }
                Ok(n) => {
                    offset += n;
                    break;
                }
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(io::Error::from(e)),
            }
        }
    }
    write_all_raw(fd, &body[offset..])
}

pub fn write_frame(fd: RawFd, msg: &Message, out_fd: Option<RawFd>) -> io::Result<()> {
    let json = serde_json::to_vec(msg).map_err(io::Error::other)?;
    let len = u32::try_from(json.len()).map_err(|_| io::Error::other("frame too large to encode"))?;
    write_all_raw(fd, &len.to_le_bytes())?;
    send_body(fd, &json, out_fd)
}

/// 프레임 하나를 읽는다. 첫 recvmsg 청크에 딸려 온 fd가 있으면 함께 반환한다.
/// 연결이 끊기면 `UnexpectedEof`.
pub fn read_frame(fd: RawFd) -> io::Result<(Message, Option<RawFd>)> {
    let mut len_buf = [0u8; 4];
    read_exact_raw(fd, &mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::other(format!(
            "frame too large: {len} bytes (max {MAX_FRAME_BYTES})"
        )));
    }

    let mut body = vec![0u8; len];
    let mut got = 0usize;
    let mut received_fd: Option<RawFd> = None;
    while got < len {
        let mut cmsg_space = nix::cmsg_space!(RawFd);
        let msg = {
            let mut iov = [IoSliceMut::new(&mut body[got..])];
            recvmsg::<()>(fd, &mut iov, Some(&mut cmsg_space), MsgFlags::empty())
                .map_err(io::Error::from)?
        };
        if msg.bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed mid-frame",
            ));
        }
        got += msg.bytes;
        for cmsg in msg.cmsgs() {
            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                for extra in fds {
                    if received_fd.is_none() {
                        received_fd = Some(extra);
                    } else {
                        // 프로토콜상 프레임당 fd는 최대 1개 -- 초과분은 방어적으로 버린다.
                        let _ = nix::unistd::close(extra);
                    }
                }
            }
        }
    }

    let message: Message = serde_json::from_slice(&body).map_err(io::Error::other)?;
    Ok((message, received_fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
    use nix::unistd::close;

    fn pair() -> (RawFd, RawFd) {
        socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap()
    }

    #[test]
    fn round_trips_a_message_without_fd() {
        let (a, b) = pair();
        let msg = Message::Hello { proto: PROTO_VERSION };
        write_frame(a, &msg, None).unwrap();
        let (decoded, fd) = read_frame(b).unwrap();
        assert!(fd.is_none());
        match decoded {
            Message::Hello { proto } => assert_eq!(proto, PROTO_VERSION),
            other => panic!("unexpected message: {other:?}"),
        }
        let _ = close(a);
        let _ = close(b);
    }

    #[test]
    fn round_trips_a_message_with_an_attached_fd() {
        let (a, b) = pair();
        // 전달할 fd로 쓸 아무 파이프.
        let (carried_read, carried_write) = nix::unistd::pipe().unwrap();

        let msg = Message::Handoff {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            pid: Some(123),
            pgid: Some(123),
            rows: 24,
            cols: 80,
            cwd: "/tmp".into(),
            cleanup_paths: vec!["/tmp/x.json".into()],
            snapshot_b64: "c2NyZWVuLXNuYXBzaG90".into(), // "screen-snapshot"
        };
        write_frame(a, &msg, Some(carried_read)).unwrap();
        // 전송측은 SCM_RIGHTS 전달 후 자기 쪽 사본을 닫아도 무방(수신측이
        // 이미 독립된 fd를 받는다) -- 실제 handoff_all 경로와 동일한 패턴.
        let _ = close(carried_read);

        let (decoded, fd) = read_frame(b).unwrap();
        let received = fd.expect("fd must ride along with the first chunk");
        // 수신 fd 번호가 송신 쪽 fd 번호와 다를 필요는 없다(같은 프로세스
        // 안에서는 방금 close된 낮은 번호가 재사용될 수 있다) -- 진짜 증거는
        // 아래에서 이 fd로 실제 데이터를 읽을 수 있는지다.

        match decoded {
            Message::Handoff {
                agent_id,
                session_id,
                pid,
                pgid,
                rows,
                cols,
                cwd,
                cleanup_paths,
                snapshot_b64,
            } => {
                assert_eq!(agent_id, "a1");
                assert_eq!(session_id, "s1");
                assert_eq!(pid, Some(123));
                assert_eq!(pgid, Some(123));
                assert_eq!(rows, 24);
                assert_eq!(cols, 80);
                assert_eq!(cwd, "/tmp");
                assert_eq!(cleanup_paths, vec!["/tmp/x.json".to_string()]);
                assert_eq!(snapshot_b64, "c2NyZWVuLXNuYXBzaG90");
            }
            other => panic!("unexpected message: {other:?}"),
        }

        // 받은 fd가 실제로 살아있는 원본 파이프를 가리키는지 데이터로 검증.
        nix::unistd::write(carried_write, b"ping").unwrap();
        let mut buf = [0u8; 4];
        let n = nix::unistd::read(received, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"ping");

        let _ = close(received);
        let _ = close(carried_write);
        let _ = close(a);
        let _ = close(b);
    }

    #[test]
    fn round_trips_list_ok_with_multiple_sessions() {
        let (a, b) = pair();
        let msg = Message::ListOk {
            sessions: vec![
                SessionInfo {
                    agent_id: "a1".into(),
                    session_id: "s1".into(),
                    pid: Some(1),
                    rows: 24,
                    cols: 80,
                    cwd: "/tmp".into(),
                    exited: false,
                    buffered_bytes: 0,
                    broker: false,
                },
                SessionInfo {
                    agent_id: "a2".into(),
                    session_id: "s2".into(),
                    pid: None,
                    rows: 40,
                    cols: 120,
                    cwd: "/tmp/b".into(),
                    exited: true,
                    buffered_bytes: 42,
                    broker: true,
                },
            ],
        };
        write_frame(a, &msg, None).unwrap();
        let (decoded, fd) = read_frame(b).unwrap();
        assert!(fd.is_none());
        match decoded {
            Message::ListOk { sessions } => {
                assert_eq!(sessions.len(), 2);
                assert_eq!(sessions[0].agent_id, "a1");
                assert!(!sessions[0].exited);
                assert_eq!(sessions[1].agent_id, "a2");
                assert!(sessions[1].exited);
                assert_eq!(sessions[1].buffered_bytes, 42);
            }
            other => panic!("unexpected message: {other:?}"),
        }
        let _ = close(a);
        let _ = close(b);
    }

    #[test]
    fn round_trips_a_large_frame_spanning_multiple_socket_reads() {
        // buffer_b64 실사용 규모(512KB 링버퍼의 base64 팽창 근사)를 흉내내
        // 소켓 송수신 버퍼를 넘는 다중 recvmsg/send 루프 경로를 실제로 태운다.
        let (a, b) = pair();
        let big = "A".repeat(700_000);
        let msg = Message::AdoptOk {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            pid: Some(1),
            pgid: Some(1),
            rows: 24,
            cols: 80,
            cwd: "/tmp".into(),
            cleanup_paths: vec![],
            buffer_b64: big.clone(),
            snapshot_b64: "c25hcHNob3Q=".into(), // "snapshot"
        };

        let writer = std::thread::spawn(move || {
            write_frame(a, &msg, None).unwrap();
            close(a).unwrap();
        });
        let (decoded, fd) = read_frame(b).unwrap();
        writer.join().unwrap();
        assert!(fd.is_none());
        match decoded {
            Message::AdoptOk { buffer_b64, snapshot_b64, .. } => {
                assert_eq!(buffer_b64, big);
                assert_eq!(snapshot_b64, "c25hcHNob3Q=");
            }
            other => panic!("unexpected message: {other:?}"),
        }
        let _ = close(b);
    }

    #[test]
    fn round_trips_v2_spawn_and_spawn_ok() {
        let (a, b) = pair();
        let msg = Message::Spawn {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            shell: "/bin/zsh".into(),
            args: vec!["-l".into(), "-i".into()],
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("AGENT_OFFICE_SESSION".into(), "s1".into()),
            ],
            rows: 24,
            cols: 80,
            cwd: "/tmp/work".into(),
            cleanup_paths: vec!["/tmp/settings.json".into()],
        };
        write_frame(a, &msg, None).unwrap();
        match read_frame(b).unwrap().0 {
            Message::Spawn { agent_id, shell, args, env, cwd, cleanup_paths, .. } => {
                assert_eq!(agent_id, "a1");
                assert_eq!(shell, "/bin/zsh");
                assert_eq!(args, vec!["-l".to_string(), "-i".to_string()]);
                assert_eq!(env[0], ("TERM".to_string(), "xterm-256color".to_string()));
                assert_eq!(cwd, "/tmp/work");
                assert_eq!(cleanup_paths, vec!["/tmp/settings.json".to_string()]);
            }
            other => panic!("unexpected: {other:?}"),
        }
        write_frame(a, &Message::SpawnOk { pid: Some(4242) }, None).unwrap();
        assert!(matches!(read_frame(b).unwrap().0, Message::SpawnOk { pid: Some(4242) }));
        let _ = close(a);
        let _ = close(b);
    }

    #[test]
    fn round_trips_v2_attach_ok_with_and_without_exit() {
        let (a, b) = pair();
        // 살아있는 세션: exit = None.
        write_frame(
            a,
            &Message::AttachOk {
                rows: 40,
                cols: 120,
                pid: Some(7),
                snapshot_b64: "c25hcA==".into(),
                exit: None,
            },
            None,
        )
        .unwrap();
        match read_frame(b).unwrap().0 {
            Message::AttachOk { rows, cols, pid, snapshot_b64, exit } => {
                assert_eq!((rows, cols, pid), (40, 120, Some(7)));
                assert_eq!(snapshot_b64, "c25hcA==");
                assert!(exit.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
        // 종료된 세션: exit = Some.
        write_frame(
            a,
            &Message::AttachOk {
                rows: 24,
                cols: 80,
                pid: None,
                snapshot_b64: String::new(),
                exit: Some(ExitStatusMsg { exit_code: Some(3), signal: None }),
            },
            None,
        )
        .unwrap();
        match read_frame(b).unwrap().0 {
            Message::AttachOk { exit: Some(e), .. } => {
                assert_eq!(e.exit_code, Some(3));
                assert_eq!(e.signal, None);
            }
            other => panic!("unexpected: {other:?}"),
        }
        let _ = close(a);
        let _ = close(b);
    }

    #[test]
    fn round_trips_v2_control_messages() {
        let (a, b) = pair();
        for msg in [
            Message::DataAttach { agent_id: "a1".into() },
            Message::DataAttachOk,
            Message::Resize { agent_id: "a1".into(), rows: 30, cols: 100 },
            Message::ResizeOk,
            Message::Wait { agent_id: "a1".into() },
            Message::WaitOk { exit_code: Some(0), signal: None },
            Message::KillAll,
            Message::KillAllOk { killed: 3 },
            Message::UpdateSnapshot { agent_id: "a1".into(), snapshot_b64: "eA==".into() },
            Message::UpdateSnapshotOk,
        ] {
            write_frame(a, &msg, None).unwrap();
            let decoded = read_frame(b).unwrap().0;
            // 타입 태그가 라운드트립되는지만 확인(개별 필드는 위 테스트에서 커버).
            assert_eq!(
                std::mem::discriminant(&decoded),
                std::mem::discriminant(&msg),
                "message discriminant must round-trip: {msg:?}"
            );
        }
        let _ = close(a);
        let _ = close(b);
    }

    #[test]
    fn session_info_broker_field_defaults_to_false_for_v1_wire() {
        // v1 데몬이 broker 필드 없이 보낸 ListOk도 그대로 역직렬화돼야 한다
        // (additive 하위호환) -- broker는 default(false).
        let json = r#"{"type":"listOk","sessions":[{"agentId":"a1","sessionId":"s1","pid":1,"rows":24,"cols":80,"cwd":"/tmp","exited":false,"bufferedBytes":0}]}"#;
        match serde_json::from_str::<Message>(json).unwrap() {
            Message::ListOk { sessions } => assert!(!sessions[0].broker),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn read_frame_reports_unexpected_eof_when_peer_closes_mid_frame() {
        let (a, b) = pair();
        // 길이 프리픽스만 쓰고 본문 없이 닫는다.
        write_all_raw(a, &100u32.to_le_bytes()).unwrap();
        close(a).unwrap();
        let err = read_frame(b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        let _ = close(b);
    }

    #[test]
    fn read_frame_rejects_frames_above_the_size_cap() {
        let (a, b) = pair();
        write_all_raw(a, &(MAX_FRAME_BYTES as u32 + 1).to_le_bytes()).unwrap();
        let err = read_frame(b).unwrap_err();
        assert!(err.to_string().contains("frame too large"));
        let _ = close(a);
        let _ = close(b);
    }
}
