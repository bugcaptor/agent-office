# 외부 터미널 세션 attach — 캐릭터를 앱 밖 터미널/tmux에 붙이기

상태: 정본 — 구현 완료(kbm #2by). 갱신: 2026-08-16.

앱 밖에서 시작된 터미널 세션(iTerm 등 일반 터미널, tmux)에 캐릭터를 붙여, 오피스 캐릭터·데스크탑 마스코트가 그 세션의 훅 알림(완료/입력대기)을 받고 성격 프롬프트를 claude에 주입한다.

## 1. 성립 근거

상태 감지는 PTY와 무관한 독립 채널이다. forwarder(`observer/forwarder.rs`)는 env `AGENT_OFFICE_SESSION`+`AGENT_OFFICE_HOOK_URL`만 있으면 어느 셸에서든 `/hook`으로 POST하고, 포트 스테일 시 `AGENT_OFFICE_APP_DATA/observer-port` 파일로 재발견한다. 파이프라인의 유일한 게이트는 `NotificationHub`가 session_id를 `SessionRegistry`로 agentId 해석하는 부분 — 등록 없는 sid의 이벤트는 폐기된다. 따라서 **"PTY 없는 논리 세션"을 SessionRegistry에 등록하는 것**이 기능의 최소 확장점이고, 성격 프롬프트도 기존 `claude --append-system-prompt "$AGENT_OFFICE_PERSONA"` 셸 래퍼를 외부 셸에 그대로 이식하면 된다.

불가능한 것: 외부 터미널 에뮬레이터가 쥔 PTY master fd는 가져올 수 없으므로 일반 외부 세션의 화면 미러링은 하지 않는다(tmux는 예외 — §5).

## 2. 두 가지 붙기 모드

| 모드 | 명령 | 화면 | 알림/성격 | 입력 주입(봇) |
|---|---|---|---|---|
| 일반 외부 터미널 | `eval "$(agent-office ctl attach <agentId>)"` | 없음(앱 탭은 안내 패널) | O | X (`is_running`=false라 봇 모드 게이트가 막음) |
| tmux | `agent-office ctl attach <agentId> --tmux <target>` | 앱 탭 = tmux 클라이언트 미러 | O (pane 안 eval로 합류) | O (`write_input` → 활성 pane) |

## 3. 논리 세션 (`session/external.rs`)

- `SessionManager.externals: parking_lot::Mutex<HashMap<AgentId, ExternalSession>>` — `Session` 구조체는 무변경. `ExternalSession { session_id, shell_pid, cleanup_paths, attached_at_ms }`.
- `attach_external[_with_profile](agent_id, shell_pid, cwd, personality_prompt)`:
  - in-app 세션이 살아 있으면(Running/Starting, !kill_requested) **BindExisting** — 같은 sid로 plan만 재생성해 반환, externals 미등록·이벤트 미방출. 1캐릭터 1세션 불변식 유지의 핵심.
  - 아니면 새 UUID sid → `prepare_session_plan`(§4) → `registry.insert(Running)` → `session_started(shell="external")`+`session_state(Running, external=true)` → externals 삽입.
  - 기존 external이 있으면 교체 재발급(낡은 sid 훅은 미등록으로 폐기, 무해).
- `detach_external(agent_id, reason)`: registry 제거 + `hub.purge_session` + settings 파일 삭제 + `session_state` 방출(사용자 해제=Disposed/intentional, 셸 종료=Exited).
- **끊김 감지**: ctl이 `parent_id()`로 수집한 셸 PID를 전달하고, 앱이 5초 tokio interval로 `sweep_externals()` — `libc::kill(pid,0)`이 ESRCH일 때만 detach(EPERM은 생존). EXIT trap 방식은 사용자 trap을 덮어쓸 위험이라 기각.
- 기존 메서드와의 관계: `session_id_for`는 sessions 우선→externals 폴백(pending_notifications/ctl clear가 그대로 동작), `dispose`/`dispose_all`/`create_with_profile`은 external을 자동 detach, `is_running`은 externals 미포함(봇 모드가 외부 세션에 스스로 안 붙게).
- `prepare_session_plan(session_id, personality_prompt)`: create_with_profile에서 추출한 공용 헬퍼 — observer plan(훅 settings 파일 작성)+pi 확장+persona 래퍼 병합+기본 env(SESSION/HOOK_URL/APP_DATA). TERM은 PTY 전용이라 plan에서 제외(외부 셸의 TERM을 덮지 않음), create가 원래 자리에 되끼운다.

## 4. ctl attach/detach (`control/`, `session/attach_script.rs`)

- 라우트 `/v1/attach`·`/v1/detach` — 기존 2단계 승인(cli_enabled + control-token)으로 충분(스크립트에 비밀 없음: 훅 URL·설정 경로·persona뿐). persona·이름·역할은 store의 프로필에서 읽는다. 미존재 agentId는 ok:false(캐릭터 자동 생성 안 함).
- 응답 `AttachResult { session_id, mode: "new"|"bind"|"tmux", script }`. **ctl은 성공 시 script만 stdout raw**, 그 외 전부 stderr — 실패 시 `eval "$(...)"`가 빈 문자열을 받아 안전.
- 스크립트(`render_attach_script`): 헤더 코멘트 → plan.env `export`(sh_quote 인용) → `render_posix(wrappers)`(unalias 포함, claude 함수 shim). 이미 뜬 셸에 eval하므로 ZDOTDIR/rcfile shim 불필요, zsh/bash 공용. fish 미지원. observer OFF면 훅 env 없이 persona 래퍼만+경고 코멘트.
- 훅 settings 파일은 내용이 세션 무관(`observer/claude.rs` write_settings_file, temp+rename 멱등)이라 외부 셸에서도 `claude --settings $AGENT_OFFICE_SETTINGS` 그대로 유효.

## 5. tmux 풀 기능 (`control/tmux.rs`)

접근: 앱이 **자기 PTY로 tmux 클라이언트를 하나 더 연다** — `create_with_profile`에 `startup_command = "exec tmux attach-session -t '<target>'"`. 기존 파이프라인(OutputSink/write_input/resize/on_exit/세션로그/봇 inject) 전부 무수정 재사용. pipe-pane+send-keys 안은 가짜 SpawnedPty·이스케이프·크기 불일치로 사실상 병렬 파이프라인 신설이라 기각했다.

- target 검증(빈 값·제어문자 거부, sh_quote로 데이터 인용) + `tmux has-session` 프로브(`TmuxProbe` 클로저 주입 — 테스트는 가짜, 프로덕션은 `system_probe`).
- 이미 살아 있는 앱 세션이 있으면 create가 재사용해 startup_command가 무시되므로 **성공 위장을 거절**(sid 동일하면 ok:false, dispose 후 재시도 안내).
- 응답 script는 코멘트 2줄(eval 무해) — pane 안에서 `eval "$(agent-office ctl attach <agentId>)"`를 실행하라는 안내. pane 셸은 in-app 세션(tmux 클라이언트)이 live이므로 **BindExisting으로 같은 sid에 합류** — 훅·persona가 앱 탭(미러 뷰)과 같은 세션에 귀속된다.
- 시맨틱: 앱 창 크기가 tmux window 크기에 영향(window-size latest). dispose는 클라이언트만 종료(tmux 서버 무사, 비파괴). Exited 후 "다시 띄우기"는 프로필 startup_command 기준이라 tmux 재접속이 아님. 봇 inject는 활성 pane으로 가므로 단일 pane 세션 권장. 상세는 `cli-control-design.md`의 tmux 절.

## 6. 프런트

- `SessionStateEvent.external?: boolean`(additive, skip_if_none) → `SessionRuntime.kind: "pty"|"external"`. kind는 매 전이의 external 유무로 확정(PTY 경로는 항상 미포함 → 자동 "pty" 복귀).
- `TerminalHost`: external+running이면 xterm 대신 안내 패널("화면 미러링 없음")+연결 해제 버튼(`detach_external_session` invoke). tmux 세션은 kind="pty"라 기존 풀 터미널 뷰.
- 오피스 씬/알림 티커/OS 알림/TTS/마스코트는 이벤트만 소비하므로 무변경으로 동작.

## 7. 제약·후속 후보

- unix 전용(PID 스윕·attach 스크립트). fish 미지원.
- 앱 재시작 시 논리 세션 소실(영속+부트 재등록은 후속). PID 재사용 오탐은 희귀 케이스로 수용.
- 외부 세션 detach 후 탭의 종료 배너 문구가 PTY용("프로세스가 종료되었습니다")과 어색 — 후속 다듬기 후보.
- 출력 기반 노이즈 억제 휴리스틱(hub의 8KB resume watch)은 외부 세션에선 출력이 없어 비활성(정상 동작, Stop/Attention은 그대로).

## 8. 구현 맵

| 영역 | 파일 |
|---|---|
| 논리 세션 | `src-tauri/src/session/external.rs`(+tests), `session/manager.rs`(externals·prepare_session_plan), `types.rs`(external 필드) |
| attach 스크립트 | `src-tauri/src/session/attach_script.rs`, `session/wrapper_script.rs`(sh_quote·safe_env_identifier pub(crate)) |
| ctl | `src-tauri/src/control/{mod,protocol,client}.rs`, `control/tmux.rs` |
| IPC/부트 | `src-tauri/src/ipc/commands/session.rs`(detach_external_session), `lib.rs`(sweep 태스크) |
| 프런트 | `src/shared/{ipc.ts,types/session.ts,types/api.ts}`, `src/renderer/store/{types,appStore}.ts`, `renderer/ipc/{tauriApi,sessionBridge}.ts`, `renderer/terminal/TerminalHost.tsx` |
