// src-tauri/src/webremote/chat.rs
//
// 웹 채팅 뷰의 서버 쪽(docs/web-remote-design.md §2·§5 M2).
//
// 캐릭터마다 **세션 로그와 독립된 두 번째 `TranscriptTailer`**를 2초 틱으로
// 돌려 새 항목을 접속한 브라우저에 push 한다. 읽기 전용 tail이라 기록
// 스레드와 간섭하지 않는다 — 같은 파일을 각자의 오프셋으로 읽을 뿐이다.
//
// 수명은 팔로워 수가 정한다: `chat.follow` 한 번에 그 WS 연결이 하나 붙고,
// 연결이 끊기면 빠진다. 아무도 안 보면 tail 스레드가 다음 슬라이스에 스스로
// 끝난다(폰을 닫아 둔 동안 파일을 계속 읽지 않는다).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::session_log::agent_transcript::{
    TranscriptItem, TranscriptSource, TranscriptTailer, BACKFILL_MAX_BYTES, BACKFILL_MAX_ITEMS,
};

use super::host::WebRemoteHub;
use super::protocol::HostMsg;

/// 전사 tail 주기(세션 로그 수집 스레드와 같은 값).
pub const CHAT_TICK: Duration = Duration::from_secs(2);
/// 정지·백필 요청에 반응하는 간격. 틱을 잘게 자 두면 채팅 화면 진입이
/// 최대 2초 걸리는 대신 이만큼만 걸린다.
const SLICE: Duration = Duration::from_millis(100);

/// 캐릭터 하나의 전사 소스 묶음을 만드는 공장. lib.rs가 실제 소스를 주입하고
/// 테스트는 가짜를 넣는다 — 이 모듈은 CLI 전사 위치를 몰라도 된다.
pub type SourceFactory =
    Arc<dyn Fn(&str, &str) -> Vec<Box<dyn TranscriptSource>> + Send + Sync>;

/// 명명 키 → 실제로 stdin에 쓸 바이트(**순수 함수**).
///
/// 웹은 이름만 보내고 제어 시퀀스는 서버가 정한다 — 브라우저가 임의 바이트를
/// 쏘는 통로를 만들지 않기 위해서다(임의 입력은 `chat.send`의 문장 주입으로
/// 충분하고, 그쪽은 개행이 제거된다).
pub fn key_bytes(name: &str) -> Option<&'static str> {
    Some(match name {
        "enter" => "\r",
        "esc" => "\x1b",
        "up" => "\x1b[A",
        "down" => "\x1b[B",
        "left" => "\x1b[D",
        "right" => "\x1b[C",
        "tab" => "\t",
        "backspace" => "\x7f",
        "space" => " ",
        "ctrl-c" => "\x03",
        "1" => "1",
        "2" => "2",
        "3" => "3",
        "4" => "4",
        "5" => "5",
        "6" => "6",
        "7" => "7",
        "8" => "8",
        "9" => "9",
        "y" => "y",
        "n" => "n",
        _ => return None,
    })
}

/// 이름 목록 전체를 바이트로 바꾼다. **하나라도 모르면 전부 거부**한다 —
/// 반쯤 쏘고 실패하면 TUI 상태가 어디로 갔는지 알 수 없다.
pub fn keys_to_bytes(names: &[String]) -> Result<Vec<&'static str>, String> {
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        match key_bytes(name) {
            Some(bytes) => out.push(bytes),
            None => return Err(name.clone()),
        }
    }
    Ok(out)
}

struct Entry {
    /// 이 캐릭터를 보고 있는 WS 연결들.
    followers: HashSet<u64>,
    stop: Arc<AtomicBool>,
    /// 뒤늦게 합류한 팔로워에게 최근 대화를 다시 보내라는 신호.
    resend: Arc<AtomicBool>,
}

/// 캐릭터별 채팅 tail 스레드의 소유자.
pub struct ChatRegistry {
    hub: Arc<WebRemoteHub>,
    factory: Mutex<Option<SourceFactory>>,
    entries: Mutex<HashMap<String, Entry>>,
}

impl ChatRegistry {
    pub fn new(hub: Arc<WebRemoteHub>) -> Arc<Self> {
        Arc::new(Self {
            hub,
            factory: Mutex::new(None),
            entries: Mutex::new(HashMap::new()),
        })
    }

    pub fn set_source_factory(&self, factory: SourceFactory) {
        *self.factory.lock().unwrap() = Some(factory);
    }

    /// 이 연결이 그 캐릭터의 채팅을 구독한다(멱등). 이미 도는 tailer가 있으면
    /// 최근 대화를 다시 보내게 하고, 없으면 새로 띄운다.
    pub fn follow(&self, agent_id: &str, cwd: &str, conn: u64) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get_mut(agent_id) {
            entry.followers.insert(conn);
            entry.resend.store(true, Ordering::Relaxed);
            return;
        }
        let factory = self.factory.lock().unwrap().clone();
        let sources = match &factory {
            Some(f) => f(agent_id, cwd),
            None => Vec::new(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let resend = Arc::new(AtomicBool::new(false));
        entries.insert(
            agent_id.to_string(),
            Entry {
                followers: HashSet::from([conn]),
                stop: stop.clone(),
                resend: resend.clone(),
            },
        );
        spawn_tail(
            self.hub.clone(),
            agent_id.to_string(),
            cwd.to_string(),
            sources,
            stop,
            resend,
        );
    }

    /// 이 연결이 그 캐릭터를 그만 본다. 마지막 팔로워였으면 tailer를 세운다.
    pub fn unfollow(&self, agent_id: &str, conn: u64) {
        let mut entries = self.entries.lock().unwrap();
        let Some(entry) = entries.get_mut(agent_id) else {
            return;
        };
        entry.followers.remove(&conn);
        if entry.followers.is_empty() {
            entry.stop.store(true, Ordering::Relaxed);
            entries.remove(agent_id);
        }
    }

    /// WS 연결이 끊겼다 — 그 연결이 보던 것 전부를 놓는다.
    pub fn release(&self, conn: u64) {
        let watched: Vec<String> = {
            let entries = self.entries.lock().unwrap();
            entries
                .iter()
                .filter(|(_, e)| e.followers.contains(&conn))
                .map(|(id, _)| id.clone())
                .collect()
        };
        for agent_id in watched {
            self.unfollow(&agent_id, conn);
        }
    }

    #[cfg(test)]
    pub fn is_following(&self, agent_id: &str) -> bool {
        self.entries.lock().unwrap().contains_key(agent_id)
    }
}

impl Drop for ChatRegistry {
    fn drop(&mut self) {
        for entry in self.entries.lock().unwrap().values() {
            entry.stop.store(true, Ordering::Relaxed);
        }
    }
}

fn unavailable_frame(agent_id: &str) -> HostMsg {
    HostMsg::Chat {
        agent_id: agent_id.to_string(),
        items: Vec::new(),
        backfill: false,
        unavailable: true,
    }
}

fn chat_frame(agent_id: &str, items: Vec<TranscriptItem>, backfill: bool) -> HostMsg {
    HostMsg::Chat {
        agent_id: agent_id.to_string(),
        items: items.into_iter().map(TranscriptItem::clamped).collect(),
        backfill,
        unavailable: false,
    }
}

fn spawn_tail(
    hub: Arc<WebRemoteHub>,
    agent_id: String,
    cwd: String,
    sources: Vec<Box<dyn TranscriptSource>>,
    stop: Arc<AtomicBool>,
    resend: Arc<AtomicBool>,
) {
    let _ = std::thread::Builder::new()
        .name(format!("web-chat-tail-{agent_id}"))
        .spawn(move || {
            // 소스가 하나도 없어도(지원 CLI 미설치) 스레드는 산다 — 늦게
            // 합류한 팔로워에게도 "전사 없음"을 알려야 하기 때문이다. 그
            // 경우 tick은 아무 일도 하지 않는다.
            let mut tailer = TranscriptTailer::new(&agent_id, &cwd, sources);
            // 지금 붙어 있는 전사 파일들. 바뀌면(리줌으로 새 세션 파일) 최근
            // 대화를 다시 실어 보낸다.
            let mut targets: Vec<PathBuf> = Vec::new();
            let mut announced_unavailable = false;
            let mut since_tick = CHAT_TICK; // 첫 바퀴는 즉시 돈다
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                // 뒤늦게 합류한 팔로워 — 틱을 기다리지 않고 바로 채워 준다.
                // (틱과 **독립**이다: 틱 경계에 걸린 팔로워가 굶지 않게.)
                if resend.swap(false, Ordering::Relaxed) {
                    if targets.is_empty() {
                        announced_unavailable = true;
                        hub.broadcast(unavailable_frame(&agent_id));
                    } else {
                        let back = tailer.backfill(BACKFILL_MAX_BYTES, BACKFILL_MAX_ITEMS);
                        hub.broadcast(chat_frame(&agent_id, back, true));
                    }
                }
                if since_tick >= CHAT_TICK {
                    since_tick = Duration::ZERO;
                    let items = tailer.tick_items();
                    let now = tailer.targets();
                    if now != targets {
                        // 새 전사 파일 — 최근 대화로 화면을 채운다(교체).
                        targets = now;
                        let back = tailer.backfill(BACKFILL_MAX_BYTES, BACKFILL_MAX_ITEMS);
                        hub.broadcast(chat_frame(&agent_id, back, true));
                    } else if !items.is_empty() {
                        hub.broadcast(chat_frame(&agent_id, items, false));
                    } else if targets.is_empty() && !announced_unavailable {
                        // 전사 파일을 못 찾았다(일반 셸·미지원 CLI).
                        announced_unavailable = true;
                        hub.broadcast(unavailable_frame(&agent_id));
                    }
                }
                std::thread::sleep(SLICE);
                since_tick += SLICE;
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_log::agent_transcript::{ItemRole, TranscriptItem};
    use std::io::Write;

    #[test]
    fn named_keys_map_to_control_bytes() {
        assert_eq!(key_bytes("enter"), Some("\r"));
        assert_eq!(key_bytes("esc"), Some("\x1b"));
        assert_eq!(key_bytes("up"), Some("\x1b[A"));
        assert_eq!(key_bytes("down"), Some("\x1b[B"));
        assert_eq!(key_bytes("tab"), Some("\t"));
        assert_eq!(key_bytes("backspace"), Some("\x7f"));
        assert_eq!(key_bytes("ctrl-c"), Some("\x03"));
        assert_eq!(key_bytes("1"), Some("1"));
        assert_eq!(key_bytes("y"), Some("y"));
        assert_eq!(key_bytes("n"), Some("n"));
        // 테이블 밖은 없는 키다 — 임의 바이트 통로를 만들지 않는다.
        for unknown in ["", "f1", "ctrl-d", "\x03", "0", "a", "ENTER"] {
            assert_eq!(key_bytes(unknown), None, "{unknown}는 열려 있으면 안 된다");
        }
    }

    #[test]
    fn one_unknown_key_rejects_the_whole_sequence() {
        let ok = keys_to_bytes(&["1".into(), "enter".into()]).unwrap();
        assert_eq!(ok, vec!["1", "\r"]);
        let err = keys_to_bytes(&["y".into(), "f13".into(), "enter".into()]).unwrap_err();
        assert_eq!(err, "f13");
    }

    // ── tail 스레드 ───────────────────────────────────────────────────

    struct LineSource {
        path: PathBuf,
    }

    impl TranscriptSource for LineSource {
        fn label(&self) -> &'static str {
            "test"
        }
        fn locate(&mut self, _agent_id: &str, _cwd: &str) -> Option<PathBuf> {
            self.path.exists().then(|| self.path.clone())
        }
        fn parse(&self, raw: &str) -> Vec<TranscriptItem> {
            vec![TranscriptItem::speech(ItemRole::User, raw)]
        }
    }

    fn scratch() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("webremote-chat-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn append(path: &PathBuf, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    /// broadcast에서 이 캐릭터의 chat 프레임만 골라 기다린다.
    fn wait_chat(
        rx: &mut tokio::sync::broadcast::Receiver<Arc<HostMsg>>,
        timeout: Duration,
    ) -> Option<HostMsg> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Ok(msg) = rx.try_recv() {
                if matches!(&*msg, HostMsg::Chat { .. }) {
                    return Some((*msg).clone());
                }
                continue;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn follow_backfills_then_streams_new_items() {
        let dir = scratch();
        let path = dir.join("t.jsonl");
        append(&path, "old-1\nold-2\n");

        let hub = WebRemoteHub::new();
        let mut rx = hub.subscribe();
        let chat = ChatRegistry::new(hub);
        let p = path.clone();
        chat.set_source_factory(Arc::new(move |_agent, _cwd| {
            vec![Box::new(LineSource { path: p.clone() })]
        }));

        chat.follow("a1", "/w", 1);
        // 백필 — 이미 파일에 있던 대화가 먼저 온다(교체 프레임).
        match wait_chat(&mut rx, Duration::from_secs(3)).expect("백필") {
            HostMsg::Chat {
                items,
                backfill,
                unavailable,
                ..
            } => {
                assert!(backfill, "첫 프레임은 교체용 백필이다");
                assert!(!unavailable);
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].text, "old-1");
                assert_eq!(items[1].text, "old-2");
            }
            other => panic!("chat 프레임이어야 한다: {other:?}"),
        }

        // 이후 새 줄은 증분으로 흐른다.
        append(&path, "new-1\n");
        match wait_chat(&mut rx, Duration::from_secs(5)).expect("증분") {
            HostMsg::Chat {
                items, backfill, ..
            } => {
                assert!(!backfill, "증분은 이어 붙인다");
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].text, "new-1");
            }
            other => panic!("chat 프레임이어야 한다: {other:?}"),
        }

        chat.unfollow("a1", 1);
        assert!(!chat.is_following("a1"), "마지막 팔로워가 빠지면 정리된다");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_transcript_reports_unavailable() {
        let dir = scratch();
        let hub = WebRemoteHub::new();
        let mut rx = hub.subscribe();
        let chat = ChatRegistry::new(hub);
        let missing = dir.join("nope.jsonl");
        chat.set_source_factory(Arc::new(move |_a, _c| {
            vec![Box::new(LineSource {
                path: missing.clone(),
            })]
        }));

        chat.follow("a1", "/w", 7);
        match wait_chat(&mut rx, Duration::from_secs(3)).expect("unavailable") {
            HostMsg::Chat {
                unavailable, items, ..
            } => {
                assert!(unavailable, "전사가 없으면 폴백 안내");
                assert!(items.is_empty());
            }
            other => panic!("chat 프레임이어야 한다: {other:?}"),
        }
        chat.release(7);
        assert!(!chat.is_following("a1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 소스가 아예 없는 기계(지원 CLI 미설치)도 조용히 죽지 않고 알린다.
    /// **뒤늦게 합류한 팔로워도** 같은 안내를 받아야 한다 — 그러지 않으면
    /// 두 번째로 채팅을 연 화면이 영원히 빈 채로 남는다.
    #[test]
    fn no_sources_at_all_is_unavailable_for_every_follower() {
        let hub = WebRemoteHub::new();
        let mut rx = hub.subscribe();
        let chat = ChatRegistry::new(hub);
        chat.set_source_factory(Arc::new(|_a, _c| Vec::new()));
        chat.follow("a1", "/w", 1);
        match wait_chat(&mut rx, Duration::from_secs(3)).expect("unavailable") {
            HostMsg::Chat { unavailable, .. } => assert!(unavailable),
            other => panic!("chat 프레임이어야 한다: {other:?}"),
        }
        // 두 번째 연결이 붙는다 — tailer는 이미 돌고 있다.
        chat.follow("a1", "/w", 2);
        match wait_chat(&mut rx, Duration::from_secs(3)).expect("늦은 팔로워 안내") {
            HostMsg::Chat { unavailable, .. } => assert!(unavailable),
            other => panic!("chat 프레임이어야 한다: {other:?}"),
        }
        chat.release(1);
        chat.release(2);
    }

    /// 늦게 합류한 팔로워는 틱을 기다리지 않고 곧바로 백필을 받는다.
    #[test]
    fn late_follower_gets_a_backfill_without_waiting_for_a_tick() {
        let dir = scratch();
        let path = dir.join("t.jsonl");
        append(&path, "이미-있던-말\n");

        let hub = WebRemoteHub::new();
        let mut rx = hub.subscribe();
        let chat = ChatRegistry::new(hub);
        let p = path.clone();
        chat.set_source_factory(Arc::new(move |_a, _c| {
            vec![Box::new(LineSource { path: p.clone() })]
        }));

        chat.follow("a1", "/w", 1);
        wait_chat(&mut rx, Duration::from_secs(3)).expect("첫 백필");

        chat.follow("a1", "/w", 2);
        // 2초 틱보다 빨리 와야 한다.
        match wait_chat(&mut rx, Duration::from_millis(1500)).expect("늦은 백필") {
            HostMsg::Chat {
                items, backfill, ..
            } => {
                assert!(backfill);
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].text, "이미-있던-말");
            }
            other => panic!("chat 프레임이어야 한다: {other:?}"),
        }
        chat.release(1);
        chat.release(2);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 두 연결이 같은 캐릭터를 볼 때 하나가 끊겨도 tailer는 남는다.
    #[test]
    fn tailer_lives_while_any_follower_remains() {
        let dir = scratch();
        let path = dir.join("t.jsonl");
        append(&path, "");
        let hub = WebRemoteHub::new();
        let chat = ChatRegistry::new(hub);
        let p = path.clone();
        chat.set_source_factory(Arc::new(move |_a, _c| {
            vec![Box::new(LineSource { path: p.clone() })]
        }));

        chat.follow("a1", "/w", 1);
        chat.follow("a1", "/w", 2);
        // 같은 연결이 두 번 불러도 팔로워는 하나다(멱등).
        chat.follow("a1", "/w", 2);
        chat.unfollow("a1", 1);
        assert!(chat.is_following("a1"), "2번 연결이 아직 본다");
        chat.release(2);
        assert!(!chat.is_following("a1"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
