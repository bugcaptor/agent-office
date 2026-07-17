# Claude 세션 이어하기(resume) 설계

이슈 #20. Claude Code의 native 세션 ID(리줌 ID)를 기억해 두었다가, 사용자가
원할 때 `claude --resume <id>`로 이전 대화를 이어서 시작하는 기능.

## 1. 배경과 요구

- Claude Code는 종료 시 "Run `claude --resume <id>`" 안내를 출력하지만,
  agent-office는 그 ID를 어디에도 기록하지 않아 사용자가 직접 복사해야 했다.
- 이슈 요구: 종료할 때 나오는 리줌 ID를 기억했다가 이어하기. **종료 시점이
  아니더라도 알 수 있으면 기억하기.**

핵심 관찰: Claude Code 훅 POST body에는 **모든 이벤트마다** native
`session_id`(그리고 `cwd`)가 실려 온다. 현재 파이프라인은 이를 전부 버린다
(`observer/event.rs`에 추출기 없음). 터미널 출력 파싱이 아니라 훅 body에서
캡처하면 "종료 전에도 기억하기"가 자연스럽게 충족된다.

주의: 저장소 곳곳의 `session_id`(SessionRegistry, session-events 시계열)는
**agent-office 자체 UUID**다. native ID는 기존 스토어 어디에서도 복구할 수
없으므로 별도 캡처·저장이 필요하다.

## 2. 캡처 — observer ingest 경로

- `observer/event.rs`에 추출기 추가:
  - `native_session_id(body) -> Option<String>` — top-level `session_id`
    문자열, 공백/빈 값은 None.
  - `hook_cwd(body) -> Option<String>` — top-level `cwd`.
- `ObserverRuntime`에 옵셔널 sink를 주입:

  ```rust
  pub trait ClaudeSessionSink: Send + Sync {
      /// ao_session_id = agent-office UUID(훅 라우팅 키), native = Claude 세션 ID.
      fn record(&self, ao_session_id: &str, native_session_id: &str, cwd: Option<&str>);
  }
  ```

  `ObserverRuntime::ingest`에서 provider가 Claude이면 `map_hook` 결과와
  **무관하게**(서브에이전트 훅이 None으로 걸러져도 body의 session_id는 메인
  세션 것이므로) 추출해 sink에 전달한다. sink 부재(테스트 다수)면 no-op.
- 프로덕션 sink `ClaudeResumeRecorder`:
  - `SessionRegistry::resolve_agent(ao_session_id)`로 agent_id 해석.
    해석 실패(미등록 세션)면 버린다.
  - ao_session별 마지막 native ID를 in-memory 캐시로 들고, **값이 바뀔 때만**
    스토어에 기록 — 훅마다 디스크 쓰기 방지. `/clear`·resume으로 native ID가
    갈리면 최신값이 이긴다.

## 3. 저장 — claude-resume.json

`persistence/claude_resume_store.rs`, 경로 `<app-data>/claude-resume.json`:

```json
{
  "agents": {
    "<agentId>": { "sessionId": "…", "cwd": "/path", "updatedAt": 1730000000000 }
  }
}
```

- 에이전트당 최신 1건(MVP). 기존 스토어처럼 Mutex + tmp→rename 원자 쓰기.
- 로드 실패(파손)는 빈 상태로 fail-open.
- 프로필 삭제된 에이전트의 잔존 엔트리는 무해 — 정리는 후속 과제.

## 4. 실행 — 기존 startup_command 경로 재사용

세션 생성의 Rust 경로는 **변경하지 않는다**. 렌더러가 이어하기 시:

1. 기존 세션 dispose(재시작 플로우와 동일),
2. `startupCommand: "claude --resume <id>"`를 이번 1회 생성에만 override로
   전달해 세션 재생성.

셸 래퍼가 `claude` 명령에 `--settings $AGENT_OFFICE_SETTINGS`(그리고 페르소나)
를 투명 주입하므로 훅 배선이 그대로 유지된다. 리줌 ID는 stdin 셸 라인에
들어가므로 **UUID 형식(`^[0-9a-fA-F-]+$`) 검증 후에만** 명령을 구성한다.

한계(감수): `claude --resume`은 같은 프로젝트 디렉터리에서만 세션을 찾는다.
캡처 시점의 `cwd`를 함께 저장해 두되, MVP는 프로필 cwd가 보통 고정이라는
전제로 그대로 실행한다.

## 5. IPC·UI

- 커맨드 `list_claude_resume_sessions() -> { [agentId]: { sessionId, cwd?, updatedAt } }`.
  렌더러는 메뉴를 열 때 조회(이벤트 푸시 불필요).
- 컨텍스트 메뉴에 "이전 세션 이어하기" — 해당 에이전트의 엔트리가 있을 때만
  활성. 확인 다이얼로그(ConfirmRestartDialog 패턴)에서 현재 세션 종료를 고지.
- 플로우는 `restartAgentSession.ts` 변형: dispose → epoch bump → 생성 시
  startupCommand override.

## 6. 비목표

- 세션 이력 여러 건 보관·선택 UI(최신 1건만).
- Codex/Pi의 resume(훅 body 계약이 다름 — 후속 이슈).
- `--fork-session` 등 resume 변형 옵션.
