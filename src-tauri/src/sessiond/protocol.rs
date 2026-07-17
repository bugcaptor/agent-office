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

pub const PROTO_VERSION: u32 = 1;

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
    /// 프로토콜 버전 불일치 등, 요청을 처리할 수 없을 때. 앱은 해당 세션의
    /// 입양/핸드오프를 포기한다(세션은 데몬에 그대로 남는다).
    Error {
        message: String,
    },
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
}

fn write_all_raw(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
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
