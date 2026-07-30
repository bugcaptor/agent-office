// src-tauri/src/persistence/memo_store.rs
//
// 에이전트별 포스트잇 메모(#79) 영속화. `memos/<agentId>/<sheetId>.txt` —
// **장(sheet) 단위 파일**이다. 한 캐릭터는 "현재 장" 하나(= frontmatter에
// `archived` 키가 없는 유일한 파일)와, 넘긴 과거 장들(= `archived` 스탬프가
// 붙은 파일들)을 갖는다. 넘기기는 파일 이동이 아니라 헤더에 스탬프를 더하는
// 것이므로 sheetId(=파일명)가 영구히 고정된다.
//
// 왜 JSON이 아니라 Obsidian식 frontmatter + plain text인가: 이 파일은 사람이
// 에디터로 직접 열어 읽고 고칠 수 있어야 한다(사용자 메모다). 본문이 JSON
// 문자열로 이스케이프되면 그 성질을 잃는다. 파서는 손으로 짠 `key: value`
// 파싱이며 yaml 크레이트를 새로 들이지 않는다 — 우리가 쓰는 키는 3개뿐이다.
//
// 본문 안에 `---` 줄이 있어도 깨지지 않는 규칙: 헤더는 **파일 첫 줄이 정확히
// `---`일 때만** 시작하고 **두 번째 `---` 줄까지**다. 닫는 `---`를 못 찾으면
// 헤더가 아닌 것으로 보고 파일 전체를 본문으로 돌린다(헤더 없는 파일도 안전).
//
// 쓰기는 work_log_store.rs와 같은 tmp+rename 원자 교체다(torn write 방지).
// agentId/sheetId 경로 안전성 검증은 diary_store.rs 선례대로 복제한다(공유
// 헬퍼가 없는 코드베이스 관례).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use chrono::{DateTime, Local, SecondsFormat};

use crate::types::{MemoSheet, MemoSheetMeta};

#[derive(Debug)]
pub enum MemoStoreError {
    /// agentId/sheetId가 경로 요소로 안전하지 않음(구분자/`..`/빈 문자열).
    InvalidId,
    /// 지목한 장이 없음(read_sheet 전용).
    NotFound,
    /// 파일 시스템 오류.
    Io(String),
}

impl std::fmt::Display for MemoStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoStoreError::InvalidId => write!(f, "invalid id (unsafe path element)"),
            MemoStoreError::NotFound => write!(f, "memo sheet not found"),
            MemoStoreError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for MemoStoreError {}

/// sheetId 충돌 시 붙이는 접미사 탐색 상한. 같은 초에 이만큼 새 장을 만드는
/// 시나리오는 없으므로, 넘으면 uuid로 확실히 유일화한다(무한 루프 방지).
const MAX_SHEET_ID_SUFFIX: u32 = 1000;

/// `<dir>/<agentId>/<sheetId>.txt` 장들을 관리한다. `dir`은 주입(테스트는 tempdir).
pub struct MemoStore {
    dir: PathBuf,
}

/// frontmatter 파싱 결과. 알 수 없는 키는 버린다(전방 호환).
struct ParsedSheet {
    created: Option<String>,
    updated: Option<String>,
    archived: Option<String>,
    body: String,
}

/// 첫 줄과 그 뒤 나머지를 나눈다. 개행이 없으면 `(전체, "")`, 빈 문자열은 `None`.
fn split_first_line(s: &str) -> Option<(&str, &str)> {
    match s.find('\n') {
        Some(i) => Some((&s[..i], &s[i + 1..])),
        None if s.is_empty() => None,
        None => Some((s, "")),
    }
}

/// frontmatter 헤더 + 본문 파싱. 헤더가 없거나 닫히지 않으면 전체를 본문으로.
fn parse_sheet_text(text: &str) -> ParsedSheet {
    let whole = || ParsedSheet {
        created: None,
        updated: None,
        archived: None,
        body: text.to_string(),
    };

    // 여는 `---`: 파일 첫 줄이 정확히 `---`여야 한다(CR 허용).
    let after_open = match split_first_line(text) {
        Some((first, rest)) if first.trim_end_matches('\r') == "---" => rest,
        _ => return whole(),
    };

    let mut created = None;
    let mut updated = None;
    let mut archived = None;
    let mut cursor = after_open;
    loop {
        let Some((line, rest)) = split_first_line(cursor) else {
            // 닫는 `---`가 없다 → 헤더로 보지 않는다(본문 첫 줄이 `---`인 경우).
            return whole();
        };
        let line = line.trim_end_matches('\r');
        if line == "---" {
            return ParsedSheet {
                created,
                updated,
                archived,
                body: rest.to_string(),
            };
        }
        if let Some((key, value)) = line.split_once(':') {
            // 값에 `:`가 더 있어도(RFC3339 시각) 첫 `:`에서만 나눈다.
            let value = value.trim().to_string();
            match key.trim() {
                "created" => created = Some(value),
                "updated" => updated = Some(value),
                "archived" => archived = Some(value),
                _ => {}
            }
        }
        cursor = rest;
    }
}

/// frontmatter 헤더 + 본문 직렬화. `archived`는 있을 때만 쓴다.
fn format_sheet_text(
    created: &str,
    updated: &str,
    archived: Option<&str>,
    content: &str,
) -> String {
    let mut out = String::with_capacity(content.len() + 128);
    out.push_str("---\n");
    out.push_str("created: ");
    out.push_str(created);
    out.push('\n');
    out.push_str("updated: ");
    out.push_str(updated);
    out.push('\n');
    if let Some(a) = archived {
        out.push_str("archived: ");
        out.push_str(a);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(content);
    out
}

/// 로컬 타임존 오프셋을 담은 초 단위 RFC3339 문자열(예: `2026-07-30T12:34:56+09:00`).
/// 사람이 파일을 열어 읽는 헤더이므로 UTC가 아니라 로컬 시각을 쓴다.
fn rfc3339_local(dt: &DateTime<Local>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn now_rfc3339() -> String {
    rfc3339_local(&Local::now())
}

/// 생성 시각 기반 sheetId(사전순 = 시간순). 예: `20260730T123456`.
fn sheet_id_from(dt: &DateTime<Local>) -> String {
    dt.format("%Y%m%dT%H%M%S").to_string()
}

impl MemoStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// 경로 요소로 쓰기 전 안전성 검증(경로 조작 방지). 구분자/`..`/빈 문자열을
    /// 거부한다(diary_store.rs와 동일 규칙 — agentId와 sheetId 모두에 적용).
    fn validate_id(id: &str) -> Result<(), MemoStoreError> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            return Err(MemoStoreError::InvalidId);
        }
        Ok(())
    }

    fn agent_dir(&self, agent_id: &str) -> PathBuf {
        self.dir.join(agent_id)
    }

    fn sheet_path(&self, agent_id: &str, sheet_id: &str) -> PathBuf {
        self.agent_dir(agent_id).join(format!("{sheet_id}.txt"))
    }

    /// 한 캐릭터의 모든 장을 읽어 sheetId 오름차순(=시간순)으로 돌려준다.
    /// 디렉터리 부재 = 빈 Vec. `.txt`가 아닌 파일(tmp 포함)은 건너뛴다.
    fn load_sheets(&self, agent_id: &str) -> Vec<MemoSheet> {
        let mut out: Vec<MemoSheet> = Vec::new();
        let entries = match fs::read_dir(self.agent_dir(agent_id)) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `<sheetId>.txt`만. `.txt.tmp`는 확장자가 `tmp`라 자연히 걸러진다.
            if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            let sheet_id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let text = String::from_utf8_lossy(&bytes);
            let parsed = parse_sheet_text(&text);
            // 헤더가 없거나 깨진 파일도 버리지 않는다 — 시각은 빈 문자열로 두고
            // 본문만 살린다(사용자가 손으로 만든 파일 내성).
            out.push(MemoSheet {
                sheet_id,
                created: parsed.created.unwrap_or_default(),
                updated: parsed.updated.unwrap_or_default(),
                archived: parsed.archived,
                content: parsed.body,
            });
        }
        out.sort_by(|a, b| a.sheet_id.cmp(&b.sheet_id));
        out
    }

    /// 아직 쓰이지 않은 sheetId를 고른다. 기준은 생성 시각이고, 같은 초에 두 번
    /// 만들면 `-1`, `-2`… 접미사를 붙인다(사전순=시간순 성질 유지).
    fn allocate_sheet_id(&self, agent_id: &str, base: &str) -> String {
        if !self.sheet_path(agent_id, base).exists() {
            return base.to_string();
        }
        for n in 1..=MAX_SHEET_ID_SUFFIX {
            let candidate = format!("{base}-{n}");
            if !self.sheet_path(agent_id, &candidate).exists() {
                return candidate;
            }
        }
        format!("{base}-{}", uuid::Uuid::new_v4().simple())
    }

    /// 장 하나를 tmp+rename으로 원자 교체 저장한다.
    fn write_sheet(&self, agent_id: &str, sheet: &MemoSheet) -> Result<(), MemoStoreError> {
        let dir = self.agent_dir(agent_id);
        fs::create_dir_all(&dir).map_err(|e| MemoStoreError::Io(e.to_string()))?;
        let file = self.sheet_path(agent_id, &sheet.sheet_id);
        let text = format_sheet_text(
            &sheet.created,
            &sheet.updated,
            sheet.archived.as_deref(),
            &sheet.content,
        );
        // 같은 디렉터리 안 tmp에 쓰고 rename(원자적 교체). tmp 이름에 sheetId를
        // 붙여 동시 저장이 서로 밟지 않게 한다.
        let tmp = dir.join(format!("{}.txt.tmp", sheet.sheet_id));
        {
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
                .map_err(|e| MemoStoreError::Io(e.to_string()))?;
            f.write_all(text.as_bytes())
                .map_err(|e| MemoStoreError::Io(e.to_string()))?;
            f.sync_all()
                .map_err(|e| MemoStoreError::Io(e.to_string()))?;
        }
        // Windows는 dest 존재 시 rename이 실패하므로 먼저 지운다(있으면).
        #[cfg(windows)]
        {
            let _ = fs::remove_file(&file);
        }
        fs::rename(&tmp, &file).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            MemoStoreError::Io(e.to_string())
        })
    }

    /// 현재 장(= `archived` 없는 유일한 장)을 돌려준다. 없으면 새 빈 장을 만들어
    /// 반환한다(첫 열람에서 바로 쓸 수 있게).
    ///
    /// 방어적 복구: `archived` 없는 장이 둘 이상이면(외부 편집/사고) 가장 최근
    /// sheetId만 현재 장으로 남기고 나머지에 지금 시각의 `archived`를 스탬프한다
    /// — 삭제는 하지 않으므로 아카이브 목록에서 되찾을 수 있다.
    pub fn load_current(&self, agent_id: &str) -> Result<MemoSheet, MemoStoreError> {
        Self::validate_id(agent_id)?;
        let sheets = self.load_sheets(agent_id);
        let mut open: Vec<MemoSheet> = sheets.into_iter().filter(|s| s.archived.is_none()).collect();

        let current = match open.pop() {
            // sheetId 오름차순이므로 pop = 가장 최근 장.
            Some(c) => c,
            None => {
                let now = Local::now();
                let sheet = MemoSheet {
                    sheet_id: self.allocate_sheet_id(agent_id, &sheet_id_from(&now)),
                    created: rfc3339_local(&now),
                    updated: rfc3339_local(&now),
                    archived: None,
                    content: String::new(),
                };
                self.write_sheet(agent_id, &sheet)?;
                return Ok(sheet);
            }
        };

        // 남은 것들은 중복 현재 장 — 스탬프해 아카이브로 밀어 넣는다.
        if !open.is_empty() {
            let stamp = now_rfc3339();
            for mut stale in open {
                stale.archived = Some(stamp.clone());
                self.write_sheet(agent_id, &stale)?;
            }
        }
        Ok(current)
    }

    /// 지목한 장의 본문을 교체하고 `updated`를 갱신한다. `created`(그리고 이미
    /// 붙은 `archived`)는 보존한다 — 넘기기 직후 뒤늦게 도착한 저장이 아카이브
    /// 스탬프를 지워 "현재 장 둘"을 만드는 사고를 막는다. 파일이 없으면 그
    /// sheetId로 새로 만든다(렌더러가 쥔 id를 존중).
    pub fn save(
        &self,
        agent_id: &str,
        sheet_id: &str,
        content: &str,
    ) -> Result<(), MemoStoreError> {
        Self::validate_id(agent_id)?;
        Self::validate_id(sheet_id)?;
        let now = now_rfc3339();
        let existing = fs::read(self.sheet_path(agent_id, sheet_id))
            .ok()
            .map(|b| parse_sheet_text(&String::from_utf8_lossy(&b)));
        let (created, archived) = match existing {
            Some(p) => (p.created.unwrap_or_else(|| now.clone()), p.archived),
            None => (now.clone(), None),
        };
        self.write_sheet(
            agent_id,
            &MemoSheet {
                sheet_id: sheet_id.to_string(),
                created,
                updated: now,
                archived,
                content: content.to_string(),
            },
        )
    }

    /// 현재 장을 통째로 아카이브(= `archived` 스탬프 추가)하고, 즉시 새 빈 장을
    /// 만들어 돌려준다. 파일은 옮기지도 지우지도 않는다.
    pub fn archive_current(&self, agent_id: &str) -> Result<MemoSheet, MemoStoreError> {
        Self::validate_id(agent_id)?;
        let mut current = self.load_current(agent_id)?;
        let now = Local::now();
        current.archived = Some(rfc3339_local(&now));
        self.write_sheet(agent_id, &current)?;

        // 새 장의 id는 지금 시각 기준 — 방금 넘긴 장과 같은 초면 접미사가 붙는다.
        let fresh = MemoSheet {
            sheet_id: self.allocate_sheet_id(agent_id, &sheet_id_from(&now)),
            created: rfc3339_local(&now),
            updated: rfc3339_local(&now),
            archived: None,
            content: String::new(),
        };
        self.write_sheet(agent_id, &fresh)?;
        Ok(fresh)
    }

    /// 아카이브된 장들의 메타(본문 제외)를 최신순으로 돌려준다. 목록 UI 전용 —
    /// 장이 수백 개가 되어도 본문을 다 읽어 올리지 않는다는 뜻은 아니지만(파일은
    /// 어차피 읽는다) 렌더러로 넘기는 payload는 메타만이다.
    pub fn list_archive(&self, agent_id: &str) -> Result<Vec<MemoSheetMeta>, MemoStoreError> {
        Self::validate_id(agent_id)?;
        let mut items: Vec<MemoSheetMeta> = self
            .load_sheets(agent_id)
            .into_iter()
            .filter_map(|s| {
                s.archived.map(|archived| MemoSheetMeta {
                    sheet_id: s.sheet_id,
                    created: s.created,
                    updated: s.updated,
                    archived,
                })
            })
            .collect();
        // 최신순: 아카이브 시각 내림차순, 같으면 sheetId 내림차순(=생성 늦은 순).
        items.sort_by(|a, b| {
            b.archived
                .cmp(&a.archived)
                .then_with(|| b.sheet_id.cmp(&a.sheet_id))
        });
        Ok(items)
    }

    /// 특정 장 전체(본문 포함). 없으면 `NotFound`.
    pub fn read_sheet(
        &self,
        agent_id: &str,
        sheet_id: &str,
    ) -> Result<MemoSheet, MemoStoreError> {
        Self::validate_id(agent_id)?;
        Self::validate_id(sheet_id)?;
        let bytes =
            fs::read(self.sheet_path(agent_id, sheet_id)).map_err(|_| MemoStoreError::NotFound)?;
        let parsed = parse_sheet_text(&String::from_utf8_lossy(&bytes));
        Ok(MemoSheet {
            sheet_id: sheet_id.to_string(),
            created: parsed.created.unwrap_or_default(),
            updated: parsed.updated.unwrap_or_default(),
            archived: parsed.archived,
            content: parsed.body,
        })
    }

    /// 캐릭터 삭제 시 그 캐릭터의 메모 폴더를 통째로 정리한다. 부재는 무해 통과.
    pub fn delete_agent(&self, agent_id: &str) -> Result<(), MemoStoreError> {
        Self::validate_id(agent_id)?;
        match fs::remove_dir_all(self.agent_dir(agent_id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(MemoStoreError::Io(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-memo-store-test-{}", uuid::Uuid::new_v4()))
    }

    struct Scratch {
        dir: PathBuf,
        store: MemoStore,
    }

    impl Scratch {
        fn new() -> Self {
            let dir = scratch_dir();
            Self {
                store: MemoStore::new(dir.clone()),
                dir,
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    // ---- frontmatter 파싱/직렬화 ----

    #[test]
    fn frontmatter_roundtrips_header_and_body() {
        let text = format_sheet_text(
            "2026-07-30T12:34:56+09:00",
            "2026-07-30T13:00:00+09:00",
            None,
            "첫 줄\n둘째 줄\n",
        );
        let parsed = parse_sheet_text(&text);
        assert_eq!(parsed.created.as_deref(), Some("2026-07-30T12:34:56+09:00"));
        assert_eq!(parsed.updated.as_deref(), Some("2026-07-30T13:00:00+09:00"));
        assert_eq!(parsed.archived, None);
        assert_eq!(parsed.body, "첫 줄\n둘째 줄\n");
    }

    #[test]
    fn frontmatter_roundtrips_archived_stamp() {
        let text = format_sheet_text(
            "2026-07-30T12:34:56+09:00",
            "2026-07-30T13:00:00+09:00",
            Some("2026-07-31T09:00:00+09:00"),
            "본문",
        );
        let parsed = parse_sheet_text(&text);
        assert_eq!(parsed.archived.as_deref(), Some("2026-07-31T09:00:00+09:00"));
        assert_eq!(parsed.body, "본문");
    }

    #[test]
    fn body_containing_dash_lines_survives_roundtrip() {
        // 본문 안의 `---`는 헤더 구분자로 오인되면 안 된다(두 번째 `---`까지만 헤더).
        let body = "메모 시작\n---\n구분선 아래\n---\n끝\n";
        let text = format_sheet_text("c", "u", None, body);
        let parsed = parse_sheet_text(&text);
        assert_eq!(parsed.created.as_deref(), Some("c"));
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn text_without_header_is_all_body() {
        let parsed = parse_sheet_text("헤더 없는 순수 텍스트\n둘째 줄");
        assert_eq!(parsed.created, None);
        assert_eq!(parsed.updated, None);
        assert_eq!(parsed.body, "헤더 없는 순수 텍스트\n둘째 줄");
    }

    #[test]
    fn unclosed_header_is_treated_as_body() {
        // 첫 줄이 `---`지만 닫는 `---`가 없다 → 헤더로 보지 않는다.
        let text = "---\n어쩌다 이렇게 시작한 본문\n";
        let parsed = parse_sheet_text(text);
        assert_eq!(parsed.created, None);
        assert_eq!(parsed.body, text);
    }

    #[test]
    fn unknown_header_keys_are_ignored() {
        let text = "---\ncreated: c\nweird: 값\nupdated: u\n---\n본문";
        let parsed = parse_sheet_text(text);
        assert_eq!(parsed.created.as_deref(), Some("c"));
        assert_eq!(parsed.updated.as_deref(), Some("u"));
        assert_eq!(parsed.body, "본문");
    }

    #[test]
    fn crlf_header_is_parsed() {
        let text = "---\r\ncreated: c\r\nupdated: u\r\n---\r\n본문";
        let parsed = parse_sheet_text(text);
        assert_eq!(parsed.created.as_deref(), Some("c"));
        assert_eq!(parsed.body, "본문");
    }

    #[test]
    fn empty_body_roundtrips() {
        let parsed = parse_sheet_text(&format_sheet_text("c", "u", None, ""));
        assert_eq!(parsed.body, "");
    }

    #[test]
    fn sheet_id_is_lexicographically_time_ordered() {
        use chrono::TimeZone;
        let earlier = Local.with_ymd_and_hms(2026, 7, 30, 12, 34, 56).unwrap();
        let later = Local.with_ymd_and_hms(2026, 7, 30, 12, 34, 57).unwrap();
        assert_eq!(sheet_id_from(&earlier), "20260730T123456");
        assert!(sheet_id_from(&earlier) < sheet_id_from(&later));
    }

    // ---- 스토어 동작 ----

    #[test]
    fn load_current_creates_an_empty_sheet_when_none_exists() {
        let s = Scratch::new();
        let sheet = s.store.load_current("a1").unwrap();

        assert_eq!(sheet.content, "");
        assert!(sheet.archived.is_none());
        assert!(!sheet.sheet_id.is_empty());
        assert!(!sheet.created.is_empty());
        // 실제 파일로 남는다(다음 로드에서 같은 장을 되찾도록).
        assert!(s.dir.join("a1").join(format!("{}.txt", sheet.sheet_id)).exists());
    }

    #[test]
    fn load_current_returns_the_same_sheet_on_second_call() {
        let s = Scratch::new();
        let first = s.store.load_current("a1").unwrap();
        let second = s.store.load_current("a1").unwrap();
        assert_eq!(first.sheet_id, second.sheet_id);
    }

    #[test]
    fn save_then_load_current_roundtrips_content() {
        let s = Scratch::new();
        let sheet = s.store.load_current("a1").unwrap();

        s.store.save("a1", &sheet.sheet_id, "오늘 할 일\n- 이슈 #79").unwrap();
        let loaded = s.store.load_current("a1").unwrap();

        assert_eq!(loaded.sheet_id, sheet.sheet_id);
        assert_eq!(loaded.content, "오늘 할 일\n- 이슈 #79");
        assert_eq!(loaded.created, sheet.created); // created 보존
    }

    #[test]
    fn save_on_missing_sheet_creates_it_with_that_id() {
        let s = Scratch::new();
        s.store.save("a1", "20260101T000000", "직접 지목한 장").unwrap();
        let loaded = s.store.read_sheet("a1", "20260101T000000").unwrap();
        assert_eq!(loaded.content, "직접 지목한 장");
        assert!(loaded.archived.is_none());
    }

    #[test]
    fn save_preserves_an_existing_archived_stamp() {
        // 넘기기 직후 뒤늦게 도착한 디바운스 저장이 아카이브를 되살리면
        // "현재 장 둘"이 된다 — 스탬프는 절대 지우지 않는다.
        let s = Scratch::new();
        let old = s.store.load_current("a1").unwrap();
        s.store.archive_current("a1").unwrap();

        s.store.save("a1", &old.sheet_id, "늦게 도착한 타이핑").unwrap();

        let reread = s.store.read_sheet("a1", &old.sheet_id).unwrap();
        assert_eq!(reread.content, "늦게 도착한 타이핑");
        assert!(reread.archived.is_some());
        // 현재 장은 여전히 아카이브 이후 새로 만든 빈 장 하나뿐.
        let current = s.store.load_current("a1").unwrap();
        assert_ne!(current.sheet_id, old.sheet_id);
        assert_eq!(current.content, "");
    }

    #[test]
    fn archive_current_stamps_old_sheet_and_returns_a_fresh_one() {
        let s = Scratch::new();
        let old = s.store.load_current("a1").unwrap();
        s.store.save("a1", &old.sheet_id, "넘길 내용").unwrap();

        let fresh = s.store.archive_current("a1").unwrap();

        assert_ne!(fresh.sheet_id, old.sheet_id);
        assert_eq!(fresh.content, "");
        assert!(fresh.archived.is_none());
        // 파일은 이동/삭제되지 않고 스탬프만 붙는다.
        let stamped = s.store.read_sheet("a1", &old.sheet_id).unwrap();
        assert_eq!(stamped.content, "넘길 내용");
        assert!(stamped.archived.is_some());
    }

    #[test]
    fn archived_sheets_appear_in_the_archive_list_newest_first() {
        let s = Scratch::new();
        // 같은 초에 두 번 넘겨도 sheetId 접미사로 갈라져 둘 다 남는다.
        let first = s.store.load_current("a1").unwrap();
        s.store.save("a1", &first.sheet_id, "1장").unwrap();
        let second = s.store.archive_current("a1").unwrap();
        s.store.save("a1", &second.sheet_id, "2장").unwrap();
        s.store.archive_current("a1").unwrap();

        let archive = s.store.list_archive("a1").unwrap();

        assert_eq!(archive.len(), 2);
        // 최신순 — 나중에 넘긴 2장이 앞.
        assert_eq!(archive[0].sheet_id, second.sheet_id);
        assert_eq!(archive[1].sheet_id, first.sheet_id);
        assert!(archive.iter().all(|m| !m.archived.is_empty()));
    }

    #[test]
    fn list_archive_excludes_the_current_sheet() {
        let s = Scratch::new();
        s.store.load_current("a1").unwrap();
        assert!(s.store.list_archive("a1").unwrap().is_empty());
    }

    #[test]
    fn read_sheet_returns_not_found_for_an_unknown_sheet() {
        let s = Scratch::new();
        assert!(matches!(
            s.store.read_sheet("a1", "20260101T000000"),
            Err(MemoStoreError::NotFound)
        ));
    }

    #[test]
    fn sheets_are_isolated_per_agent() {
        let s = Scratch::new();
        let a = s.store.load_current("a1").unwrap();
        let b = s.store.load_current("a2").unwrap();
        s.store.save("a1", &a.sheet_id, "A의 메모").unwrap();
        s.store.save("a2", &b.sheet_id, "B의 메모").unwrap();

        assert_eq!(s.store.load_current("a1").unwrap().content, "A의 메모");
        assert_eq!(s.store.load_current("a2").unwrap().content, "B의 메모");
    }

    #[test]
    fn duplicate_open_sheets_are_repaired_keeping_the_newest() {
        let s = Scratch::new();
        // 외부 편집으로 archived 없는 장이 둘 생긴 상황을 만든다.
        let dir = s.dir.join("a1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("20260101T000000.txt"),
            format_sheet_text("c1", "u1", None, "오래된 장"),
        )
        .unwrap();
        fs::write(
            dir.join("20260102T000000.txt"),
            format_sheet_text("c2", "u2", None, "새 장"),
        )
        .unwrap();

        let current = s.store.load_current("a1").unwrap();

        assert_eq!(current.sheet_id, "20260102T000000");
        assert_eq!(current.content, "새 장");
        // 오래된 장은 삭제가 아니라 아카이브로 밀려난다.
        let archive = s.store.list_archive("a1").unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(archive[0].sheet_id, "20260101T000000");
        assert_eq!(
            s.store.read_sheet("a1", "20260101T000000").unwrap().content,
            "오래된 장"
        );
    }

    #[test]
    fn write_is_atomic_and_leaves_no_tmp_file() {
        let s = Scratch::new();
        let sheet = s.store.load_current("a1").unwrap();
        s.store.save("a1", &sheet.sheet_id, "본문").unwrap();

        let leftovers: Vec<_> = fs::read_dir(s.dir.join("a1"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "tmp 파일이 남았다: {leftovers:?}");
    }

    #[test]
    fn load_sheets_ignores_tmp_and_non_txt_files() {
        let s = Scratch::new();
        let dir = s.dir.join("a1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("20260101T000000.txt.tmp"), "---\ncreated: c\nupdated: u\n---\ntmp").unwrap();
        fs::write(dir.join("README.md"), "무관한 파일").unwrap();

        // 유효한 장이 없으므로 새 빈 장이 만들어진다.
        let current = s.store.load_current("a1").unwrap();
        assert_eq!(current.content, "");
        assert_ne!(current.sheet_id, "20260101T000000");
    }

    #[test]
    fn delete_agent_removes_the_whole_folder() {
        let s = Scratch::new();
        s.store.load_current("a1").unwrap();
        assert!(s.dir.join("a1").exists());

        s.store.delete_agent("a1").unwrap();

        assert!(!s.dir.join("a1").exists());
        assert!(s.store.list_archive("a1").unwrap().is_empty());
    }

    #[test]
    fn delete_agent_on_missing_folder_is_ok() {
        let s = Scratch::new();
        s.store.delete_agent("nobody").expect("no-op on missing dir");
    }

    #[test]
    fn rejects_unsafe_agent_id() {
        let s = Scratch::new();
        assert!(matches!(
            s.store.load_current("../evil"),
            Err(MemoStoreError::InvalidId)
        ));
        assert!(matches!(
            s.store.load_current(""),
            Err(MemoStoreError::InvalidId)
        ));
        assert!(matches!(
            s.store.save("a/b", "s", "x"),
            Err(MemoStoreError::InvalidId)
        ));
        assert!(matches!(
            s.store.save("a\\b", "s", "x"),
            Err(MemoStoreError::InvalidId)
        ));
        assert!(matches!(
            s.store.archive_current(".."),
            Err(MemoStoreError::InvalidId)
        ));
        assert!(matches!(
            s.store.list_archive(""),
            Err(MemoStoreError::InvalidId)
        ));
        assert!(matches!(
            s.store.delete_agent("../evil"),
            Err(MemoStoreError::InvalidId)
        ));
    }

    #[test]
    fn rejects_unsafe_sheet_id() {
        let s = Scratch::new();
        assert!(matches!(
            s.store.save("a1", "../evil", "x"),
            Err(MemoStoreError::InvalidId)
        ));
        assert!(matches!(
            s.store.read_sheet("a1", ""),
            Err(MemoStoreError::InvalidId)
        ));
        assert!(matches!(
            s.store.read_sheet("a1", "sub/dir"),
            Err(MemoStoreError::InvalidId)
        ));
    }
}
