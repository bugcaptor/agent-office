// src-tauri/src/session/handoff_v1.rs
//
// 세션 핸드오프 v1 경로(설계 문서 §핵심 3, 4): 앱이 종료할 때 마스터 fd
// 자체를 sessiond에 넘겨 세션을 살려 둔다. 브로커 모드(v2, handoff_broker.rs)가
// 아닐 때 항상 이 경로를 타고, 브로커 모드에서도 데몬 접속 실패로 factory가
// in-process 폴백 스폰한 세션은 이 경로로 넘긴다(handoff_all_broker의
// 폴백 분기 참조).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::manager::SessionManager;
use crate::types::*;

impl SessionManager {
    /// 앱 quit(§핵심 3): Running 세션들을 sessiond로 넘긴다. `snapshots`는
    /// agentId -> 프론트가 종료 직전 직렬화한 xterm 화면(스크롤백 포함) --
    /// 데몬은 핸드오프 *이후* 출력만 링버퍼에 담으므로, 이게 없으면 재입양
    /// 후 종료 전 화면(예: ls 결과)이 사라진다(실증에서 발견된 빈틈).
    /// 반환값은 성공 개수 -- 프론트는 이 수와 무관하게 종료를 진행한다.
    /// `app_data_dir`이 없으면(테스트 등) 0.
    #[cfg(unix)]
    pub fn handoff_all(
        &self,
        snapshots: &std::collections::HashMap<String, String>,
        rendered_bytes: &std::collections::HashMap<String, u64>,
    ) -> usize {
        if self.broker_mode() {
            return self.handoff_all_broker(snapshots, rendered_bytes);
        }
        let Some(app_data_dir) = self.app_data_dir.clone() else {
            return 0;
        };
        let ids: Vec<AgentId> = {
            let map = self.sessions.lock();
            map.iter()
                .filter(|(_, s)| *s.state.lock() == SessionState::Running)
                .map(|(a, _)| a.clone())
                .collect()
        };
        if ids.is_empty() {
            return 0;
        }

        let socket_path = crate::sessiond::client::default_socket_path(&app_data_dir);
        let log_path = crate::sessiond::client::default_log_path(&app_data_dir);
        let exe_path = std::env::current_exe().unwrap_or_default();
        let client =
            match crate::sessiond::client::connect_or_spawn(&socket_path, &exe_path, &log_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("agent-office: handoff_all could not reach sessiond: {e}");
                    return 0;
                }
            };

        ids.iter()
            .filter(|agent_id| {
                let snapshot = snapshots
                    .get(agent_id.as_str())
                    .map(|s| s.clone().into_bytes())
                    .unwrap_or_default();
                self.handoff_one(agent_id, &client, snapshot)
            })
            .count()
    }

    #[cfg(not(unix))]
    pub fn handoff_all(
        &self,
        _snapshots: &std::collections::HashMap<String, String>,
        _rendered_bytes: &std::collections::HashMap<String, u64>,
    ) -> usize {
        0
    }

    /// 세션 하나를 넘긴다. 설계 문서 §핵심 3의 순서 그대로: 리더 인터럽트 →
    /// handed_off set → 전송. 실패해도 세션은 그대로 둔다(맵에 남고
    /// handed_off=true) -- 앱은 어차피 곧 종료되므로 마스터 fd가 닫히며
    /// SIGHUP으로 자연 정리된다(설계 문서 "왜 이 방식인가" 참조).
    ///
    /// `snapshot`이 비어 있으면(프론트가 이 터미널을 한 번도 구독하지 않아
    /// 직렬화 대상이 없었던 경우 등) sink의 backlog를 폴백으로 쓴다 --
    /// 실증에서 발견된 빈틈 수정: 그래야 아직 한 번도 열지 않은 터미널도
    /// 재입양 후 종료 전 출력이 최소한 backlog 분량만큼은 보존된다.
    ///
    /// pub(super): handoff_broker.rs의 handoff_all_broker가 폴백(in-process)
    /// 세션 분기에서 재사용한다(v1 경로 그대로).
    #[cfg(unix)]
    pub(super) fn handoff_one(
        &self,
        agent_id: &str,
        client: &crate::sessiond::client::Client,
        snapshot: Vec<u8>,
    ) -> bool {
        let Some(sess) = self.find(agent_id) else {
            return false;
        };
        if sess.handed_off.load(Ordering::SeqCst) {
            return false;
        }
        let Some(handoff) = sess.handoff.lock().take() else {
            return false; // Fake/입양 조립 실패 등으로 handoff 정보가 없는 세션은 핸드오프 불가.
        };

        // 재핸드오프(입양 세션)라면 EofWaiter 오발화를 막는다.
        if let Some(gate) = &sess.eof_stop_gate {
            gate.store(true, Ordering::SeqCst);
        }
        if let Some(interrupt) = sess.reader_interrupt.lock().take() {
            interrupt.interrupt();
        }
        // poll 기반 리더는 인터럽트를 수 ms 내 관측한다 -- fd를 보내기 전에
        // 짧게 양보해 리더 스레드가 실제로 빠져나갈 시간을 준다(완료 채널을
        // 새로 두는 것보다 훨씬 단순하고, 실패해도 안전 — 최악의 경우 데몬이
        // 아주 잠깐 늦게 도착한 잔여 바이트를 이어 읽을 뿐 유실은 없다).
        std::thread::sleep(std::time::Duration::from_millis(20));

        sess.handed_off.store(true, Ordering::SeqCst);

        let pid = handoff.pid;
        let pgid = handoff.pgid;
        let master_fd = handoff.take_master_fd();
        let (cols, rows) = *sess.size.lock();
        let cleanup_paths = sess
            .cleanup_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let snapshot = if snapshot.is_empty() {
            self.sink_for(agent_id).backlog_snapshot()
        } else {
            snapshot
        };

        let result = client.handoff(crate::sessiond::client::HandoffRequest {
            agent_id: agent_id.to_string(),
            session_id: sess.session_id.clone(),
            pid,
            pgid,
            rows,
            cols,
            cwd: sess.cwd.clone(),
            cleanup_paths,
            snapshot,
            master_fd,
        });

        match result {
            Ok(()) => {
                self.sessions.lock().remove(agent_id);
                self.registry.remove(&sess.session_id);
                true
            }
            Err(e) => {
                eprintln!("agent-office: handoff failed for {agent_id}: {e}");
                let _ = nix::unistd::close(master_fd);
                false
            }
        }
    }

    /// 부트스트랩(§핵심 4): sessiond에 남아 있는 세션들을 되찾는다.
    /// `known_agent_ids`는 영속 프로필의 agentId 집합 -- 여기 없는 항목은
    /// Kill 지시(삭제된 에이전트의 고아 claude 방지), exited 항목은 스킵.
    /// 소켓이 없거나 연결 실패면 빈 벡터(데몬을 새로 스폰하지 않는다 --
    /// 입양할 게 없으면 없는 대로다).
    #[cfg(unix)]
    pub fn adopt_detached(
        self: &Arc<Self>,
        known_agent_ids: &std::collections::HashSet<String>,
    ) -> Vec<AdoptedSessionInfo> {
        if self.broker_mode() {
            return self.adopt_detached_broker(known_agent_ids);
        }
        let Some(app_data_dir) = &self.app_data_dir else {
            return Vec::new();
        };
        let socket_path = crate::sessiond::client::default_socket_path(app_data_dir);
        if !socket_path.exists() {
            return Vec::new();
        }
        let client = match crate::sessiond::client::Client::connect(&socket_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let sessions = match client.list() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut adopted = Vec::new();
        for info in sessions {
            if info.exited {
                continue;
            }
            if !known_agent_ids.contains(&info.agent_id) {
                let _ = client.kill(&info.agent_id);
                continue;
            }
            if let Some(result) = self.adopt_one(&info.agent_id, &client) {
                adopted.push(result);
            }
        }
        adopted
    }

    #[cfg(not(unix))]
    pub fn adopt_detached(
        self: &Arc<Self>,
        _known_agent_ids: &std::collections::HashSet<String>,
    ) -> Vec<AdoptedSessionInfo> {
        Vec::new()
    }

    /// 세션 하나를 입양해 install_session으로 재배선한다. 실패하면 None --
    /// 그 세션은 데몬 테이블에 그대로 남아 다음 재시작에서 다시 시도할 수
    /// 있다(이번 연결에서 이미 Adopt를 보낸 뒤 실패했다면 데몬 쪽에선 이미
    /// 테이블에서 빠진 상태이므로 fd 자체는 유실 -- assemble_adopted 실패는
    /// 극히 드문 경로라 이 트레이드오프를 받아들인다).
    ///
    /// pub(super): handoff_broker.rs의 adopt_detached_broker가 v1
    /// 핸드오프/폴백 세션 분기에서 재사용한다.
    #[cfg(unix)]
    pub(super) fn adopt_one(
        self: &Arc<Self>,
        agent_id: &str,
        client: &crate::sessiond::client::Client,
    ) -> Option<AdoptedSessionInfo> {
        let adopted = client.adopt(agent_id).ok()?;
        let (spawned, stop_gate) = match crate::session::pty_factory::assemble_adopted(
            adopted.master_fd,
            adopted.pid,
            adopted.pgid,
        ) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("agent-office: failed to assemble adopted session {agent_id}: {e}");
                let _ = nix::unistd::close(adopted.master_fd);
                return None;
            }
        };
        let cleanup_paths: Vec<std::path::PathBuf> =
            adopted.cleanup_paths.iter().map(std::path::PathBuf::from).collect();
        // 이슈 #40: 앱이 꺼진 사이 사라졌을 수 있는 observer 설정 파일을 입양
        // 시점에 멱등 재작성한다. cleanup_paths가 비면(observer OFF 세션) no-op.
        self.observer
            .restore_session_artifacts(&adopted.session_id, &cleanup_paths);
        let size = (adopted.cols, adopted.rows);
        // 종료 직전 화면 스냅샷 -> 핸드오프 이후 링버퍼 순으로 이어붙인다
        // (실증에서 발견된 빈틈 수정) -- 순서가 바뀌면 화면이 뒤죽박죽으로
        // 재생된다. install_session이 빈 벡터는 initial_output 주입 자체를
        // 건너뛰므로 둘 다 없을 때를 따로 가릴 필요가 없다.
        let mut initial_output = adopted.snapshot;
        initial_output.extend_from_slice(&adopted.buffer);
        let (session, _started) = self.install_session(
            adopted.session_id,
            agent_id.to_string(),
            cleanup_paths,
            adopted.cwd,
            size,
            spawned,
            Some(stop_gate),
            Some(initial_output),
        );
        Some(AdoptedSessionInfo {
            agent_id: agent_id.to_string(),
            session_id: session.session_id.clone(),
            rows: size.1,
            cols: size.0,
        })
    }
}
