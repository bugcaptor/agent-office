// src-tauri/src/peer/pairing.rs
//
// 피어 페어링(#7k §결정 5)과 토큰 보관.
//
//   호스트: `peer-tokens.json`(0600) — 내가 승인해 준 뷰어들(peerId·토큰·권한).
//   뷰어  : `peer-hosts.json`(0600) — 내가 붙을 호스트들(주소·토큰).
//
// 페어링은 2단계다. (1) 뷰어가 `pair/start`를 치면 호스트 앱 UI에 6자리 코드와
// 승인 다이얼로그가 뜬다. (2) 사람이 승인(+권한 선택)하고, 뷰어가 그 코드로
// `pair/complete`를 치면 토큰이 발급된다. 코드는 "호스트 화면을 본 사람"만
// 완료할 수 있게 하는 사람-루프 장치이고, 승인 클릭은 권한을 고르는 자리다.
// 코드 3회 오입력이면 그 페어링은 폐기한다.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::protocol::{PeerClientKind, PeerPermission};

/// 페어링 코드 유효 시간.
pub const PAIRING_TTL: Duration = Duration::from_secs(120);
/// 코드 오입력 허용 횟수.
const MAX_CODE_ATTEMPTS: u8 = 3;
/// 동시에 대기할 수 있는 페어링 수 — 승인 다이얼로그 폭주 방지.
const MAX_PENDING: usize = 5;

/// 발급 토큰 수명(쿠키 Max-Age와 같은 값). 폰은 오래 열어 두므로 짧으면
/// 짜증이고, 길면 분실 위험이라 30일로 잡고 기기별 폐기를 실질 대안으로 둔다.
pub const TOKEN_MAX_AGE_SECS: u64 = 30 * 24 * 3600;

// ── 호스트: 발급 토큰 ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerRecord {
    pub peer_id: String,
    /// 뷰어가 자기 소개로 보낸 이름(설정 UI 표시용).
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub permission: PeerPermission,
    pub created_at: u64,
    /// 앱↔앱 뷰어인가 브라우저인가. 가시성 규칙이 갈린다(웹 호스팅 #7m).
    /// 기존 토큰 파일에는 없으므로 default = `Peer`(역호환).
    #[serde(default)]
    pub kind: PeerClientKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PeerTokensFile {
    #[serde(default)]
    peers: Vec<PeerRecord>,
}

/// 호스트가 발급한 토큰 목록. control의 `control-token`과 같은 규칙(0600,
/// 매 요청 파일 대조)이라 승인/취소가 서버 재시작 없이 즉시 반영된다.
pub struct PeerTokenStore {
    path: PathBuf,
}

impl PeerTokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Vec<PeerRecord> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        serde_json::from_str::<PeerTokensFile>(&text)
            .map(|f| f.peers)
            .unwrap_or_default()
    }

    fn save(&self, peers: &[PeerRecord]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&PeerTokensFile {
            peers: peers.to_vec(),
        })
        .unwrap_or_else(|_| "{}".into());
        std::fs::write(&self.path, text)?;
        super::set_owner_only(&self.path);
        Ok(())
    }

    pub fn insert(&self, record: PeerRecord) -> std::io::Result<()> {
        let mut peers = self.load();
        peers.retain(|p| p.peer_id != record.peer_id);
        peers.push(record);
        self.save(&peers)
    }

    /// 제시된 토큰과 일치하는 피어(상수시간 비교). 없으면 None = 401.
    pub fn authenticate(&self, token: &str) -> Option<PeerRecord> {
        self.load()
            .into_iter()
            .find(|p| super::ct_eq(p.token.as_bytes(), token.as_bytes()))
    }

    pub fn remove(&self, peer_id: &str) -> std::io::Result<()> {
        let mut peers = self.load();
        peers.retain(|p| p.peer_id != peer_id);
        self.save(&peers)
    }

    pub fn set_permission(&self, peer_id: &str, permission: PeerPermission) -> std::io::Result<()> {
        let mut peers = self.load();
        for p in peers.iter_mut() {
            if p.peer_id == peer_id {
                p.permission = permission;
            }
        }
        self.save(&peers)
    }
}

// ── 뷰어: 저장된 호스트 ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerHostRecord {
    pub peer_id: String,
    /// 사람이 읽는 이름(호스트가 알려준 hostName).
    pub label: String,
    /// `host:port`.
    pub address: String,
    pub token: String,
    #[serde(default)]
    pub permission: PeerPermission,
    /// 앱 시작 시 자동 연결할지. 기본 true.
    #[serde(default = "default_true")]
    pub auto_connect: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PeerHostsFile {
    #[serde(default)]
    hosts: Vec<PeerHostRecord>,
}

pub struct PeerHostStore {
    path: PathBuf,
}

impl PeerHostStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Vec<PeerHostRecord> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        serde_json::from_str::<PeerHostsFile>(&text)
            .map(|f| f.hosts)
            .unwrap_or_default()
    }

    fn save(&self, hosts: &[PeerHostRecord]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&PeerHostsFile {
            hosts: hosts.to_vec(),
        })
        .unwrap_or_else(|_| "{}".into());
        std::fs::write(&self.path, text)?;
        super::set_owner_only(&self.path);
        Ok(())
    }

    pub fn insert(&self, record: PeerHostRecord) -> std::io::Result<()> {
        let mut hosts = self.load();
        hosts.retain(|h| h.peer_id != record.peer_id);
        hosts.push(record);
        self.save(&hosts)
    }

    pub fn remove(&self, peer_id: &str) -> std::io::Result<()> {
        let mut hosts = self.load();
        hosts.retain(|h| h.peer_id != peer_id);
        self.save(&hosts)
    }
}

// ── 진행 중인 페어링 ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingDecision {
    Pending,
    Approved(PeerPermission),
    Rejected,
}

#[derive(Debug, Clone)]
pub struct PendingPairing {
    pub pairing_id: String,
    pub code: String,
    pub viewer_name: String,
    /// 승인 다이얼로그가 "웹 브라우저"인지 "다른 사무실"인지 구분해 보여준다.
    pub client_kind: PeerClientKind,
    pub decision: PairingDecision,
    attempts: u8,
    started: Instant,
}

impl PendingPairing {
    /// TTL까지 남은 시간(ms). 승인 다이얼로그가 만료된 코드를 계속 띄우지
    /// 않도록 렌더러에 같이 내려 준다 — 이벤트로 밀어 준 항목은 나이를
    /// 알 길이 없어서 클라이언트가 스스로 지울 근거가 필요하다.
    pub fn remaining_ms(&self) -> u64 {
        PAIRING_TTL.saturating_sub(self.started.elapsed()).as_millis() as u64
    }
}

/// `pair/complete`의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingOutcome {
    /// 승인 완료 — 토큰을 발급하면 된다.
    Approved(PeerPermission),
    /// 아직 사람이 승인/거부를 안 눌렀다. 뷰어는 잠시 후 재시도한다.
    AwaitingApproval,
    Rejected,
    /// 코드 불일치(남은 시도 횟수).
    WrongCode { remaining: u8 },
    /// 만료·불명·시도 초과 — 페어링을 처음부터 다시 해야 한다.
    Expired,
}

#[derive(Default)]
pub struct PairingState {
    pending: Mutex<HashMap<String, PendingPairing>>,
}

impl PairingState {
    /// 새 페어링을 열고 (pairingId, 6자리 코드)를 만든다. 동시 대기가 상한을
    /// 넘으면 `None` — 승인 다이얼로그를 폭주시키는 것 자체가 공격이다.
    pub fn start(&self, viewer_name: &str, client_kind: PeerClientKind) -> Option<PendingPairing> {
        self.sweep();
        if self.pending.lock().len() >= MAX_PENDING {
            return None;
        }
        let entry = PendingPairing {
            pairing_id: uuid::Uuid::new_v4().simple().to_string(),
            code: random_code(),
            viewer_name: viewer_name.to_string(),
            client_kind,
            decision: PairingDecision::Pending,
            attempts: 0,
            started: Instant::now(),
        };
        self.pending
            .lock()
            .insert(entry.pairing_id.clone(), entry.clone());
        Some(entry)
    }

    /// 호스트 UI가 표시할 대기 목록.
    pub fn list(&self) -> Vec<PendingPairing> {
        self.sweep();
        let mut v: Vec<PendingPairing> = self.pending.lock().values().cloned().collect();
        v.sort_by(|a, b| a.started.cmp(&b.started));
        v
    }

    /// 사람이 승인(권한 선택). 없는 id면 false.
    pub fn approve(&self, pairing_id: &str, permission: PeerPermission) -> bool {
        let mut pending = self.pending.lock();
        match pending.get_mut(pairing_id) {
            Some(p) => {
                p.decision = PairingDecision::Approved(permission);
                true
            }
            None => false,
        }
    }

    pub fn reject(&self, pairing_id: &str) -> bool {
        let mut pending = self.pending.lock();
        match pending.get_mut(pairing_id) {
            Some(p) => {
                p.decision = PairingDecision::Rejected;
                true
            }
            None => false,
        }
    }

    /// 뷰어가 코드를 제시했다. 승인까지 끝났으면 Approved를 돌려주고 그
    /// 페어링을 소비한다.
    pub fn complete(&self, pairing_id: &str, code: &str) -> PairingOutcome {
        self.sweep();
        let mut pending = self.pending.lock();
        let Some(entry) = pending.get_mut(pairing_id) else {
            return PairingOutcome::Expired;
        };
        if !super::ct_eq(entry.code.as_bytes(), code.trim().as_bytes()) {
            entry.attempts += 1;
            let remaining = MAX_CODE_ATTEMPTS.saturating_sub(entry.attempts);
            if remaining == 0 {
                pending.remove(pairing_id);
                return PairingOutcome::Expired;
            }
            return PairingOutcome::WrongCode { remaining };
        }
        match entry.decision {
            PairingDecision::Pending => PairingOutcome::AwaitingApproval,
            PairingDecision::Rejected => {
                pending.remove(pairing_id);
                PairingOutcome::Rejected
            }
            PairingDecision::Approved(permission) => {
                pending.remove(pairing_id);
                PairingOutcome::Approved(permission)
            }
        }
    }

    /// TTL 지난 항목 정리.
    fn sweep(&self) {
        self.pending
            .lock()
            .retain(|_, p| p.started.elapsed() < PAIRING_TTL);
    }
}

// ── 레이트리밋 ────────────────────────────────────────────────────────
//
// 브라우저 클라이언트가 생기면 페어링 표면이 커진다(#7m §4). 코드 3회 오입력
// 폐기만으로는 "새 페어링을 계속 열어 코드를 무한 시도"를 못 막으므로, 시작
// 시도와 인증 실패를 IP별 슬라이딩 윈도로 센다.

/// `pair/start` 허용 횟수와 창.
const START_LIMIT: usize = 3;
const START_WINDOW: Duration = Duration::from_secs(60);
/// 인증 실패(코드 오입력·토큰 무효) 허용 횟수와 창.
const AUTH_FAIL_LIMIT: usize = 10;
const AUTH_FAIL_WINDOW: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct PairRateLimiter {
    starts: Mutex<HashMap<std::net::IpAddr, VecDeque<Instant>>>,
    failures: Mutex<HashMap<std::net::IpAddr, VecDeque<Instant>>>,
}

fn note(
    map: &Mutex<HashMap<std::net::IpAddr, VecDeque<Instant>>>,
    ip: std::net::IpAddr,
    window: Duration,
    limit: usize,
) -> bool {
    let now = Instant::now();
    let mut guard = map.lock();
    let entry = guard.entry(ip).or_default();
    while entry.front().is_some_and(|t| now.duration_since(*t) > window) {
        entry.pop_front();
    }
    if entry.len() >= limit {
        return false;
    }
    entry.push_back(now);
    true
}

impl PairRateLimiter {
    /// 페어링 시작을 허용할지. false면 429.
    pub fn allow_start(&self, ip: std::net::IpAddr) -> bool {
        note(&self.starts, ip, START_WINDOW, START_LIMIT)
    }

    /// 인증 실패 1건 기록. false면 그 IP는 당분간 차단(429).
    pub fn note_auth_failure(&self, ip: std::net::IpAddr) -> bool {
        note(&self.failures, ip, AUTH_FAIL_WINDOW, AUTH_FAIL_LIMIT)
    }

    /// 지금 이 IP가 인증 시도를 더 해도 되는지(기록 없이 조회만).
    pub fn auth_allowed(&self, ip: std::net::IpAddr) -> bool {
        let now = Instant::now();
        let guard = self.failures.lock();
        match guard.get(&ip) {
            Some(entry) => {
                entry
                    .iter()
                    .filter(|t| now.duration_since(**t) <= AUTH_FAIL_WINDOW)
                    .count()
                    < AUTH_FAIL_LIMIT
            }
            None => true,
        }
    }
}

/// 6자리 숫자 코드. uuid v4 바이트에서 뽑아 별도 난수 의존을 두지 않는다.
fn random_code() -> String {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let n = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
    format!("{n:06}")
}

/// 128비트 토큰(control의 `issue_token_at`과 같은 형식).
pub fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn new_peer_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn token_path(dir: &Path) -> PathBuf {
    dir.join(super::protocol::PEER_TOKENS_FILE)
}

pub fn hosts_path(dir: &Path) -> PathBuf {
    dir.join(super::protocol::PEER_HOSTS_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-peer-test-{tag}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn code_is_six_digits() {
        for _ in 0..50 {
            let c = random_code();
            assert_eq!(c.len(), 6);
            assert!(c.chars().all(|ch| ch.is_ascii_digit()));
        }
    }

    #[test]
    fn pairing_requires_both_code_and_human_approval() {
        let state = PairingState::default();
        let p = state.start("맥북", PeerClientKind::Peer).unwrap();
        // 승인 전에는 코드가 맞아도 대기.
        assert_eq!(
            state.complete(&p.pairing_id, &p.code),
            PairingOutcome::AwaitingApproval
        );
        assert!(state.approve(&p.pairing_id, PeerPermission::Input));
        assert_eq!(
            state.complete(&p.pairing_id, &p.code),
            PairingOutcome::Approved(PeerPermission::Input)
        );
        // 소비된 페어링은 재사용 불가.
        assert_eq!(
            state.complete(&p.pairing_id, &p.code),
            PairingOutcome::Expired
        );
    }

    #[test]
    fn wrong_code_three_times_kills_the_pairing() {
        let state = PairingState::default();
        let p = state.start("낯선 손님", PeerClientKind::Peer).unwrap();
        state.approve(&p.pairing_id, PeerPermission::Input);
        assert_eq!(
            state.complete(&p.pairing_id, "000000-nope"),
            PairingOutcome::WrongCode { remaining: 2 }
        );
        assert_eq!(
            state.complete(&p.pairing_id, "000000-nope"),
            PairingOutcome::WrongCode { remaining: 1 }
        );
        assert_eq!(
            state.complete(&p.pairing_id, "000000-nope"),
            PairingOutcome::Expired
        );
        // 이제는 올바른 코드도 통하지 않는다.
        assert_eq!(
            state.complete(&p.pairing_id, &p.code),
            PairingOutcome::Expired
        );
    }

    #[test]
    fn rejected_pairing_reports_rejection() {
        let state = PairingState::default();
        let p = state.start("손님", PeerClientKind::Web).unwrap();
        assert!(state.reject(&p.pairing_id));
        assert_eq!(
            state.complete(&p.pairing_id, &p.code),
            PairingOutcome::Rejected
        );
    }

    #[test]
    fn pending_pairings_are_capped() {
        let state = PairingState::default();
        for _ in 0..MAX_PENDING {
            assert!(state.start("손님", PeerClientKind::Web).is_some());
        }
        // 승인 다이얼로그를 폭주시키는 것 자체가 공격이다.
        assert!(state.start("손님", PeerClientKind::Web).is_none());
    }

    #[test]
    fn rate_limiter_caps_starts_per_ip() {
        let limiter = PairRateLimiter::default();
        let ip: std::net::IpAddr = "100.64.0.9".parse().unwrap();
        let other: std::net::IpAddr = "100.64.0.10".parse().unwrap();
        for _ in 0..START_LIMIT {
            assert!(limiter.allow_start(ip));
        }
        assert!(!limiter.allow_start(ip), "같은 IP는 창 안에서 막힌다");
        // IP별 독립이라 다른 기기는 영향받지 않는다.
        assert!(limiter.allow_start(other));
    }

    #[test]
    fn rate_limiter_blocks_after_repeated_auth_failures() {
        let limiter = PairRateLimiter::default();
        let ip: std::net::IpAddr = "100.64.0.11".parse().unwrap();
        assert!(limiter.auth_allowed(ip));
        for _ in 0..AUTH_FAIL_LIMIT {
            assert!(limiter.note_auth_failure(ip));
        }
        assert!(!limiter.note_auth_failure(ip));
        assert!(!limiter.auth_allowed(ip));
    }

    #[test]
    fn unknown_pairing_is_expired() {
        let state = PairingState::default();
        assert_eq!(state.complete("nope", "123456"), PairingOutcome::Expired);
    }

    #[test]
    fn token_store_roundtrip_and_authenticate() {
        let dir = scratch("tokens");
        let store = PeerTokenStore::new(token_path(&dir));
        assert!(store.load().is_empty());
        assert!(store.authenticate("anything").is_none());

        let token = new_token();
        store
            .insert(PeerRecord {
                peer_id: "p1".into(),
                name: "맥북".into(),
                token: token.clone(),
                permission: PeerPermission::Input,
                created_at: now_ms(),
                kind: PeerClientKind::Peer,
            })
            .unwrap();

        let found = store.authenticate(&token).expect("토큰 인증");
        assert_eq!(found.peer_id, "p1");
        assert_eq!(found.permission, PeerPermission::Input);
        assert!(store.authenticate("deadbeef").is_none());

        store
            .set_permission("p1", PeerPermission::ReadOnly)
            .unwrap();
        assert_eq!(
            store.authenticate(&token).unwrap().permission,
            PeerPermission::ReadOnly
        );

        store.remove("p1").unwrap();
        assert!(store.authenticate(&token).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn token_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perm");
        let store = PeerTokenStore::new(token_path(&dir));
        store
            .insert(PeerRecord {
                peer_id: "p1".into(),
                name: "n".into(),
                token: new_token(),
                permission: PeerPermission::ReadOnly,
                created_at: 0,
                kind: PeerClientKind::Web,
            })
            .unwrap();
        let mode = std::fs::metadata(token_path(&dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_store_roundtrip() {
        let dir = scratch("hosts");
        let store = PeerHostStore::new(hosts_path(&dir));
        store
            .insert(PeerHostRecord {
                peer_id: "h1".into(),
                label: "데스크탑".into(),
                address: "100.64.0.5:47800".into(),
                token: "t".into(),
                permission: PeerPermission::Input,
                auto_connect: true,
            })
            .unwrap();
        let hosts = store.load();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].address, "100.64.0.5:47800");
        // 같은 peerId 재삽입은 갱신(중복 없음).
        store
            .insert(PeerHostRecord {
                peer_id: "h1".into(),
                label: "데스크탑2".into(),
                address: "100.64.0.6:47800".into(),
                token: "t2".into(),
                permission: PeerPermission::ReadOnly,
                auto_connect: false,
            })
            .unwrap();
        let hosts = store.load();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].label, "데스크탑2");
        store.remove("h1").unwrap();
        assert!(store.load().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
