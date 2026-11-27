// src-tauri/src/webdash/work_progress.rs
//
// 캐릭터가 지금 맡아 진행 중인 업무와 그 플랜 진행도. 웹 상태판의 캐릭터 목록·상세·스플릿 칸이
// `/api/state`의 `work`로 받아 보여 준다. 플랜 단계 순서는 자동 진행(`run_steps`)과 같다.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::control::autorun::{run_steps, title};
use crate::types::{WorkItem, WorkStatus};

/// 끝나지 않은 자동 진행의 상태(`running`·`paused`). 없거나 끝났으면 None.
fn live_run(item: &WorkItem) -> Option<&Value> {
    let run = item.auto_run.as_ref()?;
    (run.get("state").and_then(Value::as_str)? != "finished").then_some(run)
}

/// `agent_id`가 진행 중(`in_progress`)인 업무. 자동 진행이 지금 보낸 단계를 먼저, 다음은 최근에 바뀐 것.
/// 플랜 단계면 그 플랜(루트 제목·완료 수·전체 단계·몇 번째 단계)과 자동 진행 상태를 함께 준다.
pub(crate) fn agent_work(items: &[WorkItem], agent_id: &str) -> Option<Value> {
    let current: HashSet<&str> =
        items.iter().filter_map(|i| live_run(i)?.get("currentId")?.as_str()).collect();
    let task = items
        .iter()
        .filter(|i| i.status == WorkStatus::InProgress && !i.plan && i.assignee_agent_id.as_deref() == Some(agent_id))
        .max_by_key(|i| (current.contains(i.id.as_str()), i.updated_at))?;
    // 플랜 루트: 끝나지 않았고 선행 작업이 있으며 다른 업무가 기다리지 않는 업무(업무 탭 `planLayout`과 같다).
    let waited_on: HashSet<&str> = items.iter().flat_map(|i| i.depends_on.iter().map(String::as_str)).collect();
    let mut roots: Vec<&WorkItem> = items
        .iter()
        .filter(|i| i.status != WorkStatus::Done && !i.depends_on.is_empty() && !waited_on.contains(i.id.as_str()))
        .collect();
    roots.sort_by(|a, b| a.order.partial_cmp(&b.order).unwrap_or(std::cmp::Ordering::Equal));
    let driving = |r: &WorkItem| live_run(r).and_then(|run| run.get("currentId")?.as_str()) == Some(task.id.as_str());
    let plan = roots
        .iter()
        .map(|r| (*r, run_steps(r, items)))
        .filter(|(_, steps)| steps.iter().any(|s| s.id == task.id))
        .max_by_key(|(r, _)| driving(r) as u8 * 2 + live_run(r).is_some() as u8)
        .map(|(root, steps)| {
            let run = live_run(root);
            json!({
                "id": root.id,
                "title": title(root),
                "done": steps.iter().filter(|s| s.status == WorkStatus::Done).count(),
                "total": steps.len(),
                "step": steps.iter().position(|s| s.id == task.id).map(|p| p + 1),
                "autoRun": run.and_then(|r| r.get("state")).cloned(),
            })
        });
    Some(json!({ "id": task.id, "title": title(task), "plan": plan }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, status: WorkStatus, order: f64, after: &[&str]) -> WorkItem {
        WorkItem {
            id: id.into(),
            title: format!("{id} title"),
            body: String::new(),
            assignee_agent_id: Some("a1".into()),
            agent: None,
            status,
            order,
            session_ids: vec![],
            last_error: None,
            depends_on: after.iter().map(|s| s.to_string()).collect(),
            plan: false,
            max_follow_ups: None,
            permission_wait_minutes: None,
            auto_run: None,
            schedule: None,
            scheduled_from: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn standalone_task_has_no_plan() {
        let items = vec![item("x", WorkStatus::InProgress, 0.0, &[]), item("y", WorkStatus::Todo, 1.0, &[])];
        let w = agent_work(&items, "a1").unwrap();
        assert_eq!(w["id"], "x");
        assert_eq!(w["title"], "x title");
        assert!(w["plan"].is_null());
        assert!(agent_work(&items, "other").is_none());
    }

    #[test]
    fn plan_step_reports_progress_and_auto_run() {
        let mut root = item("root", WorkStatus::Todo, 9.0, &["c"]);
        root.plan = true;
        root.auto_run = Some(json!({ "state": "running", "currentId": "b" }));
        let mut c = item("c", WorkStatus::InProgress, 3.0, &["b"]);
        c.updated_at = 99;
        let items = vec![
            item("a", WorkStatus::Done, 1.0, &[]),
            item("b", WorkStatus::InProgress, 2.0, &["a"]),
            c,
            root,
        ];
        // 자동 진행이 보낸 단계(b)가 더 최근에 바뀐 c보다 먼저다.
        let w = agent_work(&items, "a1").unwrap();
        assert_eq!(w["id"], "b");
        assert_eq!(w["plan"], json!({ "id": "root", "title": "root title", "done": 1, "total": 3, "step": 2, "autoRun": "running" }));
    }

    #[test]
    fn non_plan_root_counts_itself_as_last_step() {
        let mut items = vec![item("a", WorkStatus::Done, 1.0, &[]), item("r", WorkStatus::InProgress, 2.0, &["a"])];
        items[1].auto_run = Some(json!({ "state": "finished" }));
        let w = agent_work(&items, "a1").unwrap();
        assert_eq!(w["plan"]["step"], 2);
        assert_eq!(w["plan"]["total"], 2);
        assert_eq!(w["plan"]["done"], 1);
        assert!(w["plan"]["autoRun"].is_null());
    }
}
