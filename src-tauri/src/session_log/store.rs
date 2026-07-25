// src-tauri/src/session_log/store.rs
//
// 세션 로그 파일 저장소. docs/session-log-design.md §4 가 정본.
//
//   <app_data>/session-logs/v1/<agentId>/<YYYYMMDD-HHMMSS>-<sid8>.log
//   <app_data>/session-logs/v1/study/<agentId>-<YYYYMMDD-HHMMSS>.md
//
// 다른 저장소들과 달리 temp+rename 원자 쓰기를 하지 않는다 -- 스트리밍이라
// 매번 전체를 다시 쓸 수 없고, O_APPEND는 부분 쓰기가 나도 뒷부분만 잘릴 뿐
// 앞부분은 성하다. 로그에는 그 트레이드가 맞다.
//
// 전사 필터(transcript.rs)가 확정한 줄을 받아 (1) 연속 중복 접기,
// (2) 시각 마커 삽입, (3) append 를 한다.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::types::SessionLogItem;

/// 시각 마커를 다시 찍는 최소 간격. 이보다 오래 조용했다가 출력이 재개되면
/// `--- HH:MM:SS ---` 한 줄을 끼워 넣는다.
const MARKER_GAP: Duration = Duration::from_secs(60);
/// 헤더 파싱에 읽는 앞부분 크기. 목록 조회가 파일 전체를 읽지 않게 한다.
const HEADER_PROBE_BYTES: usize = 512;

pub const HEADER_MAGIC: &str = "# agent-office session log v1";

/// `<app_data>/session-logs/v1`. 저장소 루트.
pub fn root_for(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("session-logs").join("v1")
}

/// 학습자료(.md)를 모으는 하위 디렉터리. `study`는 agentId로 쓸 수 없는
/// 이름이 아니므로(경로 요소 규칙상 가능) 충돌을 피하려고 파일 확장자와
/// 목록 필터(.log만)로 갈라 둔다.
pub fn study_dir(root: &Path) -> PathBuf {
    root.join("study")
}

/// `agent_id`를 경로 요소로 쓰기 전 안전성 검증(diary_store.rs와 동일 규칙).
pub fn valid_agent_id(agent_id: &str) -> bool {
    !(agent_id.is_empty()
        || agent_id.contains('/')
        || agent_id.contains('\\')
        || agent_id.contains("..")
        || agent_id == "study")
}

/// 한 세션의 로그 파일에 이어 쓰는 writer. 세션 하나당 하나.
pub struct SessionLogWriter {
    path: PathBuf,
    file: File,
    /// 직전에 쓴 줄과 그 반복 횟수(연속 중복 접기).
    last_line: Option<String>,
    repeat: usize,
    /// 마지막으로 실제 쓰기가 일어난 시각(시각 마커 판단용).
    last_write_at: SystemTime,
}

impl SessionLogWriter {
    /// 세션 로그 파일을 연다(없으면 헤더와 함께 생성). 같은 sessionId의 파일이
    /// 이미 있으면 **이어 붙인다** -- 앱 재시작 후 입양된 세션이 한 파일로
    /// 이어지게 하는 경로다.
    ///
    /// 실패(경로 불가·권한 등)는 `None`. 로그는 부가 기능이므로 어떤 실패도
    /// 세션 동작을 막지 않는다.
    pub fn open(
        root: &Path,
        agent_id: &str,
        session_id: &str,
        cwd: &str,
        now: SystemTime,
    ) -> Option<Self> {
        if !valid_agent_id(agent_id) {
            return None;
        }
        let dir = root.join(agent_id);
        fs::create_dir_all(&dir).ok()?;

        let sid8 = short_session_id(session_id);
        let existing = find_existing(&dir, &sid8);
        let is_new = existing.is_none();
        let path = existing.unwrap_or_else(|| {
            dir.join(format!("{}-{}.log", format_stamp(now), sid8))
        });

        let mut file = OpenOptions::new().create(true).append(true).open(&path).ok()?;
        if is_new {
            let header = format!(
                "{HEADER_MAGIC}\n# agentId: {agent_id}\n# sessionId: {session_id}\n# cwd: {cwd}\n# started: {}\n\n",
                format_rfc3339(now)
            );
            let _ = file.write_all(header.as_bytes());
        } else {
            // 이어받은 세션 -- 경계를 눈에 보이게 남긴다.
            let _ = file.write_all(
                format!("\n--- 이어짐 {} ---\n", format_clock(now)).as_bytes(),
            );
        }
        Some(Self {
            path,
            file,
            last_line: None,
            repeat: 0,
            last_write_at: now,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 확정된 줄들을 쓴다. 연속 중복은 접고, 오래 조용했으면 시각 마커를 끼운다.
    pub fn write_lines(&mut self, lines: &[String], now: SystemTime) {
        if lines.is_empty() {
            return;
        }
        let mut buf = String::new();

        // 마지막 쓰기로부터 충분히 지났으면 시각 마커를 먼저.
        if now
            .duration_since(self.last_write_at)
            .map(|d| d >= MARKER_GAP)
            .unwrap_or(false)
        {
            self.flush_repeat(&mut buf);
            buf.push_str(&format!("\n--- {} ---\n", format_clock(now)));
        }

        for line in lines {
            // 빈 줄은 접기 대상에서 뺀다(문단 구분이 사라지면 오히려 읽기 나쁘다).
            if !line.is_empty() && self.last_line.as_deref() == Some(line.as_str()) {
                self.repeat += 1;
                continue;
            }
            self.flush_repeat(&mut buf);
            buf.push_str(line);
            buf.push('\n');
            self.last_line = Some(line.clone());
        }

        if !buf.is_empty() {
            let _ = self.file.write_all(buf.as_bytes());
            self.last_write_at = now;
        }
    }

    /// 세션 종료. 남은 반복 표시를 털고 마지막 줄을 남긴다.
    pub fn finish(&mut self, now: SystemTime) {
        let mut buf = String::new();
        self.flush_repeat(&mut buf);
        buf.push_str(&format!("\n--- 세션 종료 {} ---\n", format_clock(now)));
        let _ = self.file.write_all(buf.as_bytes());
        let _ = self.file.flush();
    }

    fn flush_repeat(&mut self, buf: &mut String) {
        if self.repeat > 0 {
            buf.push_str(&format!("… (같은 줄 {}회 반복)\n", self.repeat + 1));
            self.repeat = 0;
            self.last_line = None;
        }
    }
}

/// 같은 sessionId로 이미 만들어진 로그 파일을 찾는다(입양 시 이어쓰기용).
fn find_existing(dir: &Path, sid8: &str) -> Option<PathBuf> {
    let suffix = format!("-{sid8}.log");
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(&suffix))
        })
}

/// sessionId(UUID)의 앞 8자. 파일명이 길어지지 않게 줄이되 한 캐릭터
/// 디렉터리 안에서 충돌하지 않을 만큼은 남긴다.
pub fn short_session_id(session_id: &str) -> String {
    let s: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if s.is_empty() {
        "nosessid".to_string()
    } else {
        s
    }
}

fn to_local(now: SystemTime) -> chrono::DateTime<chrono::Local> {
    chrono::DateTime::<chrono::Local>::from(now)
}

fn format_stamp(now: SystemTime) -> String {
    to_local(now).format("%Y%m%d-%H%M%S").to_string()
}

fn format_clock(now: SystemTime) -> String {
    to_local(now).format("%H:%M:%S").to_string()
}

fn format_rfc3339(now: SystemTime) -> String {
    to_local(now).to_rfc3339()
}

fn epoch_ms(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 한 캐릭터의 로그 파일 목록(최신순). `offset`/`limit`로 페이징한다.
/// 어떤 실패도 빈 목록으로 흡수한다 -- 열람 경로가 에러로 막히지 않게.
pub fn list_logs(root: &Path, agent_id: &str, offset: usize, limit: usize) -> (usize, Vec<SessionLogItem>) {
    if !valid_agent_id(agent_id) {
        return (0, Vec::new());
    }
    let dir = root.join(agent_id);
    let Ok(entries) = fs::read_dir(&dir) else {
        return (0, Vec::new());
    };

    let mut files: Vec<(PathBuf, fs::Metadata)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("log") {
                return None;
            }
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((path, meta))
        })
        .collect();

    // 최신순: 파일명이 시작 시각으로 시작하므로 이름 역순이 곧 최신순이다.
    files.sort_by(|a, b| b.0.file_name().cmp(&a.0.file_name()));
    let total = files.len();

    let items = files
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(path, meta)| {
            let header = read_header(&path);
            SessionLogItem {
                session_id: header.session_id.unwrap_or_default(),
                started_at: header.started_at.unwrap_or_else(|| {
                    meta.created()
                        .or_else(|_| meta.modified())
                        .map(epoch_ms)
                        .unwrap_or(0)
                }),
                modified_at: meta.modified().map(epoch_ms).unwrap_or(0),
                bytes: meta.len(),
                cwd: header.cwd.unwrap_or_default(),
                path: path.to_string_lossy().into_owned(),
            }
        })
        .collect();

    (total, items)
}

#[derive(Default)]
struct Header {
    session_id: Option<String>,
    cwd: Option<String>,
    started_at: Option<u64>,
}

/// 파일 앞부분만 읽어 헤더를 파싱한다. 헤더가 없거나 깨졌으면 빈 값.
fn read_header(path: &Path) -> Header {
    let mut header = Header::default();
    let Ok(mut file) = File::open(path) else {
        return header;
    };
    let mut buf = vec![0u8; HEADER_PROBE_BYTES];
    let Ok(n) = file.read(&mut buf) else {
        return header;
    };
    buf.truncate(n);
    let text = String::from_utf8_lossy(&buf);
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("# ") else {
            if line.starts_with('#') {
                continue;
            }
            break; // 헤더 블록 끝
        };
        if let Some(v) = rest.strip_prefix("sessionId: ") {
            header.session_id = Some(v.trim().to_string());
        } else if let Some(v) = rest.strip_prefix("cwd: ") {
            header.cwd = Some(v.trim().to_string());
        } else if let Some(v) = rest.strip_prefix("started: ") {
            header.started_at = chrono::DateTime::parse_from_rfc3339(v.trim())
                .ok()
                .map(|t| t.timestamp_millis().max(0) as u64);
        }
    }
    header
}

/// 주어진 경로가 세션 로그 루트 아래의 `.log`인지 검증한다(경로 탈출 차단).
/// 심볼릭 링크 우회를 막으려고 canonicalize 후 비교한다.
pub fn is_inside_root(root: &Path, path: &Path) -> bool {
    let (Ok(root), Ok(path)) = (root.canonicalize(), path.canonicalize()) else {
        return false;
    };
    path.starts_with(&root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agent-office-session-log-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_780_000_000 + secs)
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn writes_header_once_and_appends_lines() {
        let root = scratch();
        let mut w =
            SessionLogWriter::open(&root, "term-1", "abcd1234-ef", "/tmp/foo", t(0)).unwrap();
        w.write_lines(&["hello".into(), "world".into()], t(1));
        let body = read(w.path());
        assert!(body.starts_with(HEADER_MAGIC), "{body}");
        assert!(body.contains("# sessionId: abcd1234-ef"), "{body}");
        assert!(body.contains("# cwd: /tmp/foo"), "{body}");
        assert!(body.ends_with("hello\nworld\n"), "{body}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn same_session_reopens_the_same_file() {
        let root = scratch();
        let first = {
            let mut w =
                SessionLogWriter::open(&root, "term-1", "abcd1234-ef", "/tmp", t(0)).unwrap();
            w.write_lines(&["one".into()], t(1));
            w.path().to_path_buf()
        };
        let second = {
            let mut w =
                SessionLogWriter::open(&root, "term-1", "abcd1234-ef", "/tmp", t(500)).unwrap();
            w.write_lines(&["two".into()], t(501));
            w.path().to_path_buf()
        };
        assert_eq!(first, second, "같은 세션은 한 파일에 이어 써야 한다");
        let body = read(&first);
        assert!(body.contains("one"), "{body}");
        assert!(body.contains("two"), "{body}");
        assert_eq!(body.matches(HEADER_MAGIC).count(), 1, "{body}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn consecutive_duplicates_are_folded() {
        let root = scratch();
        let mut w = SessionLogWriter::open(&root, "a", "s1", "/tmp", t(0)).unwrap();
        let spin: Vec<String> = std::iter::repeat("Thinking…".to_string()).take(5).collect();
        w.write_lines(&spin, t(1));
        w.write_lines(&["done".into()], t(2));
        let body = read(w.path());
        assert!(body.contains("… (같은 줄 5회 반복)"), "{body}");
        assert_eq!(body.matches("Thinking…").count(), 1, "{body}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn blank_lines_are_not_folded() {
        let root = scratch();
        let mut w = SessionLogWriter::open(&root, "a", "s1", "/tmp", t(0)).unwrap();
        w.write_lines(&["".into(), "".into(), "".into()], t(1));
        let body = read(w.path());
        assert!(!body.contains("같은 줄"), "{body}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn long_silence_inserts_a_clock_marker() {
        let root = scratch();
        let mut w = SessionLogWriter::open(&root, "a", "s1", "/tmp", t(0)).unwrap();
        w.write_lines(&["before".into()], t(1));
        w.write_lines(&["after".into()], t(1 + 120));
        let body = read(w.path());
        let marker_lines: Vec<&str> = body.lines().filter(|l| l.starts_with("--- ")).collect();
        assert_eq!(marker_lines.len(), 1, "{body}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unsafe_agent_id_is_rejected() {
        let root = scratch();
        assert!(SessionLogWriter::open(&root, "../evil", "s", "/tmp", t(0)).is_none());
        assert!(SessionLogWriter::open(&root, "", "s", "/tmp", t(0)).is_none());
        assert!(SessionLogWriter::open(&root, "a/b", "s", "/tmp", t(0)).is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_returns_newest_first_with_paging() {
        let root = scratch();
        for i in 0..12 {
            let mut w = SessionLogWriter::open(
                &root,
                "term-1",
                &format!("sess{i:04}xx"),
                "/tmp/w",
                t(i * 3600),
            )
            .unwrap();
            w.write_lines(&[format!("line {i}")], t(i * 3600 + 1));
        }
        let (total, page1) = list_logs(&root, "term-1", 0, 10);
        assert_eq!(total, 12);
        assert_eq!(page1.len(), 10);
        // 최신순 -- 가장 마지막에 만든 세션이 먼저.
        assert_eq!(page1[0].session_id, "sess0011xx");
        assert_eq!(page1[0].cwd, "/tmp/w");
        assert!(page1[0].bytes > 0);
        assert!(page1[0].started_at > 0);

        let (_, page2) = list_logs(&root, "term-1", 10, 10);
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[1].session_id, "sess0000xx");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_of_unknown_agent_is_empty_not_error() {
        let root = scratch();
        let (total, items) = list_logs(&root, "nobody", 0, 10);
        assert_eq!(total, 0);
        assert!(items.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn path_outside_root_is_rejected() {
        let root = scratch();
        let mut w = SessionLogWriter::open(&root, "a", "s1", "/tmp", t(0)).unwrap();
        w.write_lines(&["x".into()], t(1));
        assert!(is_inside_root(&root, w.path()));
        assert!(!is_inside_root(&root, Path::new("/etc/hosts")));
        fs::remove_dir_all(&root).ok();
    }
}
