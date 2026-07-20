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
        broker: Arc<BrokerTable>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Harness {
        fn new() -> Self {
            let (client_fd, server_fd) =
                socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
            let table: Arc<Table> = Arc::new(Mutex::new(HashMap::new()));
            let broker: Arc<BrokerTable> = Arc::new(Mutex::new(HashMap::new()));
            let ever = Arc::new(AtomicBool::new(false));
            let table_for_thread = table.clone();
            let broker_for_thread = broker.clone();
            let handle = std::thread::spawn(move || {
                handle_connection(server_fd, &table_for_thread, &broker_for_thread, &ever);
                let _ = close(server_fd);
            });
            Harness { client_fd, table, broker, handle: Some(handle) }
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

    /// 소켓에서 raw 바이트를 `needle`가 나타날 때까지(또는 타임아웃) 읽는다.
    /// poll로 블로킹 read가 테스트를 영원히 매달지 않게 한다.
    fn raw_read_until(fd: RawFd, needle: &[u8]) -> Vec<u8> {
        use nix::poll::{poll, PollFd, PollFlags};
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut acc = Vec::new();
        let mut buf = [0u8; 4096];
        while std::time::Instant::now() < deadline {
            let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match poll(&mut fds, remaining.as_millis().min(200) as i32) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => break,
            }
            match nix_read(fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    acc.extend_from_slice(&buf[..n]);
                    if acc.windows(needle.len()).any(|w| w == needle) {
                        return acc;
                    }
                }
                Err(nix::errno::Errno::EINTR) | Err(nix::errno::Errno::EAGAIN) => continue,
                Err(_) => break,
            }
        }
        acc
    }

    /// 실 `UnixListener`(run_daemon_inner) 데몬을 백그라운드로 띄우고 소켓
    /// 경로/작업 디렉터리를 돌려준다. 브로커 테스트는 control/data/wait에
    /// 여러 연결을 열어야 하므로(단일 소켓쌍 Harness로는 불가) 실 소켓을 쓴다.
    fn start_real_daemon() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ao-bk-{}", short_id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket_path = dir.join("s.sock");
        let socket_for_daemon = socket_path.clone();
        let hook: ShutdownHook = Arc::new(|| {}); // 테스트에선 프로세스를 죽이지 않는다.
        std::thread::spawn(move || {
            let _ = run_daemon_inner(socket_for_daemon, Duration::from_secs(60), hook);
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !socket_path.exists() {
            assert!(std::time::Instant::now() < deadline, "daemon never bound the socket");
            std::thread::sleep(Duration::from_millis(10));
        }
        (socket_path, dir)
    }

    fn connect_hello(socket_path: &std::path::Path) -> std::os::unix::net::UnixStream {
        use std::os::unix::net::UnixStream;
        let stream = UnixStream::connect(socket_path).unwrap();
        let fd = stream.as_raw_fd();
        protocol::write_frame(fd, &Message::Hello { proto: protocol::PROTO_VERSION }, None).unwrap();
        assert!(matches!(protocol::read_frame(fd).unwrap().0, Message::HelloOk { .. }));
        stream
    }

    fn spawn_broker(
        control_fd: RawFd,
        agent_id: &str,
        script: &str,
    ) {
        protocol::write_frame(
            control_fd,
            &Message::Spawn {
                agent_id: agent_id.into(),
                session_id: format!("s-{agent_id}"),
                shell: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                env: vec![("TERM".into(), "xterm-256color".into())],
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            protocol::read_frame(control_fd).unwrap().0,
            Message::SpawnOk { .. }
        ));
    }

    #[test]
    fn broker_spawn_registers_and_data_attach_echoes() {
        // Harness 단일 연결로 Spawn -> (테이블 등록 확인) -> DataAttach -> echo.
        let h = Harness::new();
        h.send(&Message::Hello { proto: protocol::PROTO_VERSION }, None);
        assert!(matches!(h.recv().0, Message::HelloOk { .. }));

        spawn_broker(h.client_fd, "a1", "printf READY; cat");
        wait_until(|| h.broker.lock().unwrap().contains_key("a1"));

        h.send(&Message::DataAttach { agent_id: "a1".into() }, None);
        assert!(matches!(h.recv().0, Message::DataAttachOk { .. }));

        // 백로그로 "READY"가 리플레이되어야 한다(스폰 시점부터 수집).
        let backlog = raw_read_until(h.client_fd, b"READY");
        assert!(
            backlog.windows(5).any(|w| w == b"READY"),
            "spawn-time output must replay on DataAttach: {backlog:?}"
        );

        // raw 입력 -> master -> cat 에코가 돌아온다.
        protocol::write_all_raw(h.client_fd, b"ping\n").unwrap();
        let echoed = raw_read_until(h.client_fd, b"ping");
        assert!(
            echoed.windows(4).any(|w| w == b"ping"),
            "input must round-trip through the broker PTY: {echoed:?}"
        );

        // 세션을 정리(자식 kill)해 데몬 스레드가 매달리지 않게.
        h.send(&Message::Kill { agent_id: "a1".into() }, None);
        // Kill 응답은 raw 스트림 중이라 프레임으로 안 오지만, 자식은 죽는다.
        let _ = close(h.client_fd);
    }

    #[test]
    fn broker_backlog_replays_losslessly_across_reattach() {
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "printf HELLO-BACKLOG; sleep 5");

        // 첫 DataAttach: 백로그 회수.
        let data1 = connect_hello(&socket_path);
        protocol::write_frame(data1.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(protocol::read_frame(data1.as_raw_fd()).unwrap().0, Message::DataAttachOk { .. }));
        let first = raw_read_until(data1.as_raw_fd(), b"HELLO-BACKLOG");
        assert!(first.windows(13).any(|w| w == b"HELLO-BACKLOG"));
        drop(data1); // detach(자식은 안 죽는다)

        // 재 DataAttach: 같은 백로그가 무손실 리플레이돼야 한다.
        let data2 = connect_hello(&socket_path);
        protocol::write_frame(data2.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(protocol::read_frame(data2.as_raw_fd()).unwrap().0, Message::DataAttachOk { .. }));
        let second = raw_read_until(data2.as_raw_fd(), b"HELLO-BACKLOG");
        assert!(
            second.windows(13).any(|w| w == b"HELLO-BACKLOG"),
            "reattach must replay the full backlog: {second:?}"
        );

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        drop(data2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_data_attach_after_snapshot_replays_only_post_snapshot_bytes() {
        // §P2-b: UpdateSnapshot이 그 시점 링 오프셋을 기록하면, 이후 DataAttach는
        // 스냅샷 이전 출력을 리플레이하지 않고 그 이후 바이트만 흘려야 한다
        // (앱은 스냅샷을 화면으로 별도 복원 -> 중복 없이 전체 스크롤백 복원).
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        // printf PRE -> read(입력 대기, 여기서 출력이 멎어 스냅샷 오프셋이 결정적) -> printf POST.
        spawn_broker(control.as_raw_fd(), "a1", "printf PREMARKER; read x; printf POSTMARKER");

        let data1 = connect_hello(&socket_path);
        protocol::write_frame(data1.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(protocol::read_frame(data1.as_raw_fd()).unwrap().0, Message::DataAttachOk { .. }));
        // PRE가 링에 전부 들어온 상태에서(셸이 read로 멎어 더 안 나옴) 스냅샷.
        let pre = raw_read_until(data1.as_raw_fd(), b"PREMARKER");
        assert!(pre.windows(9).any(|w| w == b"PREMARKER"));

        protocol::write_frame(
            control.as_raw_fd(),
            &Message::UpdateSnapshot {
                agent_id: "a1".into(),
                snapshot_b64: "c25hcA==".into(),
                offset: None,
                compressed: false,
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            protocol::read_frame(control.as_raw_fd()).unwrap().0,
            Message::UpdateSnapshotOk
        ));

        // 입력으로 read를 풀어 POST를 만든다(입력 에코 "go"도 스냅샷 이후라 무해).
        protocol::write_all_raw(data1.as_raw_fd(), b"go\n").unwrap();
        let _ = raw_read_until(data1.as_raw_fd(), b"POSTMARKER");

        // 재접속: 스냅샷 이후 바이트만 와야 한다 -- POST 포함, PRE 제외.
        let data2 = connect_hello(&socket_path);
        protocol::write_frame(data2.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(protocol::read_frame(data2.as_raw_fd()).unwrap().0, Message::DataAttachOk { .. }));
        let backlog2 = raw_read_until(data2.as_raw_fd(), b"POSTMARKER");
        assert!(
            backlog2.windows(10).any(|w| w == b"POSTMARKER"),
            "post-snapshot output must replay: {backlog2:?}"
        );
        assert!(
            !backlog2.windows(9).any(|w| w == b"PREMARKER"),
            "pre-snapshot output must NOT replay (snapshot_offset excludes it): {backlog2:?}"
        );

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        drop(data1);
        drop(data2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_resize_updates_geometry_in_list_and_attach() {
        // §P2-c: Resize 성공 시 rows/cols 메타가 갱신되어 List/Attach가 최신
        // 지오메트리를 반환해야 한다.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "sleep 30"); // 스폰 시 24x80

        protocol::write_frame(
            control.as_raw_fd(),
            &Message::Resize { agent_id: "a1".into(), rows: 50, cols: 200 },
            None,
        )
        .unwrap();
        assert!(matches!(protocol::read_frame(control.as_raw_fd()).unwrap().0, Message::ResizeOk));

        protocol::write_frame(control.as_raw_fd(), &Message::Attach { agent_id: "a1".into() }, None)
            .unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::AttachOk { rows, cols, .. } => {
                assert_eq!((rows, cols), (50, 200), "Attach must reflect the resize");
            }
            other => panic!("unexpected: {other:?}"),
        }

        protocol::write_frame(control.as_raw_fd(), &Message::List, None).unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::ListOk { sessions } => {
                let s = sessions.iter().find(|s| s.agent_id == "a1").expect("session in list");
                assert_eq!((s.rows, s.cols), (50, 200), "List must reflect the resize");
            }
            other => panic!("unexpected: {other:?}"),
        }

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_attach_returns_cleanup_paths_for_settings_recovery() {
        // 이슈 #40: 데몬이 Spawn 때 받은 cleanup_paths를 Attach 응답에 되돌려줘야
        // 입양 앱이 사라진 설정 파일을 복구할 수 있다.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        protocol::write_frame(
            control.as_raw_fd(),
            &Message::Spawn {
                agent_id: "a1".into(),
                session_id: "s-a1".into(),
                shell: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                env: vec![("TERM".into(), "xterm-256color".into())],
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec!["/tmp/ao-a1.settings.json".into()],
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            protocol::read_frame(control.as_raw_fd()).unwrap().0,
            Message::SpawnOk { .. }
        ));

        protocol::write_frame(control.as_raw_fd(), &Message::Attach { agent_id: "a1".into() }, None)
            .unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::AttachOk { cleanup_paths, .. } => {
                assert_eq!(cleanup_paths, vec!["/tmp/ao-a1.settings.json".to_string()]);
            }
            other => panic!("unexpected: {other:?}"),
        }

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_data_attach_honors_app_supplied_offset() {
        // §P1: UpdateSnapshot{offset:Some(k)}가 오면 데몬 수신 시점 ring.total()이
        // 아니라 앱이 준 오프셋을 snapshot_offset으로 써야 한다 -- 리플레이가 그
        // 오프셋부터 시작한다. printf로 결정적 8바이트("ABCDEFGH")를 만들고
        // offset=3을 주면 재접속 백로그는 "DEFGH"여야 한다.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "printf ABCDEFGH; sleep 5");

        let data1 = connect_hello(&socket_path);
        protocol::write_frame(data1.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data1.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { stream_offset: 0 }
        ));
        let _ = raw_read_until(data1.as_raw_fd(), b"ABCDEFGH");

        protocol::write_frame(
            control.as_raw_fd(),
            &Message::UpdateSnapshot {
                agent_id: "a1".into(),
                snapshot_b64: "c25hcA==".into(),
                offset: Some(3),
                compressed: false,
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            protocol::read_frame(control.as_raw_fd()).unwrap().0,
            Message::UpdateSnapshotOk
        ));

        let data2 = connect_hello(&socket_path);
        protocol::write_frame(data2.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data2.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { stream_offset: 3 }
        ));
        let backlog2 = raw_read_until(data2.as_raw_fd(), b"DEFGH");
        assert!(backlog2.windows(5).any(|w| w == b"DEFGH"));
        assert!(
            !backlog2.windows(3).any(|w| w == b"ABC"),
            "offset=3 must skip the first 3 bytes: {backlog2:?}"
        );

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        drop(data1);
        drop(data2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_data_attach_after_child_exit_gets_backlog_and_eof_without_blocking() {
        // §P2-b: waiter가 conn 정리를 마친(closed=true) 뒤 도착한 DataAttach는
        // 새 conn을 설치하지 않고 백로그+EOF만 주고 끝나야 한다(앱 reader 무한
        // 블록 방지). 즉시 종료하는 자식으로 재현.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "printf DONE-X");

        // Wait로 자식 reap + closed=true까지 확정적으로 기다린다.
        let waiter = connect_hello(&socket_path);
        protocol::write_frame(waiter.as_raw_fd(), &Message::Wait { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(waiter.as_raw_fd()).unwrap().0,
            Message::WaitOk { .. }
        ));

        // 종료 후 DataAttach: 백로그("DONE-X")를 받고 EOF(Ok(0))로 끝나야 한다.
        let data = connect_hello(&socket_path);
        protocol::write_frame(data.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { .. }
        ));
        // EOF까지 읽는다 -- 무한 블록이면 raw_read_until의 5초 데드라인에 걸려
        // 아래 assert가 실패한다.
        let got = raw_read_until(data.as_raw_fd(), b"DONE-X");
        assert!(got.windows(6).any(|w| w == b"DONE-X"), "backlog must arrive: {got:?}");
        // 이어 read하면 EOF(0)여야 한다(블록 아님).
        {
            use nix::poll::{poll, PollFd, PollFlags};
            let mut fds = [PollFd::new(data.as_raw_fd(), PollFlags::POLLIN)];
            let ready = poll(&mut fds, 2000).unwrap();
            assert!(ready > 0, "reader must not block after backlog -- expected EOF");
            let mut buf = [0u8; 64];
            let n = nix_read(data.as_raw_fd(), &mut buf).unwrap();
            // "DONE-X"가 한 번에 안 왔을 수도 있으니 0이거나 남은 바이트.
            if n > 0 {
                let n2 = nix_read(data.as_raw_fd(), &mut buf).unwrap_or(0);
                assert_eq!(n2, 0, "must reach EOF shortly after backlog");
            }
        }

        drop(control);
        drop(waiter);
        drop(data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// control 연결로 List를 보내 특정 agent의 `attached` 값을 읽는다(없으면 None).
    fn list_attached(control_fd: RawFd, agent_id: &str) -> Option<bool> {
        protocol::write_frame(control_fd, &Message::List, None).unwrap();
        match protocol::read_frame(control_fd).unwrap().0 {
            Message::ListOk { sessions } => {
                sessions.iter().find(|s| s.agent_id == agent_id).map(|s| s.attached)
            }
            other => panic!("unexpected reply to List: {other:?}"),
        }
    }

    #[test]
    fn broker_list_reports_attached_state_across_data_conn_lifecycle() {
        // §멀티인스턴스: List의 attached는 활성 data conn 유무를 반영해야 한다 --
        // 재접속(adopt)이 "다른 앱 인스턴스가 붙여 둔 세션"을 가로채지 않게 하는
        // 근거. 붙기 전 false -> DataAttach 후 true -> 끊으면 다시 false.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "sleep 30");

        assert_eq!(list_attached(control.as_raw_fd(), "a1"), Some(false));

        let data = connect_hello(&socket_path);
        protocol::write_frame(data.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { .. }
        ));
        // DataAttachOk를 받은 시점엔 conn 설치가 같은 io 락 안에서 끝났으므로 true.
        assert_eq!(list_attached(control.as_raw_fd(), "a1"), Some(true));

        // 크래시/정상종료 시뮬레이션: data 소켓을 닫으면 데몬이 conn을 떼어내
        // attached=false로 돌아가야 한다(그래야 다음 부팅에서 입양된다).
        drop(data);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while list_attached(control.as_raw_fd(), "a1") != Some(false) {
            assert!(
                std::time::Instant::now() < deadline,
                "attached must clear after the data conn drops"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_stalled_data_consumer_does_not_freeze_control_rpcs() {
        // §#48: 앱 data 소비자가 멈춰 소켓 송신버퍼가 가득 차도, reader 스레드가
        // io 락을 쥔 채 블로킹 write에 멈추면 안 된다 -- 그러면 List/Kill 등 다른
        // RPC가 전부 얼어붙는다. 출력 write를 전용 writer 스레드 + 바운드 큐로
        // 락 밖에 뺀 것의 회귀 테스트: 소비자가 멈춰도 (1) List가 즉시 응답하고,
        // (2) 큐가 차면 그 conn이 폐기돼 attached=false로 수렴하며, (3) Kill이
        // 얼지 않고 완료해야 한다.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        // 자식이 출력을 쉼없이 쏟아낸다(소켓 송신버퍼 + writer 큐를 빠르게 채운다).
        spawn_broker(control.as_raw_fd(), "a1", "yes ABCDEFGHIJKLMNOP");

        // data conn을 붙이되 DataAttachOk만 받고 이후 절대 읽지 않는다(멈춘 소비자).
        let data = connect_hello(&socket_path);
        protocol::write_frame(data.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { .. }
        ));
        // 소켓 송신버퍼와 writer 큐가 찰 시간을 준다(writer가 블로킹 write에 멈춘다).
        std::thread::sleep(Duration::from_millis(300));

        // control 연결의 List가 멈춘 소비자와 무관하게 유계 시간 안에 응답해야 한다.
        // 워치독 스레드로 각 호출을 감싸 io 락이 얼었다면 recv_timeout으로 잡는다.
        let control_fd = control.as_raw_fd();
        for _ in 0..5 {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(list_attached(control_fd, "a1"));
            });
            assert!(
                rx.recv_timeout(Duration::from_secs(3)).is_ok(),
                "List froze while a data consumer was stalled — io lock held during blocking write"
            );
            std::thread::sleep(Duration::from_millis(50));
        }

        // 멈춘 소비자의 conn은 큐가 차면 폐기돼 attached=false로 수렴한다.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while list_attached(control.as_raw_fd(), "a1") != Some(false) {
            assert!(
                std::time::Instant::now() < deadline,
                "a stalled data conn must be dropped once its queue fills -> attached clears"
            );
            std::thread::sleep(Duration::from_millis(50));
        }

        // Kill도 얼지 않고 완료해야 한다(멈춘 소비자가 io 락을 물지 않으므로).
        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(data);
        drop(control);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_shutdown_both_clears_attached_even_with_a_lingering_clone() {
        // §#50 P0: 앱은 data 소켓의 clone fd를 여러 개 쥔다(reader 스레드 clone +
        // writer + detach 핸들). detach는 `shutdown(Both)`로 소켓을 닫는데, 이는
        // 소켓(연결) 단위 연산이라 clone이 아직 살아 있어도 즉시 FIN을 보내
        // 데몬이 conn을 떼어내고 attached=false가 되어야 한다. clone을 하나 남긴
        // 채 shutdown해도 attached가 stale-true로 고착되지 않음을 못박는다 --
        // 매니저 detach의 `broker_data_shutdown`이 의존하는 바로 그 성질.
        use std::net::Shutdown;
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "sleep 30");

        let data = connect_hello(&socket_path);
        protocol::write_frame(data.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { .. }
        ));
        assert_eq!(list_attached(control.as_raw_fd(), "a1"), Some(true));

        // 앱 reader 스레드가 쥔 clone을 흉내: shutdown 후에도 살려 둔다.
        let lingering_clone = data.try_clone().unwrap();
        // detach: 한 clone에서 shutdown(Both) -> 연결 전체가 닫혀야 한다.
        data.shutdown(Shutdown::Both).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while list_attached(control.as_raw_fd(), "a1") != Some(false) {
            assert!(
                std::time::Instant::now() < deadline,
                "shutdown(Both) must clear attached even while a clone fd lingers"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        // conn이 정리됐으니 재입양(새 DataAttach)이 성공하고 attached가 다시 true.
        let data2 = connect_hello(&socket_path);
        protocol::write_frame(data2.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data2.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { .. }
        ));
        assert_eq!(list_attached(control.as_raw_fd(), "a1"), Some(true));

        drop(lingering_clone);
        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        drop(data2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_wait_returns_child_exit_code() {
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "exit 7");

        // Wait는 전용 연결에서(§설계) -- 자식 종료까지 블로킹.
        let waiter = connect_hello(&socket_path);
        protocol::write_frame(waiter.as_raw_fd(), &Message::Wait { agent_id: "a1".into() }, None)
            .unwrap();
        match protocol::read_frame(waiter.as_raw_fd()).unwrap().0 {
            Message::WaitOk { exit_code, .. } => assert_eq!(exit_code, Some(7)),
            other => panic!("unexpected: {other:?}"),
        }

        // 종료 후 Attach는 exit 정보를 실어 준다.
        protocol::write_frame(control.as_raw_fd(), &Message::Attach { agent_id: "a1".into() }, None)
            .unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::AttachOk { exit: Some(e), .. } => assert_eq!(e.exit_code, Some(7)),
            other => panic!("unexpected AttachOk without exit: {other:?}"),
        }

        drop(control);
        drop(waiter);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_kill_all_kills_every_session() {
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "sleep 30");
        spawn_broker(control.as_raw_fd(), "a2", "sleep 30");

        protocol::write_frame(control.as_raw_fd(), &Message::KillAll, None).unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::KillAllOk { killed } => assert_eq!(killed, 2),
            other => panic!("unexpected: {other:?}"),
        }

        // 이제 List는 브로커 세션을 하나도 담지 않아야 한다.
        protocol::write_frame(control.as_raw_fd(), &Message::List, None).unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::ListOk { sessions } => {
                assert!(sessions.iter().all(|s| !s.broker), "KillAll must empty the broker table");
            }
            other => panic!("unexpected: {other:?}"),
        }

        drop(control);
        let _ = std::fs::remove_dir_all(&dir);
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
    fn hello_negotiates_down_to_older_client_proto() {
        // 구프로토(v1) 클라이언트가 Hello{1}을 보내면 데몬은 HelloOk{1}로 답해
        // 그 버전으로 협상한다 -- 앱 업데이트 직후 신데몬 ↔ 구클라이언트 호환.
        let h = Harness::new();
        h.send(&Message::Hello { proto: 1 }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HelloOk { proto: 1 }));
        h.finish();
    }

    #[test]
    fn hello_clamps_future_proto_to_daemon_max() {
        // 미래 클라이언트(proto > PROTO_VERSION)는 데몬 상한으로 클램프된다 --
        // 그 클라이언트는 협상된 버전의 메시지만 보내므로 안전(forward-compat).
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
