// src-tauri/src/persistence/work_log_store.rs
//
// 캐릭터 일기(#60) 작업 로그 영속화. `diary_store.rs`와 달리 append-only JSONL이
// 아니라 **per-agent 스냅샷**이다: `worklogs/<agentId>.json` 하나에 그 캐릭터의
// 작업 로그 버퍼 전체를 JSON 배열로 통째 저장한다.
//
// 왜 스냅샷인가: 렌더러 버퍼는 `clear(sessionId)`(일기화 후 소진)와 60개 상한
// trim이 모두 **삭제** 연산이라 append-only면 tombstone/컴팩션이 필요하다. 반면
// 버퍼 최대치는 60항목 × 400자 ≈ 24KB/agent라, 매번 전체 재쓰기가 오히려 단순하고
// 충분히 싸다. 쓰기는 tmp 파일에 쓰고 rename해 원자성을 보장한다(torn write 방지).
//
// agentId 경로 안전성 검증은 png_store/diary_store 선례대로 복제한다(공유 헬퍼가
// 없는 코드베이스 관례).

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::types::WorkLogItem;

#[derive(Debug)]
pub enum WorkLogStoreError {
    /// agentId가 경로 요소로 안전하지 않음(구분자/`..`/빈 문자열).
    InvalidId,
    /// 파일 시스템 오류.
    Io(String),
}

impl std::fmt::Display for WorkLogStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkLogStoreError::InvalidId => write!(f, "invalid agentId (unsafe path element)"),
            WorkLogStoreError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for WorkLogStoreError {}

/// `<dir>/<agentId>.json` 스냅샷들을 관리한다. `dir`은 주입(테스트는 tempdir).
pub struct WorkLogStore {
    dir: PathBuf,
}

impl WorkLogStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `agent_id`를 경로 요소로 쓰기 전 안전성 검증(경로 조작 방지). 구분자/`..`/
    /// 빈 문자열을 거부한다(diary_store.rs와 동일 규칙).
    fn validate_id(agent_id: &str) -> Result<(), WorkLogStoreError> {
        if agent_id.is_empty()
            || agent_id.contains('/')
            || agent_id.contains('\\')
            || agent_id.contains("..")
        {
            return Err(WorkLogStoreError::InvalidId);
        }
        Ok(())
    }

    fn path_for(&self, agent_id: &str) -> PathBuf {
        self.dir.join(format!("{agent_id}.json"))
    }

    /// 한 캐릭터의 작업 로그 버퍼 전체를 스냅샷 저장한다. `items`가 비면 파일을
    /// 삭제한다(빈 스냅샷을 남기지 않음 — 소진된 캐릭터는 흔적 없이 정리).
    /// tmp 파일에 쓰고 rename해 원자적으로 교체한다.
    pub fn save(&self, agent_id: &str, items: &[WorkLogItem]) -> Result<(), WorkLogStoreError> {
        Self::validate_id(agent_id)?;
        let file = self.path_for(agent_id);

        if items.is_empty() {
            // 부재 = 빈 버퍼. 이미 없으면 무해하게 통과.
            match fs::remove_file(&file) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(WorkLogStoreError::Io(e.to_string())),
            }
        } else {
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent).map_err(|e| WorkLogStoreError::Io(e.to_string()))?;
            }
            let bytes =
                serde_json::to_vec(items).map_err(|e| WorkLogStoreError::Io(e.to_string()))?;
            // 같은 디렉터리 안 tmp에 쓰고 rename(원자적 교체). tmp 이름에 agentId를
            // 붙여 동시 저장이 서로 밟지 않게 한다.
            let tmp = self.dir.join(format!("{agent_id}.json.tmp"));
            {
                let mut f = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&tmp)
                    .map_err(|e| WorkLogStoreError::Io(e.to_string()))?;
                f.write_all(&bytes)
                    .map_err(|e| WorkLogStoreError::Io(e.to_string()))?;
                f.sync_all()
                    .map_err(|e| WorkLogStoreError::Io(e.to_string()))?;
            }
            // Windows는 dest 존재 시 rename이 실패하므로 먼저 지운다(있으면).
            #[cfg(windows)]
            {
                let _ = fs::remove_file(&file);
            }
            fs::rename(&tmp, &file).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                WorkLogStoreError::Io(e.to_string())
            })
        }
    }

    /// 디렉터리의 모든 스냅샷을 읽어 `agentId -> items` 맵으로 돌려준다. 부팅 복원용.
    /// 디렉터리 부재 = 빈 맵. 파싱 실패/비-`.json`/tmp 파일은 건너뛴다(부분 write 내성).
    pub fn load_all(&self) -> HashMap<String, Vec<WorkLogItem>> {
        let mut out = HashMap::new();
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `<agentId>.json`만. `.json.tmp`는 확장자가 `tmp`라 자연히 걸러진다.
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let agent_id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            match serde_json::from_slice::<Vec<WorkLogItem>>(&bytes) {
                Ok(items) if !items.is_empty() => {
                    out.insert(agent_id, items);
                }
                _ => continue, // 손상/빈 스냅샷은 스킵.
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-worklog-store-test-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn item(at: u64, session: &str, kind: &str, text: &str) -> WorkLogItem {
        WorkLogItem {
            at,
            session_id: session.into(),
            kind: kind.into(),
            text: text.into(),
            goal: None,
        }
    }

    #[test]
    fn save_then_load_all_roundtrips_items() {
        let dir = scratch_dir();
        let store = WorkLogStore::new(dir.clone());
        let items = vec![
            item(1, "s1", "prompt", "버그를 잡아라"),
            item(2, "s1", "tool", "grep 실행"),
        ];

        store.save("a1", &items).expect("save succeeds");
        let all = store.load_all();

        assert_eq!(all.get("a1"), Some(&items));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_overwrites_previous_snapshot() {
        let dir = scratch_dir();
        let store = WorkLogStore::new(dir.clone());
        store.save("a1", &[item(1, "s1", "prompt", "첫판")]).unwrap();
        let second = vec![item(2, "s2", "prompt", "둘째판")];
        store.save("a1", &second).unwrap();

        let all = store.load_all();
        assert_eq!(all.get("a1"), Some(&second));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_empty_removes_the_file() {
        let dir = scratch_dir();
        let store = WorkLogStore::new(dir.clone());
        store.save("a1", &[item(1, "s1", "prompt", "x")]).unwrap();
        assert!(dir.join("a1.json").exists());

        store.save("a1", &[]).unwrap();

        assert!(!dir.join("a1.json").exists());
        assert!(store.load_all().get("a1").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_empty_on_missing_file_is_ok() {
        let store = WorkLogStore::new(scratch_dir());
        store.save("nobody", &[]).expect("no-op on missing file");
    }

    #[test]
    fn items_are_isolated_per_agent() {
        let dir = scratch_dir();
        let store = WorkLogStore::new(dir.clone());
        store.save("a1", &[item(1, "s1", "prompt", "A")]).unwrap();
        store.save("a2", &[item(2, "s2", "prompt", "B")]).unwrap();

        let all = store.load_all();
        assert_eq!(all.get("a1"), Some(&vec![item(1, "s1", "prompt", "A")]));
        assert_eq!(all.get("a2"), Some(&vec![item(2, "s2", "prompt", "B")]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_all_on_missing_dir_returns_empty() {
        let store = WorkLogStore::new(scratch_dir());
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn load_all_skips_a_corrupt_snapshot_but_keeps_valid_ones() {
        let dir = scratch_dir();
        fs::create_dir_all(&dir).unwrap();
        let store = WorkLogStore::new(dir.clone());
        store.save("good", &[item(1, "s1", "prompt", "정상")]).unwrap();
        fs::write(dir.join("bad.json"), b"not json").unwrap();

        let all = store.load_all();
        assert_eq!(all.get("good"), Some(&vec![item(1, "s1", "prompt", "정상")]));
        assert!(all.get("bad").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_all_ignores_tmp_files() {
        let dir = scratch_dir();
        fs::create_dir_all(&dir).unwrap();
        let store = WorkLogStore::new(dir.clone());
        fs::write(dir.join("a1.json.tmp"), b"[]").unwrap();

        assert!(store.load_all().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goal_survives_roundtrip() {
        let dir = scratch_dir();
        let store = WorkLogStore::new(dir.clone());
        let with_goal = WorkLogItem {
            at: 1,
            session_id: "s1".into(),
            kind: "prompt".into(),
            text: "지시".into(),
            goal: Some("목표달성".into()),
        };
        store.save("a1", &[with_goal.clone()]).unwrap();

        assert_eq!(store.load_all().get("a1"), Some(&vec![with_goal]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_unsafe_agent_id() {
        let store = WorkLogStore::new(scratch_dir());
        assert!(matches!(
            store.save("../evil", &[item(1, "s1", "prompt", "x")]),
            Err(WorkLogStoreError::InvalidId)
        ));
        assert!(matches!(
            store.save("a/b", &[]),
            Err(WorkLogStoreError::InvalidId)
        ));
        assert!(matches!(
            store.save("", &[item(1, "s1", "prompt", "x")]),
            Err(WorkLogStoreError::InvalidId)
        ));
    }
}
