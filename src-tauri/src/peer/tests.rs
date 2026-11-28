use super::*;

fn party(m: &str, a: &str) -> Party {
    Party { machine_id: m.into(), machine_name: format!("{m}-name"), agent_id: Some(a.into()), agent_name: Some(a.into()) }
}

fn thread(id: &str, from: Party, to: Party, at: u64, outgoing: bool, incoming: bool) -> Thread {
    Thread { id: id.into(), from, to, text: "q".into(), reply: None, created_at: at, replied_at: None, depth: 1, outgoing, incoming }
}

fn state() -> Arc<PeerState> {
    PeerState::new(None, "m1".into())
}

#[test]
fn asking_back_someone_who_waits_for_you_is_refused() {
    let p = state();
    let now = now_ms();
    // m2의 b가 m1의 a에게 물었다(a는 아직 답하지 않음).
    p.upsert(thread("p-in", party("m2", "b"), party("m1", "a"), now, false, true));
    assert_eq!(p.check_limits("a", &party("m2", "b"), now), Err("peer-reply-first: p-in".into()));
    // 다른 상대에게는 물을 수 있고, 깊이는 하나 늘어난다.
    assert_eq!(p.check_limits("a", &party("m3", "c"), now), Ok(2));
    // 답하면 풀린다.
    p.update("p-in", |t| t.reply = Some("ok".into()));
    assert_eq!(p.check_limits("a", &party("m2", "b"), now), Ok(1));
}

#[test]
fn depth_and_rate_limits() {
    let p = state();
    let now = now_ms();
    let mut deep = thread("p-deep", party("m2", "b"), party("m1", "a"), now, false, true);
    deep.depth = MAX_DEPTH;
    p.upsert(deep);
    assert_eq!(p.check_limits("a", &party("m3", "c"), now), Err(format!("peer-depth-limit: {MAX_DEPTH}")));
    // 오래된 받은 대화는 깊이에 넣지 않는다.
    assert_eq!(p.check_limits("a", &party("m3", "c"), now + DEPTH_WINDOW_MS + 1), Ok(1));

    let p = state();
    for i in 0..MAX_PAIR_PER_HOUR {
        p.upsert(thread(&format!("p-{i}"), party("m1", "a"), party("m2", "b"), now, true, false));
    }
    assert!(p.check_limits("a", &party("m2", "b"), now).unwrap_err().starts_with("peer-pair-limit"));
    assert_eq!(p.check_limits("a", &party("m2", "c"), now), Ok(1));
    // 한 시간이 지나면 다시 보낸다.
    assert_eq!(p.check_limits("a", &party("m2", "b"), now + HOUR_MS), Ok(1));
    for i in MAX_PAIR_PER_HOUR..MAX_SENDS_PER_HOUR {
        p.upsert(thread(&format!("p-{i}"), party("m1", "a"), party("m2", &format!("x{i}")), now, true, false));
    }
    assert!(p.check_limits("a", &party("m9", "z"), now).unwrap_err().starts_with("peer-rate-limit"));
    // 다른 에이전트는 따로 센다.
    assert_eq!(p.check_limits("other", &party("m9", "z"), now), Ok(1));
}

#[test]
fn same_machine_threads_merge_roles() {
    let p = state();
    let now = now_ms();
    p.upsert(thread("p-1", party("m1", "a"), party("m1", "b"), now, true, false));
    p.upsert(thread("p-1", party("m1", "a"), party("m1", "b"), now, false, true));
    let list = p.threads();
    assert_eq!(list.len(), 1);
    assert!(list[0].outgoing && list[0].incoming);
    // 받는 쪽만 지워도 보낸 쪽 기록은 남는다.
    p.remove_role("p-1", true);
    assert_eq!(p.threads().len(), 1);
    p.remove_role("p-1", false);
    assert!(p.threads().is_empty());
}

#[test]
fn replies_are_checked_against_the_thread() {
    let p = state();
    let now = now_ms();
    p.upsert(thread("p-1", party("m1", "a"), party("m2", "b"), now, true, false));
    let reply = |from: Party| json!({ "id": "p-1", "text": "done", "from": from });
    assert_eq!(p.receive_reply(&reply(party("m3", "b"))), Err("peer-thread-not-yours".into()));
    assert_eq!(p.receive_reply(&reply(party("m2", "c"))), Err("peer-thread-not-yours".into()));
    assert_eq!(p.receive_reply(&json!({ "id": "p-9", "text": "x", "from": party("m2", "b") })), Err("peer-thread-unknown".into()));
    assert_eq!(p.receive_reply(&reply(party("m2", "b"))), Ok(json!({})));
    assert_eq!(p.threads()[0].reply.as_deref(), Some("done"));
    assert_eq!(p.receive_reply(&reply(party("m2", "b"))), Err("peer-already-replied".into()));
}

#[test]
fn messages_name_the_sender_and_the_reply_command() {
    let t = thread("p-1", party("m1", "hana"), party("m2", "dori"), 0, true, false);
    let text = arrival_message(&t, std::path::Path::new("/tmp/bin/office-peer"));
    assert!(text.contains("message p-1 from hana@m1-name"));
    assert!(text.contains("/tmp/bin/office-peer reply p-1"));
    assert!(late_reply_message(&t, "ok").contains("from dori@m2-name]\nok"));
}

#[test]
fn summary_keeps_what_agents_need() {
    let c = json!({
        "agentId": "a", "name": "hana", "role": "dev", "cwd": "/w", "clockedOut": false,
        "session": { "sessionId": "s", "state": "running" },
        "turn": { "phase": "working", "since": 5, "message": "Bash: rm" },
        "cli": "claude", "atPrompt": false, "asks": [{}],
        "queue": { "items": [{ "id": "q1", "text": "first\nsecond" }] },
        "work": {}
    });
    let s = summary(&c, false);
    assert_eq!(s["phase"], "working");
    assert_eq!(s["session"], "running");
    assert_eq!(s["asks"], 1);
    assert_eq!(s["queued"], 1);
    assert!(s.get("queue").is_none());
    let d = summary(&c, true);
    assert_eq!(d["queue"][0], json!({ "id": "q1", "text": "first" }));
    assert_eq!(d["waitingFor"], "Bash: rm");
}
