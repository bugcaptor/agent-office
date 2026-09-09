use crate::automation::definition::validate;
use crate::types::{AutomationDefinition, AutomationRunRecord};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AutomationStore {
    root: PathBuf,
    /// Read/compare/write must be one critical section. Independent command
    /// clones share this lock, so equal revisions cannot both win.
    mutation_lock: Arc<Mutex<()>>,
}
impl AutomationStore {
    pub fn new(app_data: PathBuf) -> Self {
        let store = Self {
            root: app_data.join("automations"),
            mutation_lock: Arc::new(Mutex::new(())),
        };
        store.recover_interrupted();
        store
    }
    pub fn app_data_dir(&self) -> &Path {
        self.root
            .parent()
            .expect("automation store has app-data parent")
    }
    fn recover_interrupted(&self) {
        let _guard = self.mutation_lock.lock().unwrap();
        let root = self
            .root
            .parent()
            .unwrap_or(Path::new("."))
            .join("automation-runs");
        let Ok(entries) = fs::read_dir(&root) else {
            return;
        };
        for e in entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        {
            let p = e.path();
            let Ok(s) = fs::read_to_string(&p) else {
                continue;
            };
            let Ok(mut r) = serde_json::from_str::<AutomationRunRecord>(&s) else {
                continue;
            };
            if r.status == "running" {
                r.status = "interrupted".into();
                r.outcome = Some("interrupted".into());
                r.events.push(crate::types::AutomationRunEvent {
                    at: crate::types::now_ms(),
                    kind: "interrupted-after-restart".into(),
                    details: None,
                });
                let tmp = root.join(format!(".{}.{}.tmp", r.run_id, uuid::Uuid::new_v4()));
                if fs::write(&tmp, serde_json::to_vec_pretty(&r).unwrap_or_default()).is_ok() {
                    let _ = fs::rename(tmp, p);
                }
            }
        }
    }
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("automation-definition-id-invalid".into());
        };
        Ok(self.root.join(format!("{id}.json")))
    }
    pub fn list(&self) -> Result<Vec<AutomationDefinition>, String> {
        let mut out = Vec::new();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(error) => return Err(format!("automation-store-read-failed: {error}")),
        };
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                if let Ok(s) = fs::read_to_string(e.path()) {
                    if let Ok(d) = serde_json::from_str(&s) {
                        out.push(d)
                    }
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    pub fn get(&self, id: &str) -> Result<AutomationDefinition, String> {
        let p = self.path(id)?;
        let s = fs::read_to_string(p).map_err(|_| "automation-definition-not-found".to_string())?;
        serde_json::from_str(&s).map_err(|e| format!("automation-definition-invalid: {e}"))
    }
    pub fn save(&self, d: &AutomationDefinition) -> Result<AutomationDefinition, String> {
        let _guard = self.mutation_lock.lock().unwrap();
        validate(d)?;
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let p = self.path(&d.id)?;
        let mut saved = d.clone();
        if p.exists() {
            let old = self.get(&d.id)?;
            if old.revision != d.revision {
                return Err("automation-definition-revision-conflict".into());
            }
            saved.revision = old
                .revision
                .checked_add(1)
                .ok_or_else(|| "automation-definition-revision-exhausted".to_string())?;
        } else if saved.revision == 0 {
            saved.revision = 1;
        }
        let tmp = self
            .root
            .join(format!(".{}.{}.tmp", saved.id, uuid::Uuid::new_v4()));
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(tmp, p).map_err(|e| e.to_string())?;
        Ok(saved)
    }
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let _guard = self.mutation_lock.lock().unwrap();
        let p = self.path(id)?;
        match fs::remove_file(p) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn export(&self, id: &str) -> Result<String, String> {
        serde_json::to_string_pretty(&self.get(id)?).map_err(|e| e.to_string())
    }
    pub fn import(&self, json: &str) -> Result<AutomationDefinition, String> {
        let mut d: AutomationDefinition =
            serde_json::from_str(json).map_err(|e| format!("automation-import-invalid: {e}"))?;
        // Import is a copy operation, never an overwrite of a local definition.
        d.id = uuid::Uuid::new_v4().to_string();
        d.revision = 0;
        self.save(&d)
    }
    pub fn append_run(&self, record: &AutomationRunRecord) -> Result<(), String> {
        let _guard = self.mutation_lock.lock().unwrap();
        let root = self
            .root
            .parent()
            .unwrap_or(Path::new("."))
            .join("automation-runs");
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let p = root.join(format!("{}.json", record.run_id));
        let tmp = root.join(format!(".{}.{}.tmp", record.run_id, uuid::Uuid::new_v4()));
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(tmp, p).map_err(|e| e.to_string())
    }
    pub fn runs(&self) -> Result<Vec<AutomationRunRecord>, String> {
        let root = self
            .root
            .parent()
            .unwrap_or(Path::new("."))
            .join("automation-runs");
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(format!("automation-history-read-failed: {error}")),
        };
        let mut v: Vec<AutomationRunRecord> = Vec::new();
        for e in entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        {
            if let Ok(s) = fs::read_to_string(e.path()) {
                if let Ok(r) = serde_json::from_str(&s) {
                    v.push(r)
                }
            }
        }
        v.sort_by(|a, b| {
            b.events
                .first()
                .map(|event| event.at)
                .cmp(&a.events.first().map(|event| event.at))
                .then_with(|| b.run_id.cmp(&a.run_id))
        });
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AutomationDefinition, AutomationRunEvent, AutomationRunRecord, AutomationStep,
    };
    use std::collections::BTreeMap;

    fn definition(id: &str, revision: u32) -> AutomationDefinition {
        AutomationDefinition {
            schema_version: 1,
            id: id.into(),
            revision,
            name: "test".into(),
            inputs: vec![],
            steps: vec![AutomationStep::Wait {
                id: "wait".into(),
                duration_ms: 1,
            }],
            repeat: None,
            legacy_workspace: None,
            input_values: None,
        }
    }

    fn v2_definition(id: &str) -> AutomationDefinition {
        AutomationDefinition {
            schema_version: 2,
            id: id.into(),
            revision: 0,
            name: "v2 test".into(),
            inputs: vec![],
            steps: vec![
                AutomationStep::LaunchCli {
                    id: "launch".into(),
                    cli_profile_id: "codex".into(),
                    model: None,
                    effort: None,
                    startup_wait_ms: Some(1),
                },
                AutomationStep::LlmTask {
                    id: "task".into(),
                    label: "task".into(),
                    prompt_template: "{{previousResult}}".into(),
                    wait_timeout_ms: Some(1),
                    completion_grace_ms: Some(1),
                    allow_early_complete: Some(false),
                },
                AutomationStep::ExitCli {
                    id: "exit".into(),
                    command: "/exit".into(),
                    return_mode: Some(crate::types::AutomationCliReturnMode::Auto),
                    exit_wait_ms: Some(1),
                },
            ],
            repeat: None,
            legacy_workspace: None,
            input_values: None,
        }
    }
    fn run(id: &str, status: &str) -> AutomationRunRecord {
        AutomationRunRecord {
            run_id: id.into(),
            definition_snapshot: definition("definition", 1),
            inputs: BTreeMap::new(),
            workspace: "/tmp".into(),
            agent_id: "agent".into(),
            status: status.into(),
            outcome: None,
            events: vec![AutomationRunEvent {
                at: 1,
                kind: "started".into(),
                details: None,
            }],
        }
    }

    #[test]
    fn concurrent_equal_revision_has_one_winner() {
        let temp = tempfile::tempdir().unwrap();
        let store = AutomationStore::new(temp.path().to_path_buf());
        let saved = store.save(&definition("same", 0)).unwrap();
        let a = store.clone();
        let b = store.clone();
        let left = std::thread::spawn(move || a.save(&definition("same", saved.revision)));
        let right = std::thread::spawn(move || b.save(&definition("same", saved.revision)));
        let outcomes = [left.join().unwrap(), right.join().unwrap()];
        assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|r| matches!(r, Err(error) if error == "automation-definition-revision-conflict"))
                .count(),
            1
        );
    }

    #[test]
    fn recent_runs_are_sorted_by_start_time_not_directory_order() {
        let temp = tempfile::tempdir().unwrap();
        let store = AutomationStore::new(temp.path().to_path_buf());
        let mut newer = run("a-newer", "completed");
        newer.events[0].at = 20;
        store.append_run(&newer).unwrap();
        store.append_run(&run("z-older", "completed")).unwrap();
        let records = store.runs().unwrap();
        assert_eq!(records[0].run_id, "a-newer");
        assert_eq!(records[1].run_id, "z-older");
    }

    /// 옛 정의 파일에 남아 있던 `workspace`는 읽을 때는 값을 채우지만
    /// (`legacy_workspace`), 그 정의를 한 번 다시 저장하면 파일에서 사라진다.
    /// 작업 폴더는 이제 실행 시점에 탭에서 읽으므로 저장은 하지 않는다.
    #[test]
    fn legacy_workspace_is_read_but_dropped_on_next_save() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("automations")).unwrap();
        let mut definition = definition("persisted", 0);
        definition.input_values = Some(BTreeMap::from([(
            "task".into(),
            "persist this value".into(),
        )]));
        let mut raw = serde_json::to_value(&definition).unwrap();
        raw["workspace"] = serde_json::json!("/work/saved");
        std::fs::write(
            temp.path().join("automations/persisted.json"),
            serde_json::to_vec_pretty(&raw).unwrap(),
        )
        .unwrap();

        let store = AutomationStore::new(temp.path().to_path_buf());
        let restored = store
            .list()
            .unwrap()
            .into_iter()
            .find(|definition| definition.id == "persisted")
            .expect("legacy definition file is listed");
        assert_eq!(restored.legacy_workspace.as_deref(), Some("/work/saved"));
        assert_eq!(
            restored
                .input_values
                .as_ref()
                .and_then(|values| values.get("task")),
            Some(&"persist this value".to_string())
        );

        store.save(&restored).unwrap();
        let resaved = AutomationStore::new(temp.path().to_path_buf())
            .list()
            .unwrap()
            .into_iter()
            .find(|definition| definition.id == "persisted")
            .expect("resaved definition is still listed");
        assert_eq!(resaved.legacy_workspace, None);
    }

    #[test]
    fn v1_and_v2_definitions_survive_a_fresh_store_without_schema_coercion() {
        let temp = tempfile::tempdir().unwrap();
        let store = AutomationStore::new(temp.path().to_path_buf());
        store.save(&definition("legacy-v1", 0)).unwrap();
        store.save(&v2_definition("paired-v2")).unwrap();

        let restored = AutomationStore::new(temp.path().to_path_buf())
            .list()
            .unwrap();
        let versions: BTreeMap<_, _> = restored
            .iter()
            .map(|definition| (definition.id.as_str(), definition.schema_version))
            .collect();
        assert_eq!(versions.get("legacy-v1"), Some(&1));
        assert_eq!(versions.get("paired-v2"), Some(&2));
        assert!(matches!(
            restored
                .iter()
                .find(|definition| definition.id == "paired-v2")
                .unwrap()
                .steps
                .last(),
            Some(AutomationStep::ExitCli {
                return_mode: Some(crate::types::AutomationCliReturnMode::Auto),
                exit_wait_ms: Some(1),
                ..
            })
        ));
    }

    #[test]
    fn import_always_copies_and_recovery_ignores_tmp() {
        let temp = tempfile::tempdir().unwrap();
        let store = AutomationStore::new(temp.path().to_path_buf());
        let original = store.save(&definition("original", 0)).unwrap();
        let imported = store
            .import(&serde_json::to_string(&original).unwrap())
            .unwrap();
        assert_ne!(imported.id, original.id);
        assert_eq!(imported.revision, 1);
        store.append_run(&run("live", "running")).unwrap();
        let runs = temp.path().join("automation-runs");
        fs::write(
            runs.join(".stale.tmp"),
            serde_json::to_vec(&run("stale", "running")).unwrap(),
        )
        .unwrap();
        let restarted = AutomationStore::new(temp.path().to_path_buf());
        let records = restarted.runs().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "interrupted");
        assert_eq!(
            records[0].events.last().unwrap().kind,
            "interrupted-after-restart"
        );
    }
}
