# 세션 핸드오프 설계 — 앱 종료 후 터미널 존속 + 재실행 시 이어받기

작성: 2026-07-17 (Fable). 상태 갱신: 2026-07-20.
상태: 정본 — **v1 구현 완료, 현재 기본 경로**(이슈 #7 닫힘). v2 상시 브로커
(`docs/session-broker-v2-design.md`)와 공존 중이다: v2는
`AGENT_OFFICE_SESSION_BROKER=v2` opt-in·기본 off이고, 기본값에서는 이 문서의
v1 fd-핸드오프가 동작한다. `SessionManager`는 `broker_mode`/세션별 `broker_owned`
플래그로 두 경로를 분기하며, 혼합 상황(브로커 모드에서의 폴백 세션 등) 처리는
v2 문서의 "세션 단위 소유 플래그" 절이 정본. v1 제거는 REBUILD-PLAN R-8의 별도
결정 항목이다.

구현 파일(2026-07 리팩터 반영): v1 핸드오프·입양 갈래는
`session/handoff_v1.rs`(`handoff_all`/`handoff_one`/`adopt_detached`/`adopt_one`)로,
v2 브로커 갈래는 `session/handoff_broker.rs`로 파일 격리됐다.
poll reader는 `session/poll_reader.rs`, 데몬은 `sessiond/{daemon,client,protocol}.rs`.

## 목표

앱 업데이트 등으로 앱을 종료해도 진행 중인 터미널(그 안의 claude/codex 작업 포함)이
계속 살아 있고, 앱을 다시 띄우면 그 터미널을 그대로 이어받는다.

- 종료 시 근무 중 에이전트가 있으면 확인 모달에서 선택: **[터미널 유지하고 종료] [모두 종료하고 종료] [취소]**
- "유지"를 고르면 PTY 세션들이 작은 데몬 프로세스(`sessiond`)로 넘어가 존속.
- 재실행 시 부트스트랩에서 자동으로 데몬의 세션을 되찾아(입양, adopt) 터미널에 다시 연결.

**범위: unix(macOS 우선) 전용.** Windows(ConPTY)는 핸들 이전이 달라 v1 제외 —
`handoff_supported()`가 false를 반환하고 모달은 기존 2버튼 유지.

## 왜 이 방식인가 (결정)

- 현재 구조: portable-pty로 앱 프로세스가 PTY 마스터를 직접 소유, `ExitRequested`에서
  `dispose_all()`로 전부 SIGKILL (`lib.rs:237`, `manager.rs:442`).
- 자식 프로세스 자체는 앱이 죽어도 살 수 있다(reparent). 죽는 이유는 마스터 fd가 닫히며
  SIGHUP을 받기 때문. 따라서 **마스터 fd를 다른 프로세스가 이어 쥐면 세션은 산다**.
- fd는 유닉스 도메인 소켓의 SCM_RIGHTS로 프로세스 간 전달 가능. tmux/screen 같은 외부
  의존 없이 자체 데몬으로 해결한다 (tmux는 설치 의존 + 키바인딩/스크롤백 UX 간섭).
- **항시 브로커가 아니라 종료 시점 핸드오프**를 택한다: 평상시 핫패스(스폰/입출력)는
  변경 0, 기능은 순수 추가. (단점: 앱 크래시 시엔 세션이 죽는다 — 목표가 "업데이트를
  위한 정상 종료"이므로 v1에서 수용.)

## 아키텍처 개요

```
[앱 (Tauri)]                         [sessiond 데몬]
 SessionManager                       세션 테이블 {agent_id → fd, meta, ring buffer}
   │ 종료 시 handoff_all()              │ fd별 reader 스레드 → 512KB 링버퍼
   │  reader 인터럽트 → fd+메타 전송 ───▶│ EOF/EIO → exited 표시
   │  (UDS + SCM_RIGHTS)               │
   │ 시작 시 adopt_all()                │
   │  List/Adopt ◀── fd+버퍼 반환 ──────│ 테이블 비면 자체 종료
```

- 소켓: `app_data_dir()/sessiond.sock` (0700 디렉토리, 같은 uid만).
- 데몬 바이너리: 별도 빌드 없이 **앱 실행 파일 자신**을 `--sessiond <socket_path>`
  인자로 스폰 (기존 `maybe_run_observer_forwarder` 분기와 같은 패턴, `main.rs:5`).
  `pre_exec`에서 `setsid()`로 세션 분리(앱/터미널 시그널 미전파), stdio는
  `app_data_dir()/sessiond.log`로 리다이렉트.

## 프로토콜 (UDS, 버전 1)

프레이밍: `u32 LE 길이 + JSON(serde)`. fd는 해당 프레임 sendmsg의 SCM_RIGHTS 보조
데이터로 1개 첨부. `nix` crate 사용 (features: socket, uio, poll, term, signal, process).

앱 → 데몬:
- `Hello { proto: 1 }` — 응답 `HelloOk { proto: 1 }`. proto 불일치 시 데몬이 에러 응답, 앱은 입양 포기(세션은 데몬에 남음).
- `Handoff { agent_id, session_id, pid, pgid, rows, cols, cwd, cleanup_paths: Vec<String> }` + fd(마스터) — 응답 `HandoffOk`.
- `List` — 응답 `ListOk { sessions: [ { agent_id, session_id, pid, rows, cols, cwd, exited: bool, buffered_bytes } ] }`.
- `Adopt { agent_id }` — 응답 `AdoptOk { meta…, buffer_b64 }` + fd. 데몬은 해당 항목을 테이블에서 제거(reader 스레드 먼저 정지 — 아래 "이중 리더 금지" 참조).
- `Kill { agent_id }` — pgid에 SIGKILL 후 테이블 제거. 응답 `KillOk`.

데몬 수명: 연결이 끊길 때마다 테이블이 비어 있으면 소켓 파일 지우고 exit(0).
기동 직후 60초 안에 Handoff가 하나도 없으면 exit (고아 데몬 방지).

## 핵심 설계 포인트

### 1. 이중 리더 금지 (바이트 유실 방지)

같은 마스터를 두 프로세스가 동시에 read하면 바이트가 쪼개져 유실된다. 순서 강제:

- **핸드오프**: 앱 쪽 reader 스레드를 *먼저* 확정적으로 종료시킨 뒤 fd를 보낸다.
  현재 reader는 블로킹 `read()`(`manager.rs:331-345`)라 인터럽트 불가 →
  **unix에서 poll 기반으로 교체**: `PortablePtyFactory`가 reader를
  `poll([master_fd_dup, shutdown_pipe_read])` 루프로 만들고, `SpawnedPty`에
  `reader_interrupt: Option<ReaderInterrupt>` (shutdown pipe write end) 추가.
  인터럽트 후 아직 안 읽은 바이트는 커널 tty 버퍼에 남아 데몬이 이어 읽는다 → 무손실.
  Fake/Windows는 `None` (핸드오프 자체가 unix 전용).
- **입양**: 데몬이 reader 스레드를 정지시킨(같은 pipe 트릭) *뒤* fd를 보낸다.

### 2. SpawnedPty/Session 확장 (unix)

- portable-pty 0.8 unix API: `MasterPty::as_raw_fd()`, `MasterPty::process_group_leader()`,
  `Child::process_id()` 사용.
- `SpawnedPty`에 추가: `reader_interrupt: Option<…>`, `handoff: Option<HandoffInfo { master_fd(RawFd, dup 소유), pid, pgid }>`.
- `Session`에 추가: `handed_off: AtomicBool`, handoff 메타 보관.

### 3. SessionManager::handoff_all() → usize

Running 세션 각각에 대해:
1. `reader_interrupt` 발화 → reader 종료 대기(짧은 join 또는 완료 채널).
2. `handed_off` set → 데몬 connect(없으면 스폰 후 ~2초 백오프 재시도) → `Handoff` 전송.
3. 성공: sessions 맵/registry에서 제거하되 **cleanup_paths는 지우지 않는다**
   (셸 안에서 나중에 `claude --settings "$AGENT_OFFICE_SETTINGS"` 재기동 가능해야 함;
   경로 목록을 데몬 메타로 넘겨 입양 후 최종 dispose 때 정리).
4. `on_exit`/`dispose` 초입에 `handed_off`면 즉시 return (kill·cleanup·상태이벤트 금지).
5. `dispose_all()`은 handed_off 세션 스킵 (기존 호출 경로 불변).

실패한 세션은 그대로 두고 개수만 집계 — 프론트는 성공 수와 무관하게 종료 진행.

### 4. adopt 경로 — SpawnedPty와 동형으로 재조립

`SessionManager::adopt_detached()`(시작 시 1회):
1. 소켓 없으면 no-op. Hello/List → 영속 프로필에 있는 agent_id는 Adopt,
   **없는 agent_id는 Kill**(삭제된 에이전트의 고아 claude 방지). exited 항목은 스킵.
2. 받은 fd로 `SpawnedPty` 동형 번들 구성:
   - reader: poll 기반(위와 동일 구현 재사용) + EOF 시 완료 채널 발화
   - writer: fd dup의 `File`
   - control: `AdoptedControl { fd, pgid }` — resize는 `TIOCSWINSZ` ioctl,
     kill은 `kill(pgid, SIGKILL)` (pgid 없으면 pid)
   - waiter: `EofWaiter` — reader EOF 채널 수신까지 블로킹, `ExitOutcome { None, None }`
     (자식이 아니라 waitpid 불가 — 마스터 EOF를 종료 신호로 쓴다)
3. **create_with_profile의 spawn 이후 배선부(세션 등록, reader/pump/wait 3스레드,
   Running CAS)를 `install_session(...)`으로 추출**해 create/adopt가 공유한다.
   상태 머신·sink 재사용 로직(`manager.rs:305-378`)은 그대로.
4. session_id는 핸드오프 때 것을 재사용. autostart/startup_command 주입은 하지 않는다.
5. 데몬이 버퍼링한 출력(base64)은 pump mpsc에 첫 `ReaderMsg::Data`로 주입 —
   프론트가 아직 구독 전이면 기존 sink 백로그(BACKLOG_CAP 256)가 보관한다.
   데몬 링버퍼 512KB ≤ 백로그 용량과 정합.

### 5. observer 훅 포트 스테일 문제

세션 env의 `AGENT_OFFICE_HOOK_URL`은 스폰 시점 포트(`server.rs:71`, 임시 포트)를
담는다 → 재시작 후 입양된 세션의 훅이 죽은 포트를 때린다. 완화:
- 서버 기동 시 `app_data_dir()/observer-port` 파일에 현재 포트 기록.
- forwarder(`observer/forwarder.rs`)가 env URL로 POST 실패(연결 거부) 시 포트 파일을
  읽어 같은 경로로 1회 재시도.
- codex·claude 훅 모두 앱 바이너리 forwarder(`--observer-forward <provider> [event]`,
  공용 명령 빌더 `observer/hook_command.rs`)를 경유해 이 재시도 경로를 탄다. 예전
  claude는 훅 URL을 curl 명령에 직접 박아 넣어(스폰 시점 포트) 재시작 후 입양된
  세션의 훅이 전부 유실됐다 — 이슈 #30에서 forwarder 경유로 전환해 해소했다.
- forwarder 경로 자체도 실행마다 안정해야 한다. Linux AppImage는 `current_exe()`가
  실행별 `/tmp/.mount_*`를 가리키므로 `$APPIMAGE`(원본 경로)를 우선한다
  (`lib.rs forwarder_executable_path()`). **알려진 한계**: macOS App Translocation
  (격리 플래그 있는 앱을 Downloads/DMG에서 제자리 실행)에서는 `current_exe()`가
  무작위 마운트라 같은 문제가 남는다 — 원본 경로 복원 API가 비공개라 완화하지
  않았고, 로컬 빌드·/Applications 설치(격리 플래그 없음)에는 발동하지 않는다.

### 6. 프론트

- `quitGuard.ts`/`ConfirmQuitDialog.tsx`: 부팅 시 `handoff_supported()` 1회 조회.
  지원 + Running 세션 존재 시 3버튼. "유지하고 종료" = `handoff_sessions` invoke
  완료 후 `getCurrentWindow().destroy()`. "모두 종료" = 기존 동작(destroy → dispose_all).
- `bootstrap.ts`: 상태 하이드레이트 후 `adopt_detached_sessions` invoke → 반환된
  agent_id들의 세션을 Running으로 스토어 시드(기존 session_state 이벤트로도 수렴).
- 리플레이 후 화면 복원: TIOCSWINSZ는 크기가 같으면 SIGWINCH를 안 쏜다.
  입양 세션의 터미널이 처음 attach될 때 **redraw nudge**: `fit()` → `resize(cols, rows-1)`
  → `fit()` (TUI가 SIGWINCH 2회로 완전 재도색; 일반 셸엔 무해).

## 커맨드 (ipc/commands.rs)

- `handoff_supported() -> bool` — cfg(unix)에서 true.
- `handoff_sessions() -> usize` — 넘긴 세션 수.
- `adopt_detached_sessions() -> Vec<AdoptedSessionInfo { agent_id, session_id, rows, cols }>`.

## 테스트

- 프로토콜 인코딩/디코딩 라운드트립 + fd 전달은 `socketpair`로 유닛 테스트(unix).
- poll reader: 실제 `openpty` + `/bin/cat`으로 인터럽트 시 무손실 검증(#[cfg(unix)]).
- manager: Fake에 handoff 시뮬레이션 훅을 더해 `handed_off` 세션이 dispose_all에서
  kill/cleanup되지 않음을 회귀 테스트. adopt 배선은 install_session 추출로 기존
  Fake 테스트가 커버.
- 데몬: 테이블 로직(입양 후 제거, 전부 소진 시 종료 조건)을 스레드+socketpair로 유닛.
- 프론트: vitest는 반드시 `--dir src` (프로젝트 관례).

## 수동 검증 시나리오 (릴리스 전)

1. 터미널에서 `claude` 실행해 긴 작업 시작 → Cmd+Q → "유지하고 종료".
2. `ps`로 셸/claude 생존 + `sessiond` 프로세스 확인. 로그 `sessiond.log` 확인.
3. 앱 재실행 → 터미널 자동 복원, 화면 재도색, 입력/리사이즈 정상, 훅 알림 동작.
4. 이어받은 세션에서 exit → Exited 전이 정상(EOF 경로).
5. "모두 종료" 경로와 에이전트 전원 퇴근 시 무모달 종료가 기존과 동일한지 확인.
