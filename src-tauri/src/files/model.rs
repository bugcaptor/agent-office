// src-tauri/src/files/model.rs
//
// 파일 보내기의 말: 보내는 쪽·받는 쪽(머신·캐릭터), 전송 한 건(매니페스트·진행·상태), 대상 목록.
// 머신은 연동 기계 id(`link-machine-id`)로 가리킨다 — 호스트·클라이언트 어디서나 같은 값이다.
use serde::{Deserialize, Serialize};

/// 청크 크기. base64로 부풀어도(약 5.4MB) 연동 WebSocket 메시지 상한(64MB) 안에 넉넉히 든다.
pub const CHUNK: u64 = 4 * 1024 * 1024;
/// 한 전송에 담을 수 있는 파일 수.
pub const MAX_FILES: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Party {
    pub machine_id: String,
    pub machine_name: String,
    /// 캐릭터에게 보낼 때(받는 쪽) / 에이전트가 보낼 때(보내는 쪽). 그 머신의 캐릭터 id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
}

impl Party {
    pub fn label(&self) -> String {
        match &self.agent_name {
            Some(a) => format!("{a}@{}", self.machine_name),
            None => self.machine_name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// 보내는 항목 기준 상대 경로(`/`로 잇는다). 첫 마디가 `items` 중 하나다.
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// 보낼 때 본 수정 시각(ms). 이어 보낼 때 원본이 그대로인지 본다.
    #[serde(default)]
    pub mtime: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferState {
    /// 보내는 쪽이 목록·해시를 만드는 중.
    Preparing,
    Sending,
    Done,
    /// 실패하거나 끊겼다. 받은 데까지는 남아 있어 이어 보낼 수 있다.
    Failed,
    Cancelled,
}

impl TransferState {
    pub fn is_final(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }
    pub fn is_active(self) -> bool {
        matches!(self, Self::Preparing | Self::Sending)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub from: Party,
    pub to: Party,
    /// 맨 위 항목 이름(파일이나 폴더).
    pub items: Vec<String>,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    /// 빈 폴더(상대 경로). 받는 쪽이 그대로 만든다.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dirs: Vec<String>,
    pub total_bytes: u64,
    pub done_bytes: u64,
    /// 파일마다 받는 쪽에 확실히 쓴 바이트(앞에서부터 이어진 만큼). 이어 보내기의 기준.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress: Vec<u64>,
    pub state: TransferState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 받는 캐릭터 세션에 "파일이 왔다"를 넣는다.
    #[serde(default)]
    pub notify: bool,
    /// 받는 쪽이 저장한 맨 위 항목의 절대 경로(`items` 순서).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<String>,
    /// 누가 보냈나: `app`·`web`·`agent`·`ctl`.
    #[serde(default)]
    pub via: String,
    /// 보내는 머신에만 두는 원본 절대 경로(`files` 순서). 연동으로 보낼 때는 뺀다.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    /// 끝나면 지울 임시 원본 폴더(웹 업로드). 보내는 머신에만 둔다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<String>,
}

impl Transfer {
    /// 연동으로 보낼 모양(이 머신에만 의미 있는 것을 뺀다).
    pub fn public(&self) -> Transfer {
        Transfer { sources: Vec::new(), cleanup: None, ..self.clone() }
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

/// 보낼 수 있는 곳 하나.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub machine_id: String,
    pub machine_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// 이 머신(목록을 보는 쪽).
    #[serde(default)]
    pub local: bool,
    pub online: bool,
    /// 그 머신이 파일 받기를 켰나.
    pub receive: bool,
}

impl Target {
    pub fn party(&self) -> Party {
        Party {
            machine_id: self.machine_id.clone(),
            machine_name: self.machine_name.clone(),
            agent_id: self.agent_id.clone(),
            agent_name: self.agent_name.clone(),
        }
    }

    /// CLI가 쓰는 이름(`머신`, `캐릭터@머신`).
    pub fn spec(&self) -> String {
        self.party().label()
    }
}

/// 대상 글(`머신`, `캐릭터`, `캐릭터@머신`, 또는 id)을 목록에서 찾는다.
pub fn resolve_target<'a>(targets: &'a [Target], spec: &str) -> Result<&'a Target, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("file-target-empty".into());
    }
    let eq = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let found: Vec<&Target> = if let Some((agent, machine)) = spec.rsplit_once('@') {
        targets
            .iter()
            .filter(|t| {
                let m = eq(&t.machine_name, machine) || eq(&t.machine_id, machine);
                let a = t.agent_name.as_deref().is_some_and(|a| eq(a, agent)) || t.agent_id.as_deref().is_some_and(|a| eq(a, agent));
                m && a
            })
            .collect()
    } else {
        // 머신 이름이 먼저, 그다음 이 머신 캐릭터, 그다음 다른 머신 캐릭터.
        let machines: Vec<&Target> =
            targets.iter().filter(|t| t.agent_id.is_none() && (eq(&t.machine_name, spec) || eq(&t.machine_id, spec))).collect();
        if !machines.is_empty() {
            machines
        } else {
            let agents: Vec<&Target> = targets
                .iter()
                .filter(|t| {
                    t.agent_name.as_deref().is_some_and(|a| eq(a, spec)) || t.agent_id.as_deref().is_some_and(|a| eq(a, spec))
                })
                .collect();
            let local: Vec<&Target> = agents.iter().copied().filter(|t| t.local).collect();
            if !local.is_empty() {
                local
            } else {
                agents
            }
        }
    };
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("file-target-unknown: {spec}")),
        many => Err(format!(
            "file-target-ambiguous: {spec} ({})",
            many.iter().map(|t| t.spec()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// 받는 쪽에서 쓸 수 있는 이름으로(경로 구분·Windows 금지 문자 바꿈, 앞뒤 점·공백 정리).
pub fn safe_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    out = out.trim().trim_end_matches('.').trim().to_string();
    if out.is_empty() || out == "." || out == ".." {
        out = "_".into();
    }
    // Windows 예약 이름.
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        out = format!("_{out}");
    }
    out.chars().take(200).collect()
}

/// 상대 경로를 검사해 마디로 가른다. `..`·절대 경로·빈 마디는 거부한다.
pub fn split_relative(path: &str) -> Result<Vec<String>, String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\0') {
        return Err("file-bad-path".into());
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 64 {
        return Err("file-bad-path".into());
    }
    let mut out = Vec::with_capacity(parts.len());
    for p in parts {
        if p.is_empty() || p == "." || p == ".." || p.contains('\\') || (p.len() >= 2 && p.as_bytes()[1] == b':' && out.is_empty()) {
            return Err("file-bad-path".into());
        }
        out.push(safe_name(p));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(machine: &str, agent: Option<&str>, local: bool) -> Target {
        Target {
            machine_id: format!("id-{machine}"),
            machine_name: machine.into(),
            agent_id: agent.map(|a| format!("a-{a}")),
            agent_name: agent.map(str::to_string),
            local,
            online: true,
            receive: true,
        }
    }

    #[test]
    fn resolves_machines_agents_and_qualified_names() {
        let list = vec![t("mac", None, true), t("mac", Some("Ari"), true), t("pc", None, false), t("pc", Some("Ari"), false), t("pc", Some("Bo"), false)];
        assert_eq!(resolve_target(&list, "PC").unwrap().machine_name, "pc");
        // 이름만 쓰면 이 머신 캐릭터가 먼저.
        assert!(resolve_target(&list, "ari").unwrap().local);
        assert_eq!(resolve_target(&list, "Ari@pc").unwrap().machine_name, "pc");
        assert_eq!(resolve_target(&list, "bo").unwrap().agent_name.as_deref(), Some("Bo"));
        assert!(resolve_target(&list, "zed").unwrap_err().starts_with("file-target-unknown"));
        let dup = vec![t("pc", Some("Bo"), false), t("laptop", Some("Bo"), false)];
        assert!(resolve_target(&dup, "Bo").unwrap_err().starts_with("file-target-ambiguous"));
    }

    #[test]
    fn relative_paths_reject_escapes_and_clean_names() {
        assert_eq!(split_relative("a/b c/d.txt").unwrap(), vec!["a", "b c", "d.txt"]);
        for bad in ["", "/etc/passwd", "a/../b", "a//b", "./a", "C:/x", "a\\b", "\\\\srv"] {
            assert!(split_relative(bad).is_err(), "{bad}");
        }
        assert_eq!(safe_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(safe_name("con.txt"), "_con.txt");
        assert_eq!(safe_name(" .. "), "_");
    }
}
