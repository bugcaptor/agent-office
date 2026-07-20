// src-tauri/src/session/handoff_broker.rs
//
// v2 상시 브로커 모드 앱 쪽 분기(docs/session-broker-v2-design.md).
// 브로커 모드 매니저라도 세션은 두 종류가 섞일 수 있다: 데몬이 소유하는
// 브로커 세션(broker_owned)과, 데몬 접속 실패로 팩토리가 폴백 스폰한
// in-process 세션. "유지하고 종료"를 세션 단위로 가른다:
//   - broker_owned: 데몬이 자식을 이미 소유하므로 **스냅샷 업로드 후
//     detach**(맵에서만 떼어내 dispose_all이 Kill하지 않게 함).
//   - 폴백 세션: 앱이 fd를 쥐고 있으므로 **기존 v1 fd 핸드오프**(handoff_v1.rs)로
//     넘긴다.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::manager::{Session, SessionManager};
use crate::types::*;

impl SessionManager {
    /// 하나의 connect_or_spawn 연결로 두 경로를 모두 처리한다 -- 데몬은
    /// proto 2라 v1 Handoff와 v2 UpdateSnapshot을 같은 연결에서 받는다.
    #[cfg(unix)]
    pub(super) fn handoff_all_broker(
        &self,
        snapshots: &std::collections::HashMap<String, String>,
        rendered_bytes: &std::collections::HashMap<String, u64>,
    ) -> usize {
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
            crate::sessiond::client::connect_or_spawn(&socket_path, &exe_path, &log_path).ok();
        let mut count = 0;
        for agent_id in ids {
            let Some(sess) = self.find(&agent_id) else { continue };
            if sess.handed_off.load(Ordering::SeqCst) {
                continue;
            }
            if sess.broker_owned {
                // 브로커 세션: 최신 스냅샷 업로드(best-effort) 후 detach. 데몬에
                // 못 닿아도 detach는 진행해야 dispose_all이 자식을 죽이지 않는다.
                if let (Some(client), Some(snap)) = (&client, snapshots.get(agent_id.as_str())) {
                    let offset = snapshot_offset(&sess, rendered_bytes.get(agent_id.as_str()).copied());
                    let _ = client.update_snapshot(&agent_id, snap.as_bytes(), offset);
                }
                sess.handed_off.store(true, Ordering::SeqCst);
                // data 소켓을 결정적으로 shutdown: reader 스레드를 EOF로 종료시키고
                // 데몬에 FIN을 보내 conn을 정리시킨다(§#50 선결). 이게 없으면 reader
                // 스레드가 clone fd를 프로세스 종료까지 쥐어 데몬 conn이 살아 있고
                // List `attached`가 stale-true로 고착돼, 다음 인스턴스가 라이브 소유로
                // 오판하거나 같은 프로세스 재입양이 깨진다.
                if let Some(sd) = sess.broker_data_shutdown.lock().take() {
                    sd.shutdown();
                }
                // 데몬이 FIN을 관측해 conn을 떼어낼 짧은 여유(handoff_one과 동일 패턴).
                // 실제 앱 종료 시엔 이후 프로세스가 죽어 무관하나, 같은 프로세스에서
                // 곧바로 재입양하는 경우 attached=false로 수렴할 시간을 준다.
                std::thread::sleep(std::time::Duration::from_millis(20));
                self.sessions.lock().remove(&agent_id);
                self.registry.remove(&sess.session_id);
                count += 1;
            } else if let Some(client) = &client {
                // 폴백(in-process) 세션: 기존 v1 fd 핸드오프(reader 인터럽트 →
                // fd 전송 → 맵 제거). 스냅샷이 없으면 handoff_one이 backlog로 폴백.
                let snapshot = snapshots
                    .get(agent_id.as_str())
                    .map(|s| s.clone().into_bytes())
                    .unwrap_or_default();
                if self.handoff_one(&agent_id, client, snapshot) {
                    count += 1;
                }
            }
        }
        count
    }

    /// 주기 스냅샷 업로드(브로커 모드 전용). 렌더러가 30초마다 직렬화한 화면을
    /// 데몬에 올려 앱 크래시 후에도 마지막 화면을 복원할 수 있게 한다.
    /// 브로커 모드가 아니거나 데몬에 못 닿으면 no-op.
    #[cfg(unix)]
    pub fn upload_snapshots(
        &self,
        snapshots: &std::collections::HashMap<String, String>,
        rendered_bytes: &std::collections::HashMap<String, u64>,
    ) {
        if !self.broker_mode() {
            return;
        }
        let Some(app_data_dir) = &self.app_data_dir else {
            return;
        };
        let Ok(client) = crate::session::broker_pty::connect(app_data_dir) else {
            return;
        };
        for (agent_id, snap) in snapshots {
            // 데몬 테이블에 없는 agentId면 no-op으로 무시된다(안전). 스냅샷 offset은
            // base(attach 시 stream_offset) + 렌더러가 실제 렌더한 raw 바이트 누적치로
            // 동봉해 유실 창을 없앤다(§#49) -- 렌더러 누적치가 없으면 None(데몬은
            // 수신 시점 ring.total()로 폴백).
            let offset = self
                .find(agent_id)
                .and_then(|s| snapshot_offset(&s, rendered_bytes.get(agent_id).copied()));
            let _ = client.update_snapshot(agent_id, snap.as_bytes(), offset);
        }
    }

    #[cfg(not(unix))]
    pub fn upload_snapshots(
        &self,
        _snapshots: &std::collections::HashMap<String, String>,
        _rendered_bytes: &std::collections::HashMap<String, u64>,
    ) {
    }

    /// 브로커 모드 재접속: List를 훑어 세션 종류별로 되찾는다 -- **broker=true는
    /// Attach+DataAttach(브로커 경로)로, broker=false(v1 핸드오프/폴백 세션)는
    /// 기존 v1 adopt(adopt_one, fd 회수)로** 입양한다. 후자는 이전 실행이 폴백
    /// 스폰한 세션을 v1 fd 핸드오프로 넘긴 경우나, 브로커로 업그레이드하기 전
    /// 남아 있던 세션을 커버한다(협상 p=1인 구데몬 상대로는 애초에 broker 항목이
    /// 없으니 자연히 v1만 처리된다). exited 항목은 스킵.
    #[cfg(unix)]
    pub(super) fn adopt_detached_broker(
        self: &Arc<Self>,
        known_agent_ids: &std::collections::HashSet<String>,
    ) -> Vec<AdoptedSessionInfo> {
        let Some(app_data_dir) = self.app_data_dir.clone() else {
            return Vec::new();
        };
        if !crate::session::broker_pty::socket_exists(&app_data_dir) {
            return Vec::new();
        }
        let Ok(client) = crate::session::broker_pty::connect(&app_data_dir) else {
            return Vec::new();
        };
        let sessions = client.list().unwrap_or_default();
        let mut adopted = Vec::new();
        for info in sessions {
            if info.exited {
                // 종료된 브로커 세션은 best-effort Kill로 데몬 테이블에서 치운다
                // (§P2-a) -- detach 중 자식이 죽으면 exited 엔트리가 영원히 남아
                // 데몬의 table-empty 종료를 막는 누수가 된다. v1 exited 항목은
                // 기존대로 스킵(v1 Adopt/Kill 수명 규칙 유지).
                if info.broker {
                    let _ = client.kill(&info.agent_id);
                }
                continue;
            }
            // §#50: 다른 앱 인스턴스가 지금 활성 data conn을 붙여 둔 세션
            // (info.attached)은 입양하지 않는다 -- 입양하면 DataAttach 교체로
            // 데몬이 그 인스턴스의 data 소켓을 shutdown해 원본 터미널이 먹통이
            // 된다(앱은 단일 인스턴스 강제가 없다). detach가 이제 소켓을 결정적
            // shutdown하므로(broker_data_shutdown) 정상 재시작/크래시(프로세스
            // 종료로 OS가 fd를 닫음)면 데몬이 conn을 정리해 attached=false가 되어
            // 여기서 정상 입양된다. attached=true는 "살아 있는 다른 인스턴스 소유".
            // v1 세션은 데몬이 항상 attached=false로 주므로 영향 없다.
            // TOCTOU(List~DataAttach 창)는 수용: 두 인스턴스가 ms 창에서 같은
            // 미소유 세션을 경합해도 데몬 gen 직렬화로 크래시 없이 last-wins 수렴.
            if info.attached {
                eprintln!(
                    "agent-office: skip adopt of {} — attached by another live instance",
                    info.agent_id
                );
                continue;
            }
            if !known_agent_ids.contains(&info.agent_id) {
                let _ = client.kill(&info.agent_id); // 삭제된 에이전트의 고아 세션 정리.
                continue;
            }
            let result = if info.broker {
                self.adopt_one_broker(&app_data_dir, &info, &client)
            } else {
                // v1 핸드오프/폴백 세션은 기존 fd 회수 경로로 입양한다(공유 연결 사용).
                self.adopt_one(&info.agent_id, &client)
            };
            if let Some(r) = result {
                adopted.push(r);
            }
        }
        adopted
    }

    /// 브로커 세션 하나 입양: Attach로 메타/스냅샷을 회수하고 DataAttach로
    /// 백로그 리플레이 스트림을 붙인다. 종료는 BrokerWaiter(Wait RPC)가 실제
    /// exit code로 관측한다(v1 EofWaiter의 "exit code 소실" 제약 해소).
    #[cfg(unix)]
    fn adopt_one_broker(
        self: &Arc<Self>,
        app_data_dir: &std::path::Path,
        info: &crate::sessiond::protocol::SessionInfo,
        client: &crate::sessiond::client::Client,
    ) -> Option<AdoptedSessionInfo> {
        let (spawned, meta) =
            match crate::session::broker_pty::assemble_broker_adopted(app_data_dir, &info.agent_id) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("agent-office: broker adopt failed for {}: {e}", info.agent_id);
                    return None;
                }
            };
        // List와 Attach 사이에 자식이 죽었으면(경합) 입양하지 않는다. 이때
        // 데몬 테이블엔 exited 엔트리가 남아 table-empty 종료를 막으므로,
        // best-effort Kill로 치운다(입양은 boot 때 1회뿐이라 나중에 dispose할
        // 매니저 세션이 안 생겨 방치되면 영구 잔류한다).
        if meta.exit.is_some() {
            let _ = client.kill(&info.agent_id);
            return None;
        }
        // Attach가 준 라이브 크기를 우선(리사이즈 후 List가 낡았을 수 있다).
        let size = (meta.cols, meta.rows);
        // 화면 복원: 업로드된 스냅샷이 있으면 항상 initial_output으로 주입한다.
        // 데몬은 그 스냅샷 시점 이후의 링버퍼 바이트만 data 연결로 리플레이하므로
        // (snapshot_offset 기반), "스냅샷 + 이후 출력"이 되어 중복 없이 전체
        // 스크롤백이 복원된다. 스냅샷이 한 번도 업로드 안 됐으면 데몬이 링 전체를
        // 리플레이하고 meta.snapshot은 비어 있어 주입하지 않는다.
        let initial_output = (!meta.snapshot.is_empty()).then_some(meta.snapshot);
        // 이슈 #40: 삭제 소유권은 데몬이 유지하되(앱 install_session엔 빈 벡터를
        // 넘긴다), 앱이 꺼진 사이 사라졌을 수 있는 observer 설정 파일은 데몬이
        // 돌려준 cleanup_paths로 입양 시점에 멱등 재작성한다.
        let restore_paths: Vec<std::path::PathBuf> =
            meta.cleanup_paths.iter().map(std::path::PathBuf::from).collect();
        self.observer
            .restore_session_artifacts(&info.session_id, &restore_paths);
        // cleanup_paths는 데몬이 Spawn 때 받아 보관·정리하므로 앱 쪽은 비운다.
        let (session, _started) = self.install_session(
            info.session_id.clone(),
            info.agent_id.clone(),
            Vec::new(),
            info.cwd.clone(),
            size,
            spawned,
            None, // eof_stop_gate: 브로커는 Wait RPC로 종료를 관측한다.
            initial_output,
        );
        Some(AdoptedSessionInfo {
            agent_id: info.agent_id.clone(),
            session_id: session.session_id.clone(),
            rows: size.1,
            cols: size.0,
        })
    }
}

/// 스냅샷 업로드/핸드오프에 실을 절대 offset을 계산한다(§#49).
/// `base`(attach 시 DataAttachOk가 준 stream_offset, 세션당 고정) + `rendered`
/// (렌더러가 실제 렌더/소비한 raw 스트림 바이트 누적치). 렌더러 누적치가
/// 없으면(프론트가 그 세션 값을 안 실어 보냄) None을 반환해 데몬이 수신 시점
/// ring.total()로 폴백하게 한다(trim 과다 위험은 값이 아예 없을 때만).
#[cfg(unix)]
fn snapshot_offset(sess: &Session, rendered: Option<u64>) -> Option<u64> {
    let rendered = rendered?;
    let base = sess
        .broker_stream_offset
        .as_ref()
        .map(|c| c.load(Ordering::SeqCst))
        .unwrap_or(0);
    Some(base + rendered)
}
