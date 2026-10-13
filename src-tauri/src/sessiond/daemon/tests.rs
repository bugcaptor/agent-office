    use super::*;
    use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
    use nix::unistd::{close, pipe, read as nix_read, write as nix_write};
    use std::time::Duration;

    /// 하나의 socketpair 절반을 `handle_connection`에 물려 백그라운드
    /// 스레드로 돌리고, 다른 절반(`client_fd`)을 테스트가 직접
    /// read_frame/write_frame으로 구동한다. 테이블은 테스트가 직접 훑어
    /// 검증할 수 있게 통째로 반환.
    struct Harness {
        client_fd: RawFd,
        table: Arc<Table>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Harness {
        fn new() -> Self {
            let (client_fd, server_fd) =
                socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
            let table: Arc<Table> = Arc::new(Mutex::new(HashMap::new()));
            let ever = Arc::new(AtomicBool::new(false));
            let table_for_thread = table.clone();
            let handle = std::thread::spawn(move || {
                handle_connection(server_fd, &table_for_thread, &ever);
                let _ = close(server_fd);
            });
            Harness { client_fd, table, handle: Some(handle) }
        }

        fn send(&self, msg: &Message, fd: Option<RawFd>) {
            protocol::write_frame(self.client_fd, msg, fd).unwrap();
        }

        fn recv(&self) -> (Message, Option<RawFd>) {
            protocol::read_frame(self.client_fd).unwrap()
        }

        fn finish(mut self) {
            let _ = close(self.client_fd);
            if let Some(h) = self.handle.take() {
                h.join().unwrap();
            }
        }
    }

    /// 짧고 프로세스/스레드 조합으로 유일한 문자열(소켓 경로 길이 상한 대응).
    fn short_id() -> String {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("{:x}{:x}", std::process::id(), n)
    }

    fn wait_until<F: Fn() -> bool>(pred: F) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !pred() {
            assert!(std::time::Instant::now() < deadline, "condition not met within timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn hello_ok_on_matching_protocol_version() {
        let h = Harness::new();
        h.send(&Message::Hello { proto: protocol::PROTO_VERSION }, None);
        let (reply, fd) = h.recv();
        assert!(fd.is_none());
        assert!(matches!(reply, Message::HelloOk { proto } if proto == protocol::PROTO_VERSION));
        h.finish();
    }

    #[test]
    fn hello_errors_on_invalid_proto_zero() {
        // proto 0은 유효한 버전이 아니므로 거부한다(그 외 >=1은 협상 수락).
        let h = Harness::new();
        h.send(&Message::Hello { proto: 0 }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::Error { .. }));
        h.finish();
    }

    #[test]
    fn hello_clamps_newer_proto_to_daemon_max() {
        // 더 높은 proto를 요청한 클라이언트(예: 브로커 제거 전 빌드의 proto 2)는
        // 데몬 상한으로 클램프된다 -- 그 클라이언트는 협상된 버전의 메시지만 보낸다.
        let h = Harness::new();
        h.send(&Message::Hello { proto: 99 }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HelloOk { proto } if proto == protocol::PROTO_VERSION));
        h.finish();
    }

    #[test]
    fn handoff_registers_session_and_list_reflects_it() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();

        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(111),
                pgid: Some(111),
                rows: 24,
                cols: 80,
                cwd: "/tmp/work".into(),
                cleanup_paths: vec!["/tmp/settings.json".into()],
                snapshot_b64: String::new(),
            },
            Some(master_read),
        );
        let _ = close(master_read); // 전송측 사본은 곧바로 닫아도 무방(데몬이 독립 사본을 받음)
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HandoffOk));
        assert_eq!(h.table.lock().unwrap().len(), 1);

        h.send(&Message::List, None);
        let (reply, _) = h.recv();
        match reply {
            Message::ListOk { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].agent_id, "a1");
                assert_eq!(sessions[0].session_id, "s1");
                assert_eq!(sessions[0].pid, Some(111));
                assert_eq!(sessions[0].cwd, "/tmp/work");
                assert!(!sessions[0].exited);
                assert_eq!(sessions[0].buffered_bytes, 0);
            }
            other => panic!("unexpected reply: {other:?}"),
        }

        nix_write(master_write, b"hello from pty").unwrap();
        wait_until(|| {
            h.send(&Message::List, None);
            let (reply, _) = h.recv();
            matches!(&reply, Message::ListOk { sessions } if sessions[0].buffered_bytes > 0)
        });

        let _ = close(master_write);
        h.finish();
    }

    #[test]
    fn adopt_removes_from_table_stops_reader_and_hands_back_buffered_output() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();

        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(222),
                pgid: Some(222),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
                snapshot_b64: String::new(),
            },
            Some(master_read),
        );
        let _ = close(master_read);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HandoffOk));

        nix_write(master_write, b"buffered-before-adopt").unwrap();
        wait_until(|| h.table.lock().unwrap()["a1"].ring.lock().unwrap().len() > 0);

        h.send(&Message::Adopt { agent_id: "a1".into() }, None);
        let (reply, fd) = h.recv();
        let adopted_fd = fd.expect("AdoptOk must carry the master fd");
        match reply {
            Message::AdoptOk { agent_id, session_id, pid, buffer_b64, .. } => {
                assert_eq!(agent_id, "a1");
                assert_eq!(session_id, "s1");
                assert_eq!(pid, Some(222));
                use base64::Engine;
                let decoded = base64::engine::general_purpose::STANDARD.decode(buffer_b64).unwrap();
                assert_eq!(decoded, b"buffered-before-adopt");
            }
            other => panic!("unexpected reply: {other:?}"),
        }
        assert!(
            h.table.lock().unwrap().is_empty(),
            "Adopt must remove the entry from the table"
        );

        // 데몬 쪽 리더는 정지됐으니, 입양된 fd로 이어 쓴 바이트는 우리가
        // 받은 fd에서 직접 읽혀야 한다(이중 리더가 아니라는 증거).
        nix_write(master_write, b"after-adopt").unwrap();
        let mut buf = [0u8; 32];
        let n = nix_read(adopted_fd, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"after-adopt");

        let _ = close(adopted_fd);
        let _ = close(master_write);
        h.finish();
    }

    /// 종료 직전 화면 스냅샷(§실증에서 발견된 빈틈 수정) 회귀: Handoff의
    /// snapshot_b64가 그대로 테이블에 보관됐다가 Adopt 응답의 snapshot_b64로
    /// 되돌아오는지 검증한다. 데몬은 이 바이트열을 전혀 해석하지 않고
    /// 불투명하게 보관/반환만 한다.
    #[test]
    fn handoff_snapshot_is_stored_and_returned_via_adopt_ok() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();
        use base64::Engine;
        let snapshot_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"SCREEN-BEFORE-QUIT\r\n$ ls\r\n");

        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(333),
                pgid: Some(333),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
                snapshot_b64: snapshot_b64.clone(),
            },
            Some(master_read),
        );
        let _ = close(master_read);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HandoffOk));

        h.send(&Message::Adopt { agent_id: "a1".into() }, None);
        let (reply, fd) = h.recv();
        let adopted_fd = fd.expect("AdoptOk must carry the master fd");
        match reply {
            Message::AdoptOk { snapshot_b64: returned, .. } => {
                assert_eq!(returned, snapshot_b64, "snapshot must round-trip unchanged");
            }
            other => panic!("unexpected reply: {other:?}"),
        }

        let _ = close(adopted_fd);
        let _ = close(master_write);
        h.finish();
    }

    #[test]
    fn adopt_of_unknown_agent_returns_error() {
        let h = Harness::new();
        h.send(&Message::Adopt { agent_id: "ghost".into() }, None);
        let (reply, fd) = h.recv();
        assert!(fd.is_none());
        assert!(matches!(reply, Message::Error { .. }));
        h.finish();
    }

    #[test]
    fn kill_removes_from_table_and_closes_master_fd() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();
        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(333),
                pgid: Some(333),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
                snapshot_b64: String::new(),
            },
            Some(master_read),
        );
        let _ = close(master_read);
        h.recv();

        h.send(&Message::Kill { agent_id: "a1".into() }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::KillOk));
        assert!(h.table.lock().unwrap().is_empty());

        let _ = close(master_write);
        h.finish();
    }

    #[test]
    fn reader_marks_session_exited_on_real_eof_not_on_adopt_interrupt() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();
        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(444),
                pgid: Some(444),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
                snapshot_b64: String::new(),
            },
            Some(master_read),
        );
        let _ = close(master_read);
        h.recv();

        // 쓰기 끝을 닫아 "프로세스 종료"를 흉내낸다 -- 리더는 진짜 EOF를 본다.
        let _ = close(master_write);

        wait_until(|| {
            h.send(&Message::List, None);
            let (reply, _) = h.recv();
            matches!(&reply, Message::ListOk { sessions } if sessions[0].exited)
        });

        h.finish();
    }

    #[test]
    fn table_becomes_empty_after_adopting_the_only_session() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();
        h.send(
            &Message::Handoff {
                agent_id: "solo".into(),
                session_id: "s1".into(),
                pid: Some(555),
                pgid: Some(555),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
                snapshot_b64: String::new(),
            },
            Some(master_read),
        );
        let _ = close(master_read);
        h.recv();
        assert_eq!(h.table.lock().unwrap().len(), 1);

        h.send(&Message::Adopt { agent_id: "solo".into() }, None);
        let (_, fd) = h.recv();
        assert!(h.table.lock().unwrap().is_empty());

        let _ = close(fd.unwrap());
        let _ = close(master_write);
        h.finish();
    }

    /// 실 `UnixListener` accept 루프(`run_daemon_inner`)까지 통째로 검증:
    /// 소켓에 실제로 connect해 Handoff/List/Adopt를 왕복하고, 세션이 하나
    /// 뿐이던 테이블이 Adopt로 비워진 뒤 연결을 끊으면 종료 훅이 정확히
    /// 한 번 불리는지 확인한다. `on_shutdown`을 채널로 바꿔치기해
    /// `process::exit`가 테스트 프로세스를 죽이지 않게 한다.
    #[test]
    fn run_daemon_inner_fires_shutdown_hook_once_table_empties_after_disconnect() {
        use std::os::unix::net::UnixStream;

        // macOS/BSD sockaddr_un.sun_path 상한(~104바이트) 안에 들어가도록
        // 짧은 경로를 쓴다 -- temp_dir() + 서술적인 이름은 쉽게 초과한다.
        let dir = std::env::temp_dir().join(format!("ao-sd-{}", short_id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket_path = dir.join("s.sock");

        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();
        let hook: ShutdownHook = Arc::new(move || {
            let _ = shutdown_tx.send(());
        });

        let socket_for_daemon = socket_path.clone();
        std::thread::spawn(move || {
            let _ = run_daemon_inner(socket_for_daemon, Duration::from_secs(60), hook);
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !socket_path.exists() {
            assert!(std::time::Instant::now() < deadline, "daemon never bound the socket");
            std::thread::sleep(Duration::from_millis(10));
        }

        let stream = UnixStream::connect(&socket_path).unwrap();
        let fd = stream.as_raw_fd();
        protocol::write_frame(fd, &Message::Hello { proto: protocol::PROTO_VERSION }, None).unwrap();
        assert!(matches!(protocol::read_frame(fd).unwrap().0, Message::HelloOk { .. }));

        let (master_read, master_write) = pipe().unwrap();
        protocol::write_frame(
            fd,
            &Message::Handoff {
                agent_id: "only".into(),
                session_id: "s1".into(),
                pid: Some(1),
                pgid: Some(1),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
                snapshot_b64: String::new(),
            },
            Some(master_read),
        )
        .unwrap();
        let _ = close(master_read);
        assert!(matches!(protocol::read_frame(fd).unwrap().0, Message::HandoffOk));

        protocol::write_frame(fd, &Message::Adopt { agent_id: "only".into() }, None).unwrap();
        let (reply, adopted_fd) = protocol::read_frame(fd).unwrap();
        assert!(matches!(reply, Message::AdoptOk { .. }));
        let _ = close(adopted_fd.unwrap());
        let _ = close(master_write);

        // 종료 훅은 "연결이 끊길 때" 평가되므로, 아직 연결을 안 끊은
        // 지금은 신호가 오면 안 된다.
        assert!(shutdown_rx.try_recv().is_err());

        drop(stream); // 연결 종료 -> 테이블이 비었으니 훅이 불려야 한다.
        shutdown_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("shutdown hook must fire once the table empties after disconnect");

        let _ = std::fs::remove_dir_all(&dir);
    }
