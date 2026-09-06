// src-tauri/src/session/agy_hooks_file.rs
//
// 전역 `~/.gemini/config/hooks.json` 병합 기록기(docs/antigravity-support-
// design.md §3.2). Antigravity CLI(agy)는 세션별로 훅 파일을 넘길 방법이
// 없어(pi의 `-e <파일>`, claude의 `--settings <파일>`과 달리) 사용자 전역
// 훅 파일에 우리 항목 하나(`"agent-office"` 키)를 심는다.
//
// 최상위 키가 훅 "묶음" 이름이고(agy 훅 체계 §1.1) 같은 이벤트에 여러 묶음이
// 있으면 순서대로 모두 실행되므로, 우리 키만 갈아 끼우면 다른 도구(예:
// `orca-status`)가 심어 둔 항목은 그대로 살아남는다. **파싱에 실패하면
// (다른 도구가 깨진 JSON을 남겼거나 사용자가 수동 편집 중) 파일에 전혀
// 손대지 않는다** — 사용자 파일을 망가뜨리는 것보다 이번 부팅에서 관찰을
// 포기하는 편이 낫다(설계 결정).

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

const AGENT_OFFICE_KEY: &str = "agent-office";

/// hook.sh 절대 경로로부터 `"agent-office"` 키에 들어갈 값을 만든다.
/// 이벤트는 셋뿐이다: PreInvocation(평면 배열), PostToolUse(matcher 묶음),
/// Stop(평면 배열). 명령은 `AGENT_OFFICE_AGY_EVENT=<Event> sh '<hook.sh>'`
/// 꼴 — 셸이 이벤트 이름으로 source를 정한다(agy_hook.rs).
pub fn build_agent_office_entry(hook_script: &Path) -> Value {
    // 리뷰 지적: 경로를 수동으로 `'...'`로만 감싸면 경로 안에 작은따옴표가
    // 있을 때(드물지만 사용자 홈 디렉터리 이름에 따라 가능) 명령이 깨진다.
    // wrapper_script.rs의 POSIX 인용 규칙(sh_quote)을 그대로 재사용한다.
    let hook = crate::session::wrapper_script::sh_quote(&hook_script.to_string_lossy());
    let command_for = |event: &str| format!("AGENT_OFFICE_AGY_EVENT={event} sh {hook}");
    serde_json::json!({
        "PreInvocation": [
            { "type": "command", "command": command_for("PreInvocation"), "timeout": 5 },
        ],
        "PostToolUse": [
            {
                "matcher": "*",
                "hooks": [
                    { "type": "command", "command": command_for("PostToolUse"), "timeout": 5 },
                ],
            },
        ],
        "Stop": [
            { "type": "command", "command": command_for("Stop"), "timeout": 5 },
        ],
    })
}

/// `hooks_file`에서 최상위 `"agent-office"` 키만 갈아 끼워 쓴다.
///
/// - 파일이 없으면 새로 만든다.
/// - 파일이 있는데 파싱에 실패하거나 최상위가 객체가 아니면, **아무것도
///   쓰지 않고** 경고만 남긴다(사용자 파일 보존 우선).
/// - 그 외 모든 다른 최상위 키(예: `orca-status`)는 그대로 보존한다.
pub fn ensure_agy_hooks_merged(hooks_file: &Path, hook_script: &Path) -> io::Result<()> {
    let mut root: Map<String, Value> = match std::fs::read(hooks_file) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(map)) => map,
            Ok(_) => {
                eprintln!(
                    "agent-office: {} 최상위가 객체가 아니라 손대지 않습니다",
                    hooks_file.display(),
                );
                return Ok(());
            }
            Err(error) => {
                eprintln!(
                    "agent-office: {} 파싱 실패({error}) — 손대지 않습니다",
                    hooks_file.display(),
                );
                return Ok(());
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(error) => return Err(error),
    };

    root.insert(AGENT_OFFICE_KEY.to_string(), build_agent_office_entry(hook_script));

    if let Some(parent) = hooks_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&Value::Object(root))?;
    write_atomic(hooks_file, &bytes)
}

/// tmp→rename 원자 쓰기(persistence::agy_resume_store::AgyResumeStore::save_file과
/// 같은 패턴). 이 파일은 **사용자 소유**의 전역 설정 파일이라, 쓰는 도중
/// 앱이 죽거나(다른 프로세스가 동시에 읽는 등) 부분 쓰기가 보이는 사고를
/// 특히 더 피해야 한다.
fn write_atomic(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("hooks.json");
    let tmp = file.with_file_name(format!("{name}.tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, file) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// 프로덕션 경로: `~/.gemini/config/hooks.json`.
pub fn default_hooks_file() -> PathBuf {
    PathBuf::from(crate::session::manager::home_dir())
        .join(".gemini")
        .join("config")
        .join("hooks.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_file() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-agy-hooks-file-test-{}/hooks.json",
            uuid::Uuid::new_v4(),
        ))
    }

    fn hook_script_path() -> PathBuf {
        PathBuf::from("/tmp/agent-office/observer/agy/hook.sh")
    }

    #[test]
    fn creates_file_with_agent_office_key_when_missing() {
        let file = scratch_file();
        ensure_agy_hooks_merged(&file, &hook_script_path()).expect("merge succeeds");

        let value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(value.get(AGENT_OFFICE_KEY).is_some());
        assert!(value["agent-office"]["PreInvocation"].is_array());
        assert!(value["agent-office"]["PostToolUse"][0]["matcher"] == "*");
        assert!(value["agent-office"]["Stop"].is_array());

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// 설계 §3.2 핵심 계약: 다른 도구(orca)가 심어 둔 최상위 키는 병합 뒤에도
    /// 값이 그대로 살아남아야 한다(재직렬화 + 원자 쓰기라 포맷·키 순서는
    /// 바뀔 수 있지만 값은 보존된다).
    #[test]
    fn merge_preserves_other_top_level_keys_value() {
        let file = scratch_file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let existing = serde_json::json!({
            "orca-status": {
                "Stop": [{"type": "command", "command": "echo orca", "timeout": 3}],
            },
        });
        std::fs::write(&file, serde_json::to_vec_pretty(&existing).unwrap()).unwrap();

        ensure_agy_hooks_merged(&file, &hook_script_path()).expect("merge succeeds");

        let value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(value["orca-status"], existing["orca-status"]);
        assert!(value.get(AGENT_OFFICE_KEY).is_some());

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// 재병합(부팅마다 재실행)은 우리 키만 갈아 끼우고 다른 키를 또 보존해야
    /// 한다 — 매번 다시 써도 안전한지(pi 확장과 같은 성질) 검증.
    #[test]
    fn re_merging_is_idempotent_and_still_preserves_other_keys() {
        let file = scratch_file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let existing = serde_json::json!({ "orca-status": { "Stop": [] } });
        std::fs::write(&file, serde_json::to_vec_pretty(&existing).unwrap()).unwrap();

        ensure_agy_hooks_merged(&file, &hook_script_path()).unwrap();
        ensure_agy_hooks_merged(&file, &hook_script_path()).unwrap();

        let value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(value["orca-status"], existing["orca-status"]);
        assert_eq!(value.as_object().unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// 설계 §3.2 핵심 계약: 파싱 실패(깨진 JSON)면 파일을 전혀 건드리지 않는다.
    #[test]
    fn corrupted_json_is_left_completely_untouched() {
        let file = scratch_file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let broken = b"{ this is not valid json,,, ".to_vec();
        std::fs::write(&file, &broken).unwrap();

        ensure_agy_hooks_merged(&file, &hook_script_path()).expect("must not error");

        let after = std::fs::read(&file).unwrap();
        assert_eq!(after, broken, "corrupted file must be byte-for-byte unchanged");

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// 최상위가 배열 등 객체가 아니어도 같은 원칙(손대지 않음)을 지킨다.
    #[test]
    fn non_object_top_level_is_left_untouched() {
        let file = scratch_file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let array = b"[1, 2, 3]".to_vec();
        std::fs::write(&file, &array).unwrap();

        ensure_agy_hooks_merged(&file, &hook_script_path()).expect("must not error");

        let after = std::fs::read(&file).unwrap();
        assert_eq!(after, array);

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn entry_commands_carry_the_event_env_and_hook_script_path() {
        let entry = build_agent_office_entry(&hook_script_path());
        let pre = entry["PreInvocation"][0]["command"].as_str().unwrap();
        assert!(pre.starts_with("AGENT_OFFICE_AGY_EVENT=PreInvocation sh "));
        assert!(pre.contains("/tmp/agent-office/observer/agy/hook.sh"));

        let post = entry["PostToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(post.starts_with("AGENT_OFFICE_AGY_EVENT=PostToolUse sh "));

        let stop = entry["Stop"][0]["command"].as_str().unwrap();
        assert!(stop.starts_with("AGENT_OFFICE_AGY_EVENT=Stop sh "));

        assert_eq!(entry["PreInvocation"][0]["timeout"], 5);
        assert_eq!(entry["PostToolUse"][0]["hooks"][0]["timeout"], 5);
        assert_eq!(entry["Stop"][0]["timeout"], 5);
    }

    /// 리뷰 지적 회귀 방지: hook.sh 경로에 작은따옴표가 있으면(드물지만
    /// 가능) 수동 `'{hook}'` 감싸기는 명령을 깨뜨린다 — `sh_quote` 재사용으로
    /// 이스케이프돼야 한다.
    #[test]
    fn hook_script_path_containing_a_single_quote_is_escaped() {
        let path = PathBuf::from("/Users/o'brien/agent-office/observer/agy/hook.sh");
        let entry = build_agent_office_entry(&path);
        let command = entry["Stop"][0]["command"].as_str().unwrap();
        assert_eq!(
            command,
            r#"AGENT_OFFICE_AGY_EVENT=Stop sh '/Users/o'"'"'brien/agent-office/observer/agy/hook.sh'"#,
        );
    }

    #[test]
    fn default_hooks_file_points_at_gemini_config() {
        let path = default_hooks_file();
        assert!(path.ends_with(".gemini/config/hooks.json"));
    }
}
