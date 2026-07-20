    use super::fake::FakeClock;
    use super::{dedup_key, Clock, NotificationHub};
    use crate::observer::event::ObserverEvent;
    use crate::state::fake::RecordingEvents;
    use crate::state::SessionRegistry;
    use crate::types::*;
    use std::sync::Arc;
    use std::time::Duration;

    /// Standard fixture: session "s1" mapped to agent "a1", registered as
    /// Running, wired to a fresh FakeClock + RecordingEvents + 3s dedup
    /// window.
    fn fixture() -> (Arc<NotificationHub>, Arc<RecordingEvents>, Arc<FakeClock>) {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = Arc::new(NotificationHub::new(
            registry,
            events.clone(),
            clock.clone(),
            Duration::from_millis(3000),
        ));
        (hub, events, clock)
    }

    #[test]
    fn observer_events_project_to_existing_public_contracts() {
        let (hub, events, _clock) = fixture();
        hub.ingest_observer(
            "s1",
            ObserverEvent::Prompt {
                text: Some("버그 수정".into()),
                cwd: None,
            },
        );
        hub.ingest_observer(
            "s1",
            ObserverEvent::Tool {
                text: None,
                assistant: None,
            },
        );
        hub.ingest_observer("s1", ObserverEvent::Attention { message: None });
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: None,
                running: None,
            },
        );

        let activity = events.activities();
        assert_eq!(activity[0].kind, ActivityKind::Prompt);
        assert_eq!(activity[0].text.as_deref(), Some("버그 수정"));
        assert_eq!(activity[1].kind, ActivityKind::Tool);
        assert_eq!(activity[2].kind, ActivityKind::SubCount);
        assert_eq!(activity[2].count, Some(0));

        let notifications = events.notifications();
        assert_eq!(notifications[0].source, NotificationSource::Hook);
        assert_eq!(notifications[0].message, "확인이 필요합니다");
        assert_eq!(notifications[1].source, NotificationSource::Stop);
        assert_eq!(notifications[1].message, "작업이 완료되었습니다.");
    }

    #[test]
    fn tool_observer_event_carries_text_and_assistant_into_activity() {
        // 이슈 #43: ObserverEvent::Tool의 text/assistant가 ActivityEvent로 실려야 한다.
        let (hub, events, _clock) = fixture();
        hub.ingest_observer(
            "s1",
            ObserverEvent::Tool {
                text: Some("Bash: npm test".into()),
                assistant: Some("파일을 살펴보는 중".into()),
            },
        );
        let activity = events.activities();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, ActivityKind::Tool);
        assert_eq!(activity[0].text.as_deref(), Some("Bash: npm test"));
        assert_eq!(activity[0].assistant_text.as_deref(), Some("파일을 살펴보는 중"));
        // 활동 신호이므로 알림 파이프라인은 오염되지 않는다.
        assert!(events.notifications().is_empty());
    }

    #[test]
    fn prompt_observer_event_carries_cwd_into_activity() {
        // 이슈 #44 작업 D: ObserverEvent::Prompt의 cwd가 ActivityEvent로 실려야 한다.
        let (hub, events, _clock) = fixture();
        hub.ingest_observer(
            "s1",
            ObserverEvent::Prompt {
                text: Some("버그 고쳐줘".into()),
                cwd: Some("/w/project".into()),
            },
        );
        let activity = events.activities();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, ActivityKind::Prompt);
        assert_eq!(activity[0].text.as_deref(), Some("버그 고쳐줘"));
        assert_eq!(activity[0].cwd.as_deref(), Some("/w/project"));
        assert!(events.notifications().is_empty());
    }

    #[test]
    fn sub_count_emits_activity_without_notification() {
        let (hub, events, _clock) = fixture();
        hub.ingest_observer("s1", ObserverEvent::SubCount { running: 3 });

        let activity = events.activities();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, ActivityKind::SubCount);
        assert_eq!(activity[0].count, Some(3));
        assert!(events.notifications().is_empty());
    }

    #[test]
    fn stop_emits_absolute_count_before_notification_and_defaults_to_zero() {
        let (hub, events, _clock) = fixture();
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: Some("first".into()),
                running: Some(2),
            },
        );
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: Some("second".into()),
                running: None,
            },
        );

        let activity = events.activities();
        assert_eq!(activity.len(), 2);
        assert_eq!(activity[0].kind, ActivityKind::SubCount);
        assert_eq!(activity[0].count, Some(2));
        assert_eq!(activity[1].kind, ActivityKind::SubCount);
        assert_eq!(activity[1].count, Some(0));
        // running=2인 첫 Stop은 완료가 아니므로 알림이 억제되고, running 부재
        // (=0 간주)인 두 번째 Stop만 알림된다.
        let notifications = events.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].message, "second");
    }

    #[test]
    fn stop_with_running_subagents_suppresses_completion_notification() {
        let (hub, events, _clock) = fixture();
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: Some("아직 서브에이전트 진행 중".into()),
                running: Some(1),
            },
        );

        // 카운트 스냅샷은 그대로 흘려보내되 완료 알림은 내지 않는다(#27).
        let activity = events.activities();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, ActivityKind::SubCount);
        assert_eq!(activity[0].count, Some(1));
        assert!(events.notifications().is_empty());

        // 최종 Stop(running=0)은 평소대로 알림된다.
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: None,
                running: Some(0),
            },
        );
        let notifications = events.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].source, NotificationSource::Stop);
        assert_eq!(notifications[0].message, "작업이 완료되었습니다.");
    }

    #[test]
    fn stop_snapshot_preserves_two_running_subagents_after_delta_events() {
        let (hub, events, _clock) = fixture();
        hub.ingest_observer("s1", ObserverEvent::SubStart);
        hub.ingest_observer("s1", ObserverEvent::SubStart);
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: None,
                running: Some(2),
            },
        );

        let activity = events.activities();
        assert_eq!(activity.len(), 3);
        assert_eq!(activity[0].kind, ActivityKind::SubStart);
        assert_eq!(activity[1].kind, ActivityKind::SubStart);
        assert_eq!(activity[2].kind, ActivityKind::SubCount);
        assert_eq!(activity[2].count, Some(2));
    }

    fn msg(text: &str) -> Vec<u8> {
        serde_json::json!({ "message": text })
            .to_string()
            .into_bytes()
    }

    fn is_uuid_v4(s: &str) -> bool {
        // 8-4-4-4-12 hex groups, version nibble '4' at the start of the 3rd group.
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
            && [8, 4, 4, 4, 12]
                .iter()
                .zip(&parts)
                .all(|(len, p)| p.len() == *len)
            && parts[2].starts_with('4')
            && s.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
    }

    // ---- T-C: dedup 3s window (FakeClock) ----

    #[test]
    fn dedup_suppresses_within_window_then_passes_once_window_elapses() {
        let (hub, events, clock) = fixture();

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=0, passes
        clock.advance(1000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=1000, suppressed (slides window to 1000)
        clock.advance(4000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=5000, 5000-1000=4000 >= 3000, passes

        assert_eq!(events.notifications().len(), 2);
        let pending = hub.pending("s1");
        assert_eq!(pending.len(), 2);

        // Pin real emitted payload values, not just counts.
        assert_eq!(pending[0].session_id, "s1");
        assert_eq!(pending[0].agent_id, "a1");
        assert_eq!(pending[0].source, NotificationSource::Hook);
        assert_eq!(pending[0].message, "need input");
        assert_eq!(
            pending[0].dedup_key,
            dedup_key("s1", NotificationSource::Hook, "need input")
        );
        assert_eq!(pending[0].at, 1_700_000_000_000); // clock.now_ms() at t=0

        // Second passing notification reflects the *slid* window, i.e. it
        // was emitted at t=5000, not t=0 or t=1000.
        assert_eq!(pending[1].at, 1_700_000_005_000);
        assert_ne!(
            pending[0].id, pending[1].id,
            "each passing notification gets a fresh id"
        );
        assert!(is_uuid_v4(&pending[0].id) && is_uuid_v4(&pending[1].id));
    }

    #[test]
    fn dedup_window_slide_means_suppressed_hit_resets_the_clock() {
        // If suppression did NOT slide last_seen, a hit at t=1000 followed by
        // one at t=3500 (3500ms after t=0) would incorrectly pass. Pin that
        // it stays suppressed because the window slid to t=1000 on the
        // t=1000 hit, so t=3500 is only 2500ms after the new baseline.
        let (hub, events, clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=0, passes
        clock.advance(1000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=1000, suppressed, slides to 1000
        clock.advance(2500);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=3500, 3500-1000=2500 < 3000, still suppressed

        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
    }

    #[test]
    fn dedup_key_is_per_session_so_other_sessions_are_unaffected() {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        registry.insert("s2", "a2", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub =
            NotificationHub::new(registry, events.clone(), clock, Duration::from_millis(3000));

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input"));
        hub.ingest_hook("s2", NotificationSource::Hook, &msg("need input")); // same message, different session: not suppressed

        assert_eq!(events.notifications().len(), 2);
        assert_eq!(hub.pending("s1").len(), 1);
        assert_eq!(hub.pending("s2").len(), 1);
    }

    #[test]
    fn different_messages_on_same_session_are_not_deduped() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input"));
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("different message")); // distinct message, separate entry

        assert_eq!(events.notifications().len(), 2);
        assert_eq!(hub.pending("s1").len(), 2);
    }

    // ---- T-D: clear partial vs full ----

    #[test]
    fn clear_with_specific_ids_removes_only_those_and_emits_cleared() {
        let (hub, events, _clock) = fixture();
        for m in ["m1", "m2", "m3"] {
            hub.ingest_hook("s1", NotificationSource::Hook, &msg(m));
        }
        let ids: Vec<String> = hub.pending("s1").iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids.len(), 3);

        let cleared = hub.clear("s1", Some(vec![ids[1].clone()]));
        assert_eq!(cleared, vec![ids[1].clone()]);

        let remaining: Vec<String> = hub.pending("s1").iter().map(|e| e.id.clone()).collect();
        assert_eq!(remaining, vec![ids[0].clone(), ids[2].clone()]);

        let cleared_events = events.cleared();
        assert_eq!(cleared_events.len(), 1);
        assert_eq!(cleared_events[0], ("a1".to_string(), vec![ids[1].clone()]));
    }

    #[test]
    fn clear_with_none_clears_entire_session_and_emits_all_ids() {
        let (hub, events, _clock) = fixture();
        for m in ["m1", "m2", "m3"] {
            hub.ingest_hook("s1", NotificationSource::Hook, &msg(m));
        }
        let ids: Vec<String> = hub.pending("s1").iter().map(|e| e.id.clone()).collect();

        let cleared = hub.clear("s1", None);
        assert_eq!(cleared.len(), 3);
        assert_eq!(
            cleared.iter().collect::<std::collections::HashSet<_>>(),
            ids.iter().collect::<std::collections::HashSet<_>>()
        );
        assert!(hub.pending("s1").is_empty());

        let cleared_events = events.cleared();
        assert_eq!(cleared_events.len(), 1);
        assert_eq!(cleared_events[0].0, "a1");
        assert_eq!(
            cleared_events[0]
                .1
                .iter()
                .collect::<std::collections::HashSet<_>>(),
            ids.iter().collect::<std::collections::HashSet<_>>()
        );
    }

    #[test]
    fn clear_with_some_empty_vec_behaves_like_clear_all() {
        // The match arm `Some(ids) if !ids.is_empty()` only takes the
        // partial-clear path for a *non-empty* id list; `Some(vec![])` falls
        // through to the `_` (clear-all) arm. Pin this exact, easy-to-break
        // boundary condition rather than assuming it.
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m1"));
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m2"));

        let cleared = hub.clear("s1", Some(vec![]));
        assert_eq!(cleared.len(), 2);
        assert!(hub.pending("s1").is_empty());
        assert_eq!(events.cleared().len(), 1);
    }

    #[test]
    fn clear_on_session_with_no_pending_returns_empty_and_emits_nothing() {
        let (hub, events, _clock) = fixture();
        // No ingest happened for "s1" at all.
        assert_eq!(hub.clear("s1", None), Vec::<String>::new());
        assert!(events.cleared().is_empty());
    }

    #[test]
    fn clear_with_unknown_ids_matches_nothing_and_leaves_queue_intact() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m1"));

        let cleared = hub.clear("s1", Some(vec!["not-a-real-id".to_string()]));
        assert!(cleared.is_empty());
        assert_eq!(hub.pending("s1").len(), 1); // untouched
        assert!(events.cleared().is_empty()); // no emit when nothing actually cleared
    }

    // ---- on_bell ----

    #[test]
    fn on_bell_ingests_bell_source_with_fixed_message() {
        let (hub, events, _clock) = fixture();
        hub.on_bell("s1");

        let pending = hub.pending("s1");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source, NotificationSource::Bell);
        assert_eq!(pending[0].message, "Terminal bell");
        assert_eq!(events.notifications().len(), 1);
    }

    #[test]
    fn on_bell_shares_the_same_dedup_pipeline_as_hook_ingest() {
        let (hub, events, clock) = fixture();
        hub.on_bell("s1"); // passes
        clock.advance(500);
        hub.on_bell("s1"); // within 3s window, suppressed

        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
    }

    // ---- purge_session ----

    #[test]
    fn purge_session_drops_pending_without_emitting_cleared() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m1"));
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m2"));
        assert_eq!(hub.pending("s1").len(), 2);

        hub.purge_session("s1");

        assert!(hub.pending("s1").is_empty());
        // Unlike clear(), purge_session (used on exit/dispose for
        // unresolved notifications) must NOT emit
        // notification-cleared — the session/agent may already be torn down.
        assert!(events.cleared().is_empty());
    }

    #[test]
    fn purge_session_on_unknown_session_is_a_noop() {
        let (hub, events, _clock) = fixture();
        hub.purge_session("does-not-exist"); // must not panic
        assert!(events.cleared().is_empty());
    }

    // ---- hook for unknown/dead session is discarded ----

    #[test]
    fn ingest_hook_for_unregistered_session_is_discarded() {
        let registry = Arc::new(SessionRegistry::new()); // "s1" never inserted
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub =
            NotificationHub::new(registry, events.clone(), clock, Duration::from_millis(3000));

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input"));

        assert!(hub.pending("s1").is_empty());
        assert!(events.notifications().is_empty());
    }

    #[test]
    fn ingest_hook_after_session_removed_from_registry_is_discarded() {
        // Simulates the actual trigger: Disposed sessions are removed from
        // the registry (see session/manager.rs on_exit), so a hook that
        // arrives afterward must resolve to nothing and be dropped.
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = NotificationHub::new(
            registry.clone(),
            events.clone(),
            clock,
            Duration::from_millis(3000),
        );

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("first")); // registered, passes
        registry.remove("s1");
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("second")); // now unresolvable, discarded

        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
        assert_eq!(hub.pending("s1")[0].message, "first");
    }

    #[test]
    fn on_bell_for_unregistered_session_is_discarded() {
        let registry = Arc::new(SessionRegistry::new());
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub =
            NotificationHub::new(registry, events.clone(), clock, Duration::from_millis(3000));

        hub.on_bell("unknown-session");

        assert!(events.notifications().is_empty());
    }

    // ---- on_output: Stop-후 출력 복귀 휴리스틱(이슈 #39) ----

    fn ingest_final_stop(hub: &NotificationHub) {
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: None,
                running: Some(0),
            },
        );
    }

    #[test]
    fn on_output_after_grace_over_threshold_clears_stop_and_emits_resume() {
        let (hub, events, clock) = fixture();
        ingest_final_stop(&hub);
        assert_eq!(hub.pending("s1").len(), 1);

        // grace(3s) 이내 출력은 프롬프트 리드로우로 보고 무시한다.
        clock.advance(1000);
        hub.on_output("s1", 100_000);
        assert!(events.cleared().is_empty());
        assert!(events
            .activities()
            .iter()
            .all(|a| a.kind != ActivityKind::Resume));
        assert_eq!(hub.pending("s1").len(), 1);

        // grace 경과 후 임계치 초과 → 복귀 발화.
        clock.advance(3000); // elapsed = 4s > grace
        hub.on_output("s1", 9000); // > 8KB

        assert!(hub.pending("s1").is_empty(), "stop 알림이 걷혀야 한다");
        let cleared = events.cleared();
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0].0, "a1");
        assert!(events
            .activities()
            .iter()
            .any(|a| a.kind == ActivityKind::Resume && a.agent_id == "a1"));

        // 재발화 방지: 이후 출력엔 더 반응하지 않는다.
        let cleared_before = events.cleared().len();
        let acts_before = events.activities().len();
        hub.on_output("s1", 100_000);
        assert_eq!(events.cleared().len(), cleared_before);
        assert_eq!(events.activities().len(), acts_before);
    }

    #[test]
    fn on_output_under_threshold_does_not_resume() {
        let (hub, events, clock) = fixture();
        ingest_final_stop(&hub);
        clock.advance(4000); // grace 경과
        hub.on_output("s1", 1000);
        hub.on_output("s1", 2000); // 누적 3000 < 8192

        assert_eq!(hub.pending("s1").len(), 1);
        assert!(events.cleared().is_empty());
        assert!(events
            .activities()
            .iter()
            .all(|a| a.kind != ActivityKind::Resume));
    }

    #[test]
    fn on_output_after_window_stops_watching() {
        let (hub, events, clock) = fixture();
        ingest_final_stop(&hub);
        clock.advance(31_000); // 30s 감시창 경과
        hub.on_output("s1", 100_000);

        assert_eq!(hub.pending("s1").len(), 1);
        assert!(events.cleared().is_empty());
        assert!(events
            .activities()
            .iter()
            .all(|a| a.kind != ActivityKind::Resume));
    }

    #[test]
    fn on_output_without_prior_stop_is_a_noop() {
        let (hub, events, clock) = fixture();
        clock.advance(5000);
        hub.on_output("s1", 100_000);
        assert!(events.activities().is_empty());
        assert!(events.cleared().is_empty());
    }

    // ---- 오토모드 질문 알림 홀드(이슈 #41) ----

    /// fixture + hold_duration 주입. 기본 fixture 는 hold=0(즉시 방출)이므로
    /// 홀드 동작 테스트는 이 헬퍼로 hold 를 켠다.
    fn hold_fixture(ms: u64) -> (Arc<NotificationHub>, Arc<RecordingEvents>, Arc<FakeClock>) {
        let (hub, events, clock) = fixture();
        hub.set_hold_duration(Duration::from_millis(ms));
        (hub, events, clock)
    }

    #[test]
    fn hold_defers_hook_until_flush_and_records_last_seen_on_emit() {
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // held at t=0

        // 보류 중 — 방출·큐 없음(pending 은 held 를 포함하지 않는다).
        assert!(events.notifications().is_empty());
        assert!(hub.pending("s1").is_empty());

        // 만료 전(t=4000) flush 는 방출하지 않는다.
        clock.advance(4000);
        hub.flush_expired();
        assert!(events.notifications().is_empty());
        assert!(hub.pending("s1").is_empty());

        // 5s 경과(t=5000) flush → 방출.
        clock.advance(1000);
        hub.flush_expired();
        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
        assert_eq!(hub.pending("s1")[0].message, "need input");

        // 방출 시점(t=5000)에 last_seen 이 남으므로 같은 메시지 재ingest 는 dedup 억제.
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input"));
        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
    }

    #[test]
    fn hold_cancelled_by_tool_activity_and_requestion_re_holds() {
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // held at t=0
        hub.ingest_observer(
            "s1",
            ObserverEvent::Tool {
                text: None,
                assistant: None,
            },
        ); // 계속 일하는 신호 → 폐기
        assert!(hub.pending("s1").is_empty());

        clock.advance(6000);
        hub.flush_expired();
        assert!(
            events.notifications().is_empty(),
            "폐기된 홀드는 만료돼도 방출되지 않는다"
        );

        // last_seen 미기록 검증: 같은 메시지를 다시 보내면 억제되지 않고 새로 held 로 들어간다.
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // held at t=6000
        assert!(hub.pending("s1").is_empty(), "즉시 방출이 아니라 다시 보류");
        clock.advance(5000);
        hub.flush_expired();
        assert_eq!(events.notifications().len(), 1);
    }

    #[test]
    fn hold_cancelled_by_prompt_and_substart_but_not_substop_or_subcount() {
        // Prompt 취소
        let (hub, _events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        hub.ingest_observer("s1", ObserverEvent::Prompt { text: None, cwd: None });
        clock.advance(6000);
        hub.flush_expired();
        assert!(hub.pending("s1").is_empty());

        // SubStart 취소
        let (hub, _events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        hub.ingest_observer("s1", ObserverEvent::SubStart);
        clock.advance(6000);
        hub.flush_expired();
        assert!(hub.pending("s1").is_empty());

        // SubStop 은 취소하지 않는다.
        let (hub, _events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        hub.ingest_observer("s1", ObserverEvent::SubStop);
        clock.advance(6000);
        hub.flush_expired();
        assert_eq!(hub.pending("s1").len(), 1);

        // SubCount 는 취소하지 않는다.
        let (hub, _events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        hub.ingest_observer("s1", ObserverEvent::SubCount { running: 2 });
        clock.advance(6000);
        hub.flush_expired();
        assert_eq!(hub.pending("s1").len(), 1);
    }

    #[test]
    fn hold_discarded_by_stop_observer_regardless_of_running() {
        // running=0: 질문은 폐기되고 Stop 완료 알림만 방출된다(이중 알림 방지).
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: None,
                running: Some(0),
            },
        );
        clock.advance(6000);
        hub.flush_expired();
        let notifs = events.notifications();
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].source, NotificationSource::Stop);
        assert!(notifs.iter().all(|n| n.source != NotificationSource::Hook));

        // running>0: Stop 알림 자체는 억제(#27)되지만 held 는 폐기된다.
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        hub.ingest_observer(
            "s1",
            ObserverEvent::Stop {
                message: None,
                running: Some(1),
            },
        );
        clock.advance(6000);
        hub.flush_expired();
        assert!(events.notifications().is_empty());
        assert!(hub.pending("s1").is_empty());
    }

    #[test]
    fn hold_output_within_grace_ignored_then_flood_after_grace_discards() {
        // grace(1s) 내 대량 출력은 무시 → 만료 시 방출.
        let (hub, _events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        clock.advance(500); // < grace
        hub.on_output("s1", 100_000);
        clock.advance(4500); // 만료(t=5000)
        hub.flush_expired();
        assert_eq!(hub.pending("s1").len(), 1, "grace 내 출력은 폐기하지 않는다");

        // grace 후 누적 8KB 이상 → 폐기.
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        clock.advance(1500); // > grace
        hub.on_output("s1", 9000); // > 8KB
        clock.advance(4000);
        hub.flush_expired();
        assert!(hub.pending("s1").is_empty());
        assert!(events.notifications().is_empty());

        // grace 후 8KB 미만 → 만료 시 방출.
        let (hub, _events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        clock.advance(1500);
        hub.on_output("s1", 1000);
        hub.on_output("s1", 2000); // 누적 3000 < 8192
        clock.advance(4000);
        hub.flush_expired();
        assert_eq!(hub.pending("s1").len(), 1);
    }

    #[test]
    fn hold_same_key_keeps_timer_but_different_key_replaces() {
        // 같은 dedup_key 재수신 → 단일 held, 원래 held_at 유지(교체 안 됨).
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q")); // held at t=0
        clock.advance(3000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q")); // 같은 키 → 무시
        clock.advance(2000); // t=5000, 원래 타이머 기준 5s 경과
        hub.flush_expired();
        assert_eq!(events.notifications().len(), 1);

        // 다른 키 → 교체(만료 시 새 메시지만, 새 타이머로 방출).
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("first")); // held at t=0
        clock.advance(3000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("second")); // 교체 → held at t=3000
        clock.advance(2000); // t=5000, 새 타이머 기준 2s < 5s
        hub.flush_expired();
        assert!(hub.pending("s1").is_empty(), "교체된 타이머는 아직 만료 전");
        clock.advance(3000); // t=8000
        hub.flush_expired();
        let notifs = events.notifications();
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].message, "second");
    }

    #[test]
    fn hold_full_clear_discards_but_partial_clear_keeps() {
        // clear(None) → held 폐기.
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        hub.clear("s1", None);
        clock.advance(6000);
        hub.flush_expired();
        assert!(events.notifications().is_empty());

        // clear(Some(ids)) → held 유지.
        let (hub, events, clock) = hold_fixture(5000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        hub.clear("s1", Some(vec!["irrelevant".to_string()]));
        clock.advance(6000);
        hub.flush_expired();
        assert_eq!(events.notifications().len(), 1);
    }

    #[test]
    fn hold_zero_emits_hook_immediately() {
        // 기본(hold=0)에서는 현행 동작 그대로 즉시 방출한다.
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
    }

    #[test]
    fn flush_discards_held_when_session_gone_from_registry() {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone(),
            clock.clone(),
            Duration::from_millis(3000),
        ));
        hub.set_hold_duration(Duration::from_millis(5000));

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("q"));
        registry.remove("s1"); // flush 시점에 세션 사망
        clock.advance(6000);
        hub.flush_expired();
        assert!(events.notifications().is_empty());
        assert!(hub.pending("s1").is_empty());
    }

    #[test]
    fn bell_source_emits_immediately_even_when_hold_enabled() {
        let (hub, events, _clock) = hold_fixture(5000);
        hub.on_bell("s1");
        assert_eq!(events.notifications().len(), 1);
        assert_eq!(events.notifications()[0].source, NotificationSource::Bell);
        assert_eq!(hub.pending("s1").len(), 1);
    }

    #[test]
    fn set_hold_duration_runtime_change_applies_to_subsequent_ingest() {
        let (hub, events, clock) = fixture(); // hold=0
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("first")); // 즉시 방출
        assert_eq!(events.notifications().len(), 1);

        hub.set_hold_duration(Duration::from_millis(5000));
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("second")); // 이제 보류
        assert_eq!(events.notifications().len(), 1, "두 번째는 보류되어 방출 안 됨");

        clock.advance(5000);
        hub.flush_expired();
        assert_eq!(events.notifications().len(), 2);
        assert_eq!(hub.pending("s1")[1].message, "second");
    }

    // ---- ingest_activity: dedup/큐 우회, now_ms 타임스탬프 ----

    #[test]
    fn ingest_activity_emits_event_with_backend_timestamp_and_resolved_agent() {
        let (hub, events, clock) = fixture();
        clock.advance(500); // now_ms = 1_700_000_000_500

        hub.ingest_activity("s1", ActivityKind::Prompt);

        let acts = events.activities();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].agent_id, "a1");
        assert_eq!(acts[0].session_id, "s1");
        assert_eq!(acts[0].kind, ActivityKind::Prompt);
        assert_eq!(acts[0].at, 1_700_000_000_500); // 백엔드 now_ms()

        // 알림 파이프라인은 전혀 오염되지 않는다.
        assert!(events.notifications().is_empty());
        assert!(hub.pending("s1").is_empty());
    }

    #[test]
    fn ingest_activity_bypasses_dedup_even_for_identical_repeats() {
        let (hub, events, _clock) = fixture();
        hub.ingest_activity("s1", ActivityKind::Tool);
        hub.ingest_activity("s1", ActivityKind::Tool); // 같은 종류 연속 — dedup 없이 둘 다 방출

        assert_eq!(events.activities().len(), 2);
        assert!(hub.pending("s1").is_empty());
    }

    #[test]
    fn ingest_activity_for_unregistered_session_is_discarded() {
        let registry = Arc::new(SessionRegistry::new()); // s1 미등록
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub =
            NotificationHub::new(registry, events.clone(), clock, Duration::from_millis(3000));

        hub.ingest_activity("s1", ActivityKind::Prompt);

        assert!(events.activities().is_empty());
    }

    // ---- overhead-task-label: prompt 원문 추출 ----

    fn prompt_body(text: &str) -> Vec<u8> {
        serde_json::json!({ "session_id": "s1", "prompt": text })
            .to_string()
            .into_bytes()
    }

    #[test]
    fn ingest_activity_with_body_extracts_and_truncates_prompt_text() {
        let (hub, events, _clock) = fixture();

        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body("버그 고쳐줘"));
        let acts = events.activities();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].kind, ActivityKind::Prompt);
        assert_eq!(acts[0].text.as_deref(), Some("버그 고쳐줘"));

        // 2,000자 초과 → chars 기준 절단 (멀티바이트 안전)
        let long: String = "가".repeat(2500);
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body(&long));
        let acts = events.activities();
        assert_eq!(acts[1].text.as_ref().unwrap().chars().count(), 2000);
    }

    #[test]
    fn ingest_activity_with_body_survives_bad_bodies() {
        let (hub, events, _clock) = fixture();
        // 비JSON / prompt 필드 부재 / 빈 문자열 → text=None, 이벤트는 방출된다.
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, b"not json");
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, br#"{"session_id":"s1"}"#);
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, br#"{"prompt":"   "}"#);
        let acts = events.activities();
        assert_eq!(acts.len(), 3);
        assert!(acts.iter().all(|a| a.text.is_none()));
    }

    #[test]
    fn ingest_activity_with_body_drops_command_prompts() {
        let (hub, events, _clock) = fixture();
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body("!git status"));
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body("/clear"));
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body("#메모"));
        let acts = events.activities();
        assert_eq!(acts.len(), 3);
        assert!(acts.iter().all(|a| a.text.is_none()));
    }

    #[test]
    fn plain_ingest_activity_keeps_text_none() {
        let (hub, events, _clock) = fixture();
        hub.ingest_activity("s1", ActivityKind::Tool);
        assert_eq!(events.activities()[0].text, None);
    }

    // ---- temporary legacy hook entry point fallback behavior ----

    #[test]
    fn observer_legacy_ingest_hook_prefers_body_message_field() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("custom text"));
        assert_eq!(events.notifications()[0].message, "custom text");
    }

    #[test]
    fn observer_legacy_ingest_hook_uses_neutral_stop_fallback() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Stop, b"{}");
        assert_eq!(events.notifications()[0].message, "작업이 완료되었습니다.");
    }

    #[test]
    fn observer_legacy_ingest_hook_uses_neutral_attention_fallback() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, b"{}");
        hub.ingest_hook("s1", NotificationSource::Bell, b"{}");
        assert_eq!(events.notifications()[0].message, "확인이 필요합니다");
        assert_eq!(events.notifications()[1].message, "확인이 필요합니다");
    }

    #[test]
    fn observer_legacy_ingest_hook_falls_back_on_blank_message_field() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("   "));
        assert_eq!(events.notifications()[0].message, "확인이 필요합니다");
    }

    #[test]
    fn observer_legacy_ingest_hook_falls_back_on_invalid_json_body() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, b"not json");
        assert_eq!(events.notifications()[0].message, "확인이 필요합니다");
    }

    // ---- dedup_key sanity (used implicitly above; pin the algorithm's key inputs directly) ----

    #[test]
    fn dedup_key_differs_by_session_source_and_message() {
        let base = dedup_key("s1", NotificationSource::Hook, "hello");
        assert_ne!(base, dedup_key("s2", NotificationSource::Hook, "hello"));
        assert_ne!(base, dedup_key("s1", NotificationSource::Stop, "hello"));
        assert_ne!(base, dedup_key("s1", NotificationSource::Hook, "other"));
        assert_eq!(base, dedup_key("s1", NotificationSource::Hook, "hello")); // stable/deterministic
    }

    #[test]
    fn dedup_key_trims_message_whitespace() {
        assert_eq!(
            dedup_key("s1", NotificationSource::Hook, "hello"),
            dedup_key("s1", NotificationSource::Hook, "  hello  ")
        );
    }

    // Sanity check that Clock is genuinely an injection seam usable behind a
    // trait object (mirrors state.rs's analogous AppEvents check).
    #[test]
    fn fake_clock_is_usable_behind_the_clock_trait_object() {
        // Advance through the concrete type (only tests get to control
        // time), then erase to `Arc<dyn Clock>` — the same shape
        // `NotificationHub::new` requires in production — and confirm both
        // `now()` and `now_ms()` reflect the advance through the trait object.
        let clock = Arc::new(FakeClock::new());
        let ms_before = clock.now_ms();
        let instant_before = clock.now();
        clock.advance(2000);
        let handle: Arc<dyn Clock> = clock;

        assert_eq!(handle.now_ms() - ms_before, 2000);
        assert_eq!(
            handle.now().duration_since(instant_before),
            Duration::from_millis(2000)
        );
    }
