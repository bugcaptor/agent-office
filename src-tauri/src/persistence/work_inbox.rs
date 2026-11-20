//! 파일 수신함. scan은 원본을 보존하고 업무 스냅샷 저장 후 ack한다.
//! 형식 오류는 scan이, 의미 오류(id 충돌·없는 완료 대상)는 렌더러가 판정해 `rejected/`로
//! 옮기고, 고쳐 받은 업무(버린 선행·모르는 담당)는 `warnings.log`에 남긴다.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::Path, time::{Duration, SystemTime}};

pub const MAX_BYTES: usize = 64 * 1024;
const MAX_BATCH: usize = 50;
pub const WARNINGS_LOG: &str = "warnings.log";
/// 이보다 커지면 `warnings.log.1` 하나만 남기고 새로 쓴다.
const MAX_WARNINGS_BYTES: u64 = 256 * 1024;

/// 누락은 기존 값 유지, null은 플랜 값 상속, 숫자는 해당 단계의 설정이다.
#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLimitsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_follow_ups: Option<Option<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_wait_minutes: Option<Option<u32>>,
}
impl RunLimitsPatch {
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let (field, min, max) = match key {
            "max-follow-ups" => (&mut self.max_follow_ups, 0, 99),
            "permission-wait-minutes" => (&mut self.permission_wait_minutes, 1, 1440),
            _ => return Err(format!("unknown auto-run setting: {key}")),
        };
        let value = value.trim();
        *field = Some(if value == "inherit" {
            None
        } else {
            Some(value.parse::<u32>().ok().filter(|n| (min..=max).contains(n))
                .ok_or_else(|| format!("{key} requires an integer from {min} to {max}, or inherit"))?)
        });
        Ok(())
    }
}

/// 반복 예약 간격의 하한·상한(분). 렌더러 `workSchedule.ts`와 같다.
pub const SCHEDULE_MIN_MINUTES: u32 = 5;
pub const SCHEDULE_MAX_MINUTES: u32 = 30 * 24 * 60;
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn schedule_time(value: &str) -> Result<String, String> {
    let (h, m) = value.split_once(':').ok_or_else(|| format!("invalid time {value:?}; use HH:MM"))?;
    let digits = |part: &str, len: std::ops::RangeInclusive<usize>| len.contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit());
    let h: u32 = Some(h).filter(|h| digits(h, 1..=2)).and_then(|h| h.parse().ok()).filter(|h| *h < 24).ok_or_else(|| format!("invalid hour in {value:?}; use HH:MM"))?;
    let m: u32 = Some(m).filter(|m| digits(m, 2..=2)).and_then(|m| m.parse().ok()).filter(|m| *m < 60).ok_or_else(|| format!("invalid minute in {value:?}; use HH:MM"))?;
    Ok(format!("{h:02}:{m:02}"))
}
/// 요일 목록(`mon,wed`, `weekdays`, `weekends`, `daily`). 일요일부터 차례로, 중복 없이.
fn schedule_days(value: &str) -> Result<Vec<usize>, String> {
    let mut days = [false; 7];
    for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "weekdays" => (1..=5).for_each(|d| days[d] = true),
            "weekends" => { days[0] = true; days[6] = true; }
            "daily" => days = [true; 7],
            name => days[WEEKDAYS.iter().position(|d| d.eq_ignore_ascii_case(name) || name.starts_with(d) && name.len() <= 9)
                .ok_or_else(|| format!("unknown weekday {part:?}; use mon,tue,wed,thu,fri,sat,sun, weekdays or weekends"))?] = true,
        }
    }
    let days: Vec<usize> = (0..7).filter(|d| days[*d]).collect();
    if days.is_empty() { return Err("pick at least one weekday".into()); }
    Ok(days)
}
fn schedule_interval(value: &str) -> Result<u32, String> {
    let value = value.trim();
    let (number, unit) = value.split_at(value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len()));
    let n: u32 = number.parse().map_err(|_| format!("invalid interval {value:?}; use e.g. 30m, 2h or 1d"))?;
    let minutes = match unit { "m" | "min" => Some(n), "h" => n.checked_mul(60), "d" => n.checked_mul(24 * 60), _ => None }
        .ok_or_else(|| format!("invalid interval {value:?}; use e.g. 30m, 2h or 1d"))?;
    if !(SCHEDULE_MIN_MINUTES..=SCHEDULE_MAX_MINUTES).contains(&minutes) {
        return Err(format!("the interval must be from {SCHEDULE_MIN_MINUTES}m to {}d", SCHEDULE_MAX_MINUTES / 1440));
    }
    Ok(minutes)
}
/// `at` 일시. `2026-11-20 09:00` 또는 `2026-11-20T09:00`, 로컬 시간.
pub fn schedule_at(value: &str) -> Result<String, String> {
    let value = value.trim().replacen('T', " ", 1);
    let (date, time) = value.split_once(' ').ok_or_else(|| format!("invalid date and time {value:?}; use YYYY-MM-DD HH:MM"))?;
    let date = chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").map_err(|_| format!("invalid date {date:?}; use YYYY-MM-DD"))?;
    Ok(format!("at {} {}", date.format("%Y-%m-%d"), schedule_time(time.trim())?))
}
/// `office-task --every` 값. `30m`·`2h`·`1d`는 간격, `daily@09:00`은 매일, `mon,wed@09:00`은 매주다.
pub fn schedule_every(value: &str) -> Result<String, String> {
    let value = value.trim();
    match value.split_once('@') {
        None => Ok(format!("every {}m", schedule_interval(value)?)),
        Some((days, time)) if days.trim().eq_ignore_ascii_case("daily") => Ok(format!("daily {}", schedule_time(time.trim())?)),
        Some((days, time)) => {
            let days = schedule_days(days)?;
            let time = schedule_time(time.trim())?;
            if days.len() == 7 { return Ok(format!("daily {time}")); }
            Ok(format!("weekly {} {time}", days.iter().map(|d| WEEKDAYS[*d]).collect::<Vec<_>>().join(",")))
        }
    }
}
/// 수신함 `schedule:` 값을 정규형으로. `none`은 예약을 지우라는 뜻이다(수정 요청에서만 의미가 있다).
pub fn normalize_schedule(spec: &str) -> Result<String, String> {
    let spec = spec.trim();
    let (kind, rest) = spec.split_once(char::is_whitespace).map_or((spec, ""), |(k, r)| (k, r.trim()));
    match kind.to_ascii_lowercase().as_str() {
        "none" if rest.is_empty() => Ok("none".into()),
        "at" => schedule_at(rest),
        "every" => Ok(format!("every {}m", schedule_interval(rest)?)),
        "daily" => Ok(format!("daily {}", schedule_time(rest)?)),
        "weekly" => {
            let (days, time) = rest.rsplit_once(char::is_whitespace).ok_or_else(|| "weekly needs days and a time, e.g. weekly mon,wed 09:00".to_string())?;
            schedule_every(&format!("{}@{}", days.trim(), time.trim()))
        }
        _ => Err(format!("invalid schedule {spec:?}; use at YYYY-MM-DD HH:MM, every 30m, daily HH:MM, weekly mon,wed HH:MM or none")),
    }
}

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub title: String,
    pub body: String,
    pub assignee: Option<String>,
    /// 생산자가 정한 업무 id. 형식이 틀리면 버리고 해시 key로 만든다.
    pub id: Option<String>,
    /// 선행 업무 id. 앱이 가져올 때 목록에 있는 것만 남기고 나머지는 경고로 남긴다.
    pub after: Vec<String>,
    /// 플랜 루트(`plan: true`). 자동 진행이 실행하지 않는 플랜 메모 전용 업무다.
    pub plan: bool,
    pub limits: RunLimitsPatch,
    /// 예약 정규형(`normalize_schedule`). 앱은 꺼진 채로 받는다.
    pub schedule: Option<String>,
    /// 완료 요청. 비어 있지 않으면 새 업무가 아니라 이 id들을 완료로 바꾸는 파일이다.
    pub done: Vec<String>,
    /// 삭제 요청. 비어 있지 않으면 이 id들을 지우는 파일이다.
    pub delete: Vec<String>,
    /// 수정 요청. 있으면 새 업무가 아니라 기존 업무 하나를 고치는 파일이다.
    pub edit: Option<Edit>,
}
/// 기존 업무 하나의 수정 요청. 각 필드는 키가 있을 때만 바꾼다.
#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// 빈 문자열이면 미배정으로 바꾼다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// 선행 업무 전체를 이 목록으로 바꾼다. 비어 있으면 모두 뺀다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub add_after: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remove_after: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// 플랜 루트 표시를 켜거나 끈다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<bool>,
    #[serde(flatten)] pub limits: RunLimitsPatch,
    /// 예약 정규형. `none`이면 지운다. 바꾼 예약은 앱이 꺼 둔다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
}
impl Edit {
    fn is_empty(&self) -> bool {
        self.title.is_none() && self.body.is_none() && self.assignee.is_none() && self.after.is_none()
            && self.add_after.is_empty() && self.remove_after.is_empty() && self.status.is_none() && self.plan.is_none() && self.limits == RunLimitsPatch::default() && self.schedule.is_none()
    }
}
/// 업무 상태 값. `office-task --edit --status`와 수신함 `status` 키가 받는 값이다.
pub const STATUSES: [&str; 4] = ["todo", "in_progress", "review", "done"];
#[derive(Debug, PartialEq)]
pub enum RejectReason { InvalidUtf8, TooLarge, UnclosedFrontmatter, Empty, InvalidLimit(String), InvalidSchedule(String), ControlChar(char) }
impl RejectReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "invalid-utf8", Self::TooLarge => "too-large",
            Self::UnclosedFrontmatter => "unclosed-frontmatter", Self::Empty => "empty",
            Self::ControlChar(_) => "control-char", Self::InvalidLimit(_) => "invalid-run-limit",
            Self::InvalidSchedule(_) => "invalid-schedule",
        }
    }
    /// `.error` 둘째 줄. 제어문자 거부는 첫 문자를 `U+001B`처럼 적는다.
    pub fn detail(&self) -> Option<String> {
        match self { Self::InvalidLimit(reason) | Self::InvalidSchedule(reason) => Some(reason.clone()), Self::ControlChar(c) => Some(format!("U+{:04X}", *c as u32)), _ => None }
    }
}
/// 업무 제목·본문에 둘 수 없는 문자. 업무 보내기(`work_send`)와 같은 기준으로 탭·줄바꿈만 허용한다.
pub fn forbidden_control(text: &str) -> Option<char> {
    text.chars().find(|c| c.is_control() && !matches!(c, '\n' | '\t'))
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxEntry {
    pub file_name: String,
    pub key: String,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub plan: bool,
    #[serde(flatten)] pub limits: RunLimitsPatch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub done: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub delete: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<Edit>,
}

/// 수신함으로 지정할 수 있는 업무 id. 앱이 만든 id(nanoid)도 이 형식이다.
pub fn valid_task_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
/// `plan:` 값. 참·거짓으로 읽을 수 없으면 None(키가 없는 것과 같다).
pub fn parse_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}
/// `after:` 값은 쉼표나 공백으로 나눈다. 형식이 틀린 id는 버리고 중복은 합친다.
pub fn parse_after(value: &str) -> Vec<String> {
    let mut ids: Vec<String> = vec![];
    for id in value.split(|c: char| c == ',' || c.is_whitespace()).filter(|id| valid_task_id(id)) {
        if !ids.iter().any(|known| known == id) { ids.push(id.to_string()); }
    }
    ids
}

pub fn parse(_name: &str, bytes: &[u8]) -> Result<Parsed, RejectReason> {
    if bytes.len() > MAX_BYTES { return Err(RejectReason::TooLarge); }
    let text = std::str::from_utf8(bytes).map_err(|_| RejectReason::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    let mut lines = text.split('\n').peekable();
    let mut assignee = None;
    let mut id = None;
    let mut after = vec![];
    let mut done = vec![];
    let mut delete = vec![];
    // 수정 요청은 키가 있는지가 뜻을 가지므로(빈 `assignee:`는 미배정) 값을 따로 모은다.
    let mut edit_target: Option<String> = None;
    let mut edit = Edit::default();
    let mut replace_body = false;
    if lines.peek() == Some(&"---") {
        lines.next();
        let mut closed = false;
        for line in lines.by_ref() {
            if line == "---" { closed = true; break; }
            if let Some((key, value)) = line.split_once(':') {
                match key.trim() {
                    "assignee" => {
                        assignee = Some(value.trim().to_string()).filter(|v| !v.is_empty());
                        edit.assignee = Some(value.trim().to_string());
                    }
                    "id" => id = Some(value.trim().to_string()).filter(|v| valid_task_id(v)),
                    "after" => { after = parse_after(value); edit.after = Some(after.clone()); }
                    "done" => done = parse_after(value),
                    "delete" => delete = parse_after(value),
                    "edit" => edit_target = Some(value.trim().to_string()).filter(|v| valid_task_id(v)),
                    "title" => edit.title = Some(value.trim().to_string()),
                    "add-after" => edit.add_after = parse_after(value),
                    "remove-after" => edit.remove_after = parse_after(value),
                    "status" => edit.status = Some(value.trim().to_string()).filter(|v| STATUSES.contains(&v.as_str())),
                    key @ ("max-follow-ups" | "permission-wait-minutes") => edit.limits.set(key, value).map_err(RejectReason::InvalidLimit)?,
                    "plan" => edit.plan = parse_flag(value),
                    "schedule" => edit.schedule = Some(normalize_schedule(value).map_err(RejectReason::InvalidSchedule)?),
                    "body" => replace_body = true,
                    _ => {}
                }
            }
        }
        if !closed { return Err(RejectReason::UnclosedFrontmatter); }
    }
    let rest: Vec<&str> = lines.collect();
    let mut lines = rest.iter().copied();
    let first = lines.by_ref().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    // 빈 제목은 `# `로 쓴다. trim 뒤 `#`만 남으므로 그것도 빈 제목으로 본다.
    let title = if first == "#" { "" } else { first.strip_prefix("# ").unwrap_or(first) }.trim().to_string();
    let body = trim_blank_lines(&lines.collect::<Vec<_>>());
    // 한 파일은 요청 하나다: 완료 → 삭제 → 수정 → 새 업무 순으로 먼저 있는 것으로 읽는다.
    if !done.is_empty() || !delete.is_empty() {
        if !done.is_empty() { delete.clear(); }
        return Ok(Parsed { title: String::new(), body: String::new(), assignee: None, id: None, after: vec![], plan: false, limits: Default::default(), schedule: None, done, delete, edit: None });
    }
    if let Some(id) = edit_target {
        edit.id = id;
        // 수정 요청의 본문은 `body` 키가 있을 때만 frontmatter 뒤 전체(제목 줄 규칙 없이)다.
        if replace_body { edit.body = Some(trim_blank_lines(&rest)); }
        if edit.is_empty() { return Err(RejectReason::Empty); }
        let text = [edit.title.as_deref(), edit.body.as_deref(), edit.assignee.as_deref()];
        if let Some(c) = text.into_iter().flatten().find_map(forbidden_control) { return Err(RejectReason::ControlChar(c)); }
        return Ok(Parsed { title: String::new(), body: String::new(), assignee: None, id: None, after: vec![], plan: false, limits: Default::default(), schedule: None, done, delete, edit: Some(edit) });
    }
    if title.is_empty() && body.trim().is_empty() { return Err(RejectReason::Empty); }
    // ESC 등이 남으면 `office-task --list`·터미널에서 해석되고 업무 보내기도 거절하므로 받지 않는다.
    if let Some(c) = forbidden_control(&title).or_else(|| forbidden_control(&body)) { return Err(RejectReason::ControlChar(c)); }
    let plan = edit.plan.unwrap_or(false);
    // 새 업무에서 `none`은 예약이 없는 것과 같다.
    let schedule = edit.schedule.filter(|spec| spec != "none");
    Ok(Parsed { title, body, assignee, id, after, plan, limits: edit.limits, schedule, done, delete, edit: None })
}
/// 본문의 들여쓰기와 trailing spaces는 보존하고 앞뒤 빈 줄만 다듬는다.
fn trim_blank_lines(lines: &[&str]) -> String {
    let start = lines.iter().position(|s| !s.trim().is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|s| !s.trim().is_empty()).map_or(start, |i| i + 1);
    lines[start..end].join("\n")
}
fn accepted_name(name: &str) -> bool {
    !name.starts_with('.') && name.ends_with(".md") && !name.contains(['/', '\\']) && !name.contains("..")
}
/// 렌더러가 판정한 의미 오류 거부. `reason`은 `.error` 첫 줄, `detail`은 둘째 줄이다.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejection {
    pub file_name: String,
    pub reason: String,
    #[serde(default)]
    pub detail: Option<String>,
}
/// 업무는 만들었지만 입력 일부를 버리거나 바꾼 기록.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub file_name: String,
    pub task_id: String,
    pub code: String,
    pub detail: String,
}
fn valid_code(code: &str) -> bool {
    (1..=32).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}
/// 기록 파일 한 칸에 들어갈 값. 제어 문자는 공백으로 바꾸고 길이를 자른다.
fn field(value: &str) -> String {
    value.chars().map(|c| if c.is_control() { ' ' } else { c }).take(512).collect()
}
/// `root/name`을 `root/rejected/`로 옮기고 이유를 `<name>.error`에 남긴다. 토의 수신함도 쓴다.
pub(super) fn reject(root: &Path, name: &str, code: &str, detail: Option<&str>) -> std::io::Result<()> {
    let dir = root.join("rejected");
    fs::create_dir_all(&dir)?;
    // 같은 생산자가 이름을 재사용해도 이전 거부 파일을 보존한다.
    let target = if dir.join(name).exists() || dir.join(format!("{name}.error")).exists() {
        dir.join(format!("{}-{name}", uuid::Uuid::new_v4()))
    } else { dir.join(name) };
    fs::rename(root.join(name), &target)?;
    let mut error = fs::File::create(target.with_file_name(format!("{}.error", target.file_name().unwrap().to_string_lossy())))?;
    writeln!(error, "{code}")?;
    if let Some(detail) = detail { writeln!(error, "{}", field(detail))?; }
    Ok(())
}
pub fn scan(root: &Path, now: SystemTime) -> Result<Vec<InboxEntry>, String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let mut names = vec![];
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        // 디렉터리와 심볼릭 링크는 수신함 파일로 취급하지 않는다.
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() { continue; }
        let Ok(name) = entry.file_name().into_string() else { continue; };
        if name.starts_with('.') && name.ends_with(".tmp") {
            let old = entry.metadata().and_then(|m| m.modified()).ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > Duration::from_secs(24 * 3600));
            if old { fs::remove_file(entry.path()).map_err(|e| e.to_string())?; }
        } else if accepted_name(&name) { names.push(name); }
    }
    names.sort();
    let mut entries = vec![];
    for name in names.into_iter().take(MAX_BATCH) {
        let file = match fs::File::open(root.join(&name)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = vec![];
        file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        match parse(&name, &bytes) {
            Ok(parsed) => {
                let mut hash = Sha256::new();
                hash.update(name.as_bytes()); hash.update([0]); hash.update(&bytes);
                let key = format!("{:x}", hash.finalize())[..16].to_string();
                entries.push(InboxEntry { file_name: name, key, title: parsed.title, body: parsed.body, assignee: parsed.assignee, id: parsed.id, after: parsed.after, plan: parsed.plan, limits: parsed.limits, schedule: parsed.schedule, done: parsed.done, delete: parsed.delete, edit: parsed.edit });
            }
            Err(reason) => reject(root, &name, reason.code(), reason.detail().as_deref()).map_err(|e| e.to_string())?,
        }
    }
    Ok(entries)
}
pub fn ack(root: &Path, file_names: &[String]) -> Result<(), String> {
    for name in file_names.iter().filter(|name| accepted_name(name)) {
        match fs::remove_file(root.join(name)) {
            Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
/// 의미 오류 파일을 `rejected/`로 옮긴다. 안전하지 않은 이름·사유 코드는 무시하고 이미 없으면 건너뛴다.
pub fn reject_files(root: &Path, rejections: &[Rejection]) -> Result<(), String> {
    for rejection in rejections.iter().filter(|r| accepted_name(&r.file_name) && valid_code(&r.reason)) {
        if !root.join(&rejection.file_name).is_file() { continue; }
        reject(root, &rejection.file_name, &rejection.reason, rejection.detail.as_deref()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
/// `warnings.log`에 시각·파일·업무 id·코드·값을 탭으로 나눠 한 줄씩 덧붙인다.
pub fn append_warnings(root: &Path, warnings: &[Warning], now: chrono::DateTime<chrono::Utc>) -> Result<(), String> {
    let lines: String = warnings.iter().filter(|w| valid_code(&w.code)).map(|w| format!(
        "{}\t{}\t{}\t{}\t{}\n", now.format("%Y-%m-%dT%H:%M:%SZ"), field(&w.file_name), field(&w.task_id), w.code, field(&w.detail),
    )).collect();
    if lines.is_empty() { return Ok(()); }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = root.join(WARNINGS_LOG);
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_WARNINGS_BYTES) {
        fs::rename(&path, root.join(format!("{WARNINGS_LOG}.1"))).map_err(|e| e.to_string())?;
    }
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| e.to_string())?;
    file.write_all(lines.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_plain_frontmatter_bom_crlf_and_preserves_body_indentation() {
        assert_eq!(parse("x.md", b"\n# Title\n\n  code  \n\n").unwrap(), Parsed { title: "Title".into(), body: "  code  ".into(), assignee: None, id: None, after: vec![], plan: false, limits: Default::default(), schedule: None, done: vec![], delete: vec![], edit: None });
        let p = parse("x.md", "\u{feff}---\r\nfuture: ignored\r\nassignee: 하나 \r\n---\r\n# 제목\r\n본문\r\n".as_bytes()).unwrap();
        assert_eq!(p, Parsed { title: "제목".into(), body: "본문".into(), assignee: Some("하나".into()), id: None, after: vec![], plan: false, limits: Default::default(), schedule: None, done: vec![], delete: vec![], edit: None });
        assert_eq!(parse("x.md", b"Title").unwrap().body, "");
        assert_eq!(parse("x.md", b"# \nBody").unwrap(), Parsed { title: "".into(), body: "Body".into(), assignee: None, id: None, after: vec![], plan: false, limits: Default::default(), schedule: None, done: vec![], delete: vec![], edit: None });
    }
    #[test]
    fn parses_task_id_and_prerequisites() {
        // 자기 자신은 앱이 가져올 때 경고와 함께 뺀다.
        let p = parse("x.md", b"---\nid: t-ab12\nafter: t-1, t-2 t-1,bad/id,t-ab12\n---\n# Step\n").unwrap();
        assert_eq!((p.id.as_deref(), p.after), (Some("t-ab12"), vec!["t-1".to_string(), "t-2".to_string(), "t-ab12".to_string()]));
        let p = parse("x.md", b"---\nid: not valid\nafter:\n---\n# Step\n").unwrap();
        assert_eq!((p.id, p.after), (None, vec![]));
        assert!(!parse("x.md", b"# Step\n").unwrap().plan);
        assert!(parse("x.md", b"---\nplan: true\nafter: t-1\n---\n# Goal\n").unwrap().plan);
        assert!(valid_task_id("vLdkeylcT4Nb9W4IQVT4g") && valid_task_id("inbox-0123abcd"));
        assert!(!valid_task_id("") && !valid_task_id(&"a".repeat(65)) && !valid_task_id("a.b"));
    }
    #[test]
    fn parses_completion_requests_without_title() {
        let p = parse("x.md", b"---\ndone: t-1, t-2\n---\n").unwrap();
        assert_eq!((p.done, p.title.as_str()), (vec!["t-1".to_string(), "t-2".to_string()], ""));
        assert_eq!(parse("x.md", b"---\ndone: a.b\n---\n"), Err(RejectReason::Empty));
    }
    #[test]
    fn parses_delete_and_edit_requests() {
        let p = parse("x.md", b"---\ndelete: t-1 t-2\n---\n").unwrap();
        assert_eq!((p.delete, p.edit), (vec!["t-1".to_string(), "t-2".to_string()], None));
        // 키가 있는 필드만 바꾼다. 빈 assignee·after는 비우라는 뜻이다.
        let p = parse("x.md", b"---\nedit: t-1\ntitle:  New: title \nassignee:\nafter:\nadd-after: t-3\nremove-after: t-4,t-5\nstatus: review\nbody: replace\n---\n\n# not a title\n  kept\n\n").unwrap();
        assert_eq!(p.edit, Some(Edit {
            id: "t-1".into(), title: Some("New: title".into()), body: Some("# not a title\n  kept".into()), assignee: Some("".into()),
            after: Some(vec![]), add_after: vec!["t-3".into()], remove_after: vec!["t-4".into(), "t-5".into()], status: Some("review".into()), plan: None, limits: Default::default(), schedule: None,
        }));
        assert_eq!(parse("x.md", b"---\nedit: t-1\nplan: no\n---\n").unwrap().edit.unwrap().plan, Some(false));
        assert_eq!(parse("x.md", b"---\nedit: t-1\nplan: maybe\n---\n"), Err(RejectReason::Empty));
        let p = parse("x.md", b"---\nedit: t-1\nbody: replace\n---\n").unwrap();
        assert_eq!(p.edit.unwrap().body.as_deref(), Some(""));
        // 본문 키가 없으면 frontmatter 뒤는 무시한다.
        assert_eq!(parse("x.md", b"---\nedit: t-1\nstatus: done\n---\nignored").unwrap().edit.unwrap().body, None);
        assert_eq!(parse("x.md", b"---\nedit: t-1\nstatus: bogus\n---\n"), Err(RejectReason::Empty));
        assert_eq!(parse("x.md", b"---\nedit: t-1\ntitle: a\x1bb\n---\n"), Err(RejectReason::ControlChar('\u{1b}')));
        // 형식이 틀린 대상이면 수정 요청이 아니라 일반 업무로 읽는다.
        assert_eq!(parse("x.md", b"---\nedit: bad id\n---\n# Task").unwrap().title, "Task");
    }
    #[test]
    fn parses_and_normalizes_schedules() {
        assert_eq!(normalize_schedule("at 2026-11-20 9:05").as_deref(), Ok("at 2026-11-20 09:05"));
        assert_eq!(normalize_schedule("at 2026-11-20T09:05").as_deref(), Ok("at 2026-11-20 09:05"));
        assert_eq!(normalize_schedule("every 2h").as_deref(), Ok("every 120m"));
        assert_eq!(normalize_schedule("every 1d").as_deref(), Ok("every 1440m"));
        assert_eq!(normalize_schedule("daily 7:30").as_deref(), Ok("daily 07:30"));
        assert_eq!(normalize_schedule("weekly Wed,mon,wed 09:00").as_deref(), Ok("weekly mon,wed 09:00"));
        assert_eq!(normalize_schedule("weekly weekdays 09:00").as_deref(), Ok("weekly mon,tue,wed,thu,fri 09:00"));
        assert_eq!(normalize_schedule("NONE").as_deref(), Ok("none"));
        for bad in ["", "every 4m", "every 31d", "every 5x", "daily 24:00", "daily 9:5", "at 2026-13-01 09:00", "at tomorrow", "weekly 09:00", "weekly funday 09:00", "hourly"] {
            assert!(normalize_schedule(bad).is_err(), "{bad}");
        }
        assert_eq!(schedule_every("daily@9:00").as_deref(), Ok("daily 09:00"));
        assert_eq!(schedule_every("sat,sun@10:00").as_deref(), Ok("weekly sun,sat 10:00"));
        assert_eq!(schedule_every("mon,tue,wed,thu,fri,sat,sun@10:00").as_deref(), Ok("daily 10:00"));
        assert_eq!(schedule_every("90m").as_deref(), Ok("every 90m"));
        let p = parse("x.md", b"---
schedule: daily 09:00
---
# Daily check
").unwrap();
        assert_eq!(p.schedule.as_deref(), Some("daily 09:00"));
        assert_eq!(parse("x.md", b"---
schedule: none
---
# Task
").unwrap().schedule, None);
        assert!(matches!(parse("x.md", b"---
schedule: sometimes
---
# Task
"), Err(RejectReason::InvalidSchedule(_))));
        assert_eq!(parse("x.md", b"---
edit: t-1
schedule: none
---
").unwrap().edit.unwrap().schedule.as_deref(), Some("none"));
    }
    #[test]
    fn rejects_malformed_content_and_accepts_exact_size_limit() {
        assert_eq!(parse("x.md", b"---\nassignee: a"), Err(RejectReason::UnclosedFrontmatter));
        assert_eq!(parse("x.md", b"\n \n"), Err(RejectReason::Empty));
        assert_eq!(parse("x.md", &[255]), Err(RejectReason::InvalidUtf8));
        assert_eq!(parse("x.md", &vec![b'a'; MAX_BYTES + 1]), Err(RejectReason::TooLarge));
        assert!(parse("x.md", &vec![b'a'; MAX_BYTES]).is_ok());
    }
    #[test]
    fn rejects_control_characters_except_tab_and_newline() {
        assert_eq!(parse("x.md", b"# a\x1b]52;c;aGk=\x07"), Err(RejectReason::ControlChar('\u{1b}')));
        assert_eq!(parse("x.md", b"# a\nline\rover"), Err(RejectReason::ControlChar('\r')));
        assert_eq!(parse("x.md", "# a\n\u{9b}31m".as_bytes()), Err(RejectReason::ControlChar('\u{9b}')));
        assert_eq!(parse("x.md", b"# a\tb\r\n\tindented\r\n").unwrap().body, "\tindented");
        assert!(parse("x.md", b"---\ndone: t-1\n---\n\x1b").is_ok());
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("esc.md"), "# \x1b[8mhidden").unwrap();
        assert!(scan(root.path(), SystemTime::now()).unwrap().is_empty());
        assert_eq!(fs::read_to_string(root.path().join("rejected/esc.md.error")).unwrap(), "control-char\nU+001B\n");
    }
    #[test]
    fn scans_sorted_limited_and_deterministic_then_acks_safe_names() {
        let root = tempfile::tempdir().unwrap();
        for i in (0..55).rev() { fs::write(root.path().join(format!("{i:03}.md")), "Task").unwrap(); }
        fs::write(root.path().join(".hidden.md"), "Ignore").unwrap();
        fs::write(root.path().join("partial.tmp"), "Ignore").unwrap();
        let first = scan(root.path(), SystemTime::now()).unwrap();
        assert_eq!(first.len(), 50); assert_eq!(first[0].file_name, "000.md");
        assert_eq!(first[49].file_name, "049.md");
        assert_eq!(first[0].key, scan(root.path(), SystemTime::now()).unwrap()[0].key);
        assert_eq!(first[0].key.len(), 16);
        ack(root.path(), &["000.md".into(), "missing.md".into(), "../escape.md".into(), "a\\x.md".into(), ".hidden.md".into(), "partial.tmp".into()]).unwrap();
        assert!(!root.path().join("000.md").exists()); assert!(root.path().join(".hidden.md").exists());
    }
    #[test]
    fn rejects_and_cleans_only_stale_hidden_temps() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("bad.md"), [255]).unwrap();
        fs::write(root.path().join(".stale.tmp"), "partial").unwrap();
        fs::write(root.path().join("visible.tmp"), "partial").unwrap();
        assert!(scan(root.path(), SystemTime::now() + Duration::from_secs(24 * 3600 + 1)).unwrap().is_empty());
        assert!(!root.path().join(".stale.tmp").exists()); assert!(root.path().join("visible.tmp").exists());
        assert_eq!(fs::read(root.path().join("rejected/bad.md")).unwrap(), [255]);
        assert_eq!(fs::read_to_string(root.path().join("rejected/bad.md.error")).unwrap(), "invalid-utf8\n");
        fs::write(root.path().join("bad.md"), "---").unwrap();
        scan(root.path(), SystemTime::now()).unwrap();
        assert_eq!(fs::read_dir(root.path().join("rejected")).unwrap().count(), 4);
    }
    #[test]
    fn rejects_semantic_errors_with_detail_and_skips_unsafe_or_missing() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("dup.md"), "---\nid: t-1\n---\n# New").unwrap();
        fs::write(root.path().join("keep.md"), "Keep").unwrap();
        let rejection = |name: &str, reason: &str, detail: Option<&str>| Rejection { file_name: name.into(), reason: reason.into(), detail: detail.map(String::from) };
        reject_files(root.path(), &[
            rejection("dup.md", "duplicate-id", Some("t-1\nx")),
            rejection("gone.md", "unknown-task", None),
            rejection("../keep.md", "unknown-task", None),
            rejection("keep.md", "Bad Code", None),
        ]).unwrap();
        assert!(!root.path().join("dup.md").exists()); assert!(root.path().join("keep.md").exists());
        assert_eq!(fs::read_to_string(root.path().join("rejected/dup.md")).unwrap(), "---\nid: t-1\n---\n# New");
        assert_eq!(fs::read_to_string(root.path().join("rejected/dup.md.error")).unwrap(), "duplicate-id\nt-1 x\n");
        assert_eq!(fs::read_dir(root.path().join("rejected")).unwrap().count(), 2);
    }
    #[test]
    fn appends_warnings_and_rotates_large_log() {
        let root = tempfile::tempdir().unwrap();
        let now: chrono::DateTime<chrono::Utc> = "2026-10-14T01:02:03Z".parse().unwrap();
        let warning = |code: &str, detail: &str| Warning { file_name: "a.md".into(), task_id: "t-1".into(), code: code.into(), detail: detail.into() };
        append_warnings(root.path(), &[warning("unknown-assignee", "없는\t사람"), warning("BAD", "x")], now).unwrap();
        append_warnings(root.path(), &[warning("unknown-prerequisite", "t-ghost")], now).unwrap();
        append_warnings(root.path(), &[], now).unwrap();
        assert_eq!(fs::read_to_string(root.path().join(WARNINGS_LOG)).unwrap(),
            "2026-10-14T01:02:03Z\ta.md\tt-1\tunknown-assignee\t없는 사람\n2026-10-14T01:02:03Z\ta.md\tt-1\tunknown-prerequisite\tt-ghost\n");
        fs::write(root.path().join(WARNINGS_LOG), vec![b'x'; MAX_WARNINGS_BYTES as usize + 1]).unwrap();
        append_warnings(root.path(), &[warning("self-prerequisite", "t-1")], now).unwrap();
        assert_eq!(fs::metadata(root.path().join(format!("{WARNINGS_LOG}.1"))).unwrap().len(), MAX_WARNINGS_BYTES + 1);
        assert!(fs::read_to_string(root.path().join(WARNINGS_LOG)).unwrap().ends_with("\tself-prerequisite\tt-1\n"));
        // scan은 기록 파일을 업무로 읽지 않는다.
        assert!(scan(root.path(), SystemTime::now()).unwrap().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn ignores_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let body = b"# External task\nMust not enter the inbox\n";
        fs::write(outside.path(), body).unwrap();
        assert!(parse("link.md", body).is_ok(), "target must otherwise be a valid task");
        let link = root.path().join("link.md");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(scan(root.path(), SystemTime::now()).unwrap().is_empty());
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(outside.path()).unwrap(), body);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
