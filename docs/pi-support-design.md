# Pi(pi.dev) CLI 작업 상태 감지 지원 — 설계 문서

작성: 2026-07-12 (Fable, 주요 설계) · 상태: 사용자 승인 대기 항목 4건 포함
대상 버전: pi-coding-agent v0.80.3 기준 (로컬 실측: `/opt/homebrew/bin/pi`)

---

## 0. 요약

Claude Code의 훅 → `curl POST /hook` 파이프라인을 Pi에서는 **Pi 확장(extension) 1개**로
재현한다. 확장은 pi 프로세스 안에서 라이프사이클 이벤트를 구독해 기존
`127.0.0.1:<port>/hook?session=<id>&source=<kind>` 엔드포인트로 POST한다.
**hook_server → hub → turnReducer → UI 다운스트림은 한 줄도 수정하지 않는다.**

Claude 대비 구조적 단순화 1건: Claude는 훅 커맨드 문자열에 포트/세션id를 박아야
해서 세션별 `--settings` 파일(`HookSettingsWriter`)이 필요했지만, Pi 확장은
프로세스 내에서 `process.env`를 직접 읽을 수 있으므로 **정적 확장 파일 1개**면
된다. 세션 식별에 필요한 env는 이미 전부 주입되고 있다:

- `AGENT_OFFICE_SESSION` — 모든 세션에 무조건 주입 (src-tauri/src/session/manager.rs:217)
- `AGENT_OFFICE_HOOK_URL` — hooks_on일 때만 주입, 포트 포함 (src-tauri/src/session/manager.rs:221)

즉 **새 env 1개(`AGENT_OFFICE_PI_EXT`, 확장 파일 경로) + 셸 래퍼의 `pi()` 함수 +
확장 파일 본문**이 이번 작업의 전부다.

---

## 1. 현재 구조 확인 (file:line 근거)

| 계층 | 파일 | 역할 |
|---|---|---|
| 훅 스키마 | src-tauri/src/notification/hook_settings.rs:82-98 | Claude `--settings` JSON, 이벤트→source 매핑 (Notification→hook, Stop→stop, UserPromptSubmit→prompt, PostToolUse→tool, SubagentStart/Stop→sub-start/stop) |
| HTTP 수신 | src-tauri/src/notification/hook_server.rs:50-58 | `source` 쿼리로 라우팅. **프레임워크 무관** — `HookQuery`는 `session`/`source`만 역직렬화(30-35행), 미지 쿼리 파라미터는 무시됨 |
| 허브 | src-tauri/src/notification/hub.rs:65-94 | stop/hook → 알림(dedup 3s), prompt/tool/sub-* → activity 즉시 방출. `at`은 백엔드 `now_ms()` (90행) |
| 메시지 추출 | src-tauri/src/notification/hub.rs:189-212 | body `prompt`/`message` 필드 우선, 폴백 문자열만 Claude 전용("Claude finished a task") — **body에 message를 실으면 폴백을 안 탄다** |
| 상태머신 | src/renderer/timeline/turnReducer.ts:81-120 | prompt→턴 open, tool→waiting 복귀/하트비트, notification→waiting, stop/settle→정산 |
| 렌더러 배선 | src/renderer/store/appStore.ts:429-484 | activity(prompt/tool)→reduceTurn+라벨, 알림(stop→stop, hook/bell→notification)→reduceTurn, 세션 종료→settle |
| env/opt-in | src-tauri/src/session/manager.rs:180-237, src-tauri/src/lib.rs:45-46, src-tauri/src/persistence/settings_store.rs:28 | `claude_hooks_enabled` 단일 토글 → 훅 서버 기동 + settings 파일 + env + 셸 심 전부 게이트 |
| 셸 래퍼 | src-tauri/src/session/zsh_wrapper.rs:41-61, bash_wrapper.rs:17-25, shells.rs:144-154 | `claude()` 함수/PS function이 `--settings $AGENT_OFFICE_SETTINGS` 투명 주입 |

---

## 2. Pi 이벤트 경계 확정 (types.d.ts 실측 근거)

타입 정의: `/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent/dist/core/extensions/types.d.ts`

### 2.1 제안 매핑에서 수정된 것 — `input`은 waiting이 아니다

`InputEvent`(types.d.ts:603-614)는 **"Fired when user input is received, before
agent processing"** — 사용자가 입력을 *제출*했을 때 발화한다. 즉 Claude의
UserPromptSubmit 등가물(working 시작 후보)이지, "에이전트가 사용자 입력을
기다림"(waiting) 신호가 아니다. 과제 지시문의 후보 표는 이 지점에서 틀렸다.

더 중요한 발견: **Pi 코어에는 Claude `Notification` 훅의 등가물이 없다.**
`ExtensionEvent` 유니온 전체(types.d.ts:748)를 훑어도 권한 요청/입력 대기
알림성 이벤트가 없다 — Pi는 설계상 권한 게이트 없이 툴을 바로 실행하는
CLI다(`tool_call`은 확장이 *블록*할 수는 있어도, 코어가 사용자에게 물어보는
단계가 없음). 따라서:

> **결정 D1: v1에서 Pi 세션은 waiting 상태가 없다 (idle ↔ working 2상태).**
> turnReducer는 notification 입력이 안 들어오면 자연히 working→stop→idle로만
> 흐르므로(turnReducer.ts:108-113은 도달 불가일 뿐) 다운스트림 수정이 없다.
> 후속에서 Pi에 권한/질문 이벤트가 생기면 source=hook POST 한 줄 추가로 끝난다.

### 2.2 확정 매핑

| 앱 source | Claude 훅 | Pi 이벤트 (확정) | 근거 |
|---|---|---|---|
| `prompt` (턴 시작) | UserPromptSubmit | **`before_agent_start`** | types.d.ts:504-515. "Fired after user submits prompt but before agent loop". `prompt: string` 필드를 그대로 body `{"prompt": ...}`에 실으면 hub의 `extract_prompt_text`(hub.rs:189-197)와 라벨 파이프라인이 무수정 재사용된다 |
| `tool` (하트비트) | PostToolUse | **`tool_execution_end`** | types.d.ts:570-576. PostToolUse와 동일한 "툴 실행 완료 후" 시맨틱. (`tool_call`은 PreToolUse 등가라 부적합, `tool_execution_start`도 하트비트론 무해하나 시맨틱 일치를 위해 end 채택) |
| `stop` (idle 정산) | Stop | **`agent_end`** + body `{"message":"Pi finished a task"}` | types.d.ts:520-524. body에 message를 실으므로 Claude 전용 폴백 문자열(hub.rs:208)을 타지 않는다 |
| `stop` (안전망) | — | **`session_shutdown`** → source=stop 재전송 | types.d.ts:453-458 (quit/reload/new/resume/fork). 열린 턴 정산 보강. idle이면 reducer가 무시(turnReducer.ts:91-94), dedup은 메시지 문자열을 달리해("Pi session ended") 회피 |
| `hook` (waiting) | Notification | **v1 없음 (D1)** | 2.1 참조 |
| `sub-start/stop` | SubagentStart/Stop | **v1 제외** | Pi 코어에 서브에이전트 없음. pi-subagents는 서드파티 확장으로 `pi -p` 서브프로세스를 띄우는 방식 — 우리 확장이 관찰할 코어 이벤트가 없다. 후속 과제 |

`agent_start`/`turn_start`를 쓰지 않는 이유: `turn_start/turn_end`(types.d.ts:526-537)는
**LLM 호출 1회 단위**(Claude로 치면 어시스턴트 메시지 1개 단위)라 앱의 "턴"
(사용자 프롬프트→완료)보다 훨씬 잘다. `agent_start`(types.d.ts:517-519)는
의미는 맞지만 프롬프트 원문이 없다. `before_agent_start`가 시점과 페이로드
모두에서 유일하게 정확하다.

`input`을 쓰지 않는 이유: 스트리밍 중 steering/followUp 입력에도 발화하고
(`streamingBehavior` 필드, types.d.ts:612-613), followUp은 큐잉만 될 뿐 즉시
루프를 돌지 않는다. 큐잉된 메시지가 나중에 실행될 때 `before_agent_start`가
다시 발화하므로 그쪽이 회계적으로 정확하다. (스파이크 확인 항목 S1)

### 2.3 확정이 필요한 잔여 관찰 항목 → Phase 0 스파이크

- **S1**: followUp/steer 입력 시 `before_agent_start` 발화 횟수·시점
- **S2**: ESC/abort 시 `agent_end` 발화 여부 (미발화면 열린 턴은 다음 prompt의
  close-and-reopen(turnReducer.ts:86-89) 또는 세션 종료 settle(appStore.ts:476-484)이
  정산 — Claude에서 Stop 미발화와 동일한 기존 안전망이라 치명적이지 않음)
- **S3**: 슬래시 커맨드/로컬 처리 입력에서 `before_agent_start`가 발화하지 않는지
- **S4**: `pi -p`(print 모드)에서 이벤트 발화 (확장은 모든 모드에서 로드됨 —
  types.d.ts:207 `ExtensionMode = "tui" | "rpc" | "json" | "print"`)
- **S5**: `session_shutdown` 발화 조건 실측 (Ctrl+C 강제종료 포함 여부)

---

## 3. 아키텍처

```
┌ 앱 부팅 ──────────────────────────────────────────────────────────┐
│ pi_extension::ensure_extension()                                   │
│   → <tmp>/agent-office/pi/agent-office-pi.ts (정적, blind overwrite)│
└────────────────────────────────────────────────────────────────────┘
┌ 세션 생성 (manager.rs create, hooks_on일 때) ─────────────────────┐
│ env += AGENT_OFFICE_PI_EXT=<확장 파일 경로>                        │
│ (AGENT_OFFICE_SESSION / AGENT_OFFICE_HOOK_URL은 기존 그대로)        │
└────────────────────────────────────────────────────────────────────┘
┌ 셸 (zsh/bash/PowerShell 래퍼) ────────────────────────────────────┐
│ pi() { [ -n "$AGENT_OFFICE_PI_EXT" ] && command pi -e "$AGENT_..." │
│        "$@" || command pi "$@" }   ← claude() 래퍼와 나란히 정의    │
└────────────────────────────────────────────────────────────────────┘
┌ pi 프로세스 내 확장 ──────────────────────────────────────────────┐
│ env 가드(HOOK_URL/SESSION 없으면 전부 no-op)                        │
│ before_agent_start → POST source=prompt  body={"prompt": ...}       │
│ tool_execution_end → POST source=tool                                │
│ agent_end          → POST source=stop    body={"message":"Pi ..."}   │
│ session_shutdown   → POST source=stop    body={"message":"Pi ..."}   │
│ (+ &agent=pi 쿼리 — 서버는 오늘 무시, 후일 구분용 선제 송신)          │
└────────────────────────────────────────────────────────────────────┘
        ↓ HTTP POST 127.0.0.1:<port>/hook  (기존 엔드포인트)
   hook_server.rs → hub.rs → activity/notification 이벤트 → turnReducer → UI
   (전 구간 무수정)
```

### 3.1 확장 파일 배포 — HookSettingsWriter가 아니라 zsh_wrapper 패턴

세션별 내용이 없으므로(세션id/포트는 env에서) `HookSettingsWriter`의 세션별
write/cleanup(hook_settings.rs:100-116)과 RAII 가드(manager.rs:194-207)가 전부
불필요하다. `zsh_wrapper::write_shim`(zsh_wrapper.rs:67-81)과 동일한 패턴:

- 새 모듈 `src-tauri/src/session/pi_extension.rs`
  - `const PI_EXTENSION_TS: &str = r#"..."#;` — 확장 소스 임베드
  - `pub fn write_extension(base: &Path) -> io::Result<PathBuf>` — 정적, blind overwrite
  - `pub fn ensure_extension() -> io::Result<PathBuf>` — `<tmp>/agent-office/pi/agent-office-pi.ts`
- 호출 시점: `manager.rs create()`의 hooks_on env 블록(manager.rs:220-237) 안.
  zsh ZDOTDIR 주입과 동일하게 실패는 `eprintln!` 후 계속(비치명).
- 정리: 정적 파일 1개라 세션별 cleanup 없음. temp dir라 OS가 언젠가 청소.
  (hooks/ 디렉토리 누적 사고 같은 게 구조적으로 불가능)

### 3.2 확장 본문 스케치 (~120줄, 의존성 0)

```ts
// agent-office-pi.ts — pi 프로세스 내에서 jiti로 로드됨 (default export 팩토리,
// loader.js:310-318 실측). pi 패키지에서 타입 import 금지(버전 드리프트 격리) —
// ExtensionAPI는 구조적 타이핑으로만 사용.
export default function agentOffice(pi: any) {
  const url = process.env.AGENT_OFFICE_HOOK_URL;
  const session = process.env.AGENT_OFFICE_SESSION;
  if (!url || !session) return;                    // agent-office 밖: 완전 no-op

  // 사용자가 같은 확장을 -e로 중복 지정해도 이벤트가 2배가 되지 않게 가드
  const g = globalThis as any;
  if (g.__AGENT_OFFICE_PI_HOOKED__) return;
  g.__AGENT_OFFICE_PI_HOOKED__ = true;

  // POST 직렬화 큐: prompt→tool 역전으로 백엔드 at 타임스탬프가 뒤집히는 것 방지
  let chain: Promise<unknown> = Promise.resolve();
  const post = (source: string, body: unknown) => {
    chain = chain.then(() =>
      fetch(`${url}?session=${session}&source=${source}&agent=pi`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body ?? {}),
        signal: AbortSignal.timeout(2000),
      }).catch(() => {})                            // 앱이 내려가 있어도 pi는 무사
    );
  };

  const on = (ev: string, fn: (e: any) => void) => {
    try { pi.on(ev, fn); } catch { /* 미래 pi에서 이벤트가 사라져도 생존 */ }
  };
  on("before_agent_start", (e) => post("prompt", { prompt: e?.prompt ?? "" }));
  on("tool_execution_end", () => post("tool", {}));
  on("agent_end", () => post("stop", { message: "Pi finished a task" }));
  on("session_shutdown", () => post("stop", { message: "Pi session ended" }));
}
```

설계 포인트:
- **타임스탬프는 안 보낸다** — `at`은 hub가 ingest 시점에 찍는다(hub.rs:90).
  localhost라 지연 왜곡은 무시 가능(Claude curl과 동일한 노출).
- **직렬화 큐** — Claude의 curl들은 독립 프로세스라 순서 보장이 없지만, 확장은
  공짜로 순서를 보장할 수 있으므로 한다(3줄).
- **`&agent=pi`** — `HookQuery`(hook_server.rs:30-35)는 session/source만 읽고
  나머지는 serde가 무시하므로 오늘 무해, 후일 CLI 구분이 필요해지면 백엔드만
  읽기 시작하면 된다 (§5).
- **stop 두 종의 메시지 문자열을 다르게** — dedup 키가 메시지를 포함하므로
  (hub.rs:172-177) agent_end 직후 shutdown이 와도 서로를 삼키지 않는다.

### 3.3 셸 래퍼 — `pi()` 함수 추가

`-e`는 **반복 가능 + 탐색된 확장과 additive**(`pi --help` 실측: "can be used
multiple times", `--no-extensions`도 "explicit -e paths still work"). 따라서
claude 래퍼의 `--settings` 이중 주입 가드(zsh_wrapper.rs:47) 같은 스킵 조건이
**불필요하다** — 사용자가 자기 -e를 줘도 충돌하지 않고, 같은 파일을 중복
지정하는 극단 케이스는 확장 내 globalThis 가드가 막는다.

1. **zsh** (zsh_wrapper.rs:41-61 `ZSHRC` 상수): `claude()` 아래에 추가
   ```zsh
   pi() {
     if [[ -n "$AGENT_OFFICE_PI_EXT" ]]; then
       command pi -e "$AGENT_OFFICE_PI_EXT" "$@"
     else
       command pi "$@"
     fi
   }
   ```
2. **bash** (bash_wrapper.rs:17-25 `BASHRC` 상수): 동일 POSIX 형태 추가.
3. **PowerShell** (shells.rs:144-154 `CLAUDE_WRAPPER_PS`): `function pi { ... }`
   추가. `Get-Command pi -CommandType Application,ExternalScript` 패턴 그대로
   미러(재귀 방지 동일 원리). 상수명은 `AGENT_WRAPPER_PS`로 개명 권장(내용이
   claude+pi 둘을 담게 되므로) — 테스트 `assert_ps_wrapper_args`(shells.rs:440-448)
   갱신 동반.
4. **WSL**: 기존과 동일하게 hooks_supported=false(shells.rs:294) — 변경 없음.

주의: env 주입 조건과 래퍼 동작이 자동 정합한다 — hooks OFF면
`AGENT_OFFICE_PI_EXT`가 없어 래퍼가 `command pi`로 폴백하고, git-bash의
`--rcfile` 심 설치 여부도 기존 `hooks_on` 분기(shells.rs:327-339)를 그대로 탄다.

---

## 4. opt-in / 설정

**권장(결정 D2): 기존 `claude_hooks_enabled` 단일 토글에 Pi를 태운다.**

근거: 이 토글은 이미 "훅 인프라 전체"의 스위치다 — 훅 서버 기동(lib.rs:123),
포트 게터(lib.rs:45-46), settings 파일/env/셸 심(manager.rs:180-237)이 전부
여기 묶여 있다. Pi 지원은 그 인프라의 두 번째 소비자일 뿐이다. 토글을 분리하면
설정 스키마 마이그레이션 + UI + "서버는 켜졌는데 pi만 꺼짐" 조합 상태가 생기는
반면 실익(pi만 끄고 싶은 사용자)은 가정적이다.

변경 범위: 설정 UI 문구만 "Claude Code 훅" → "CLI 훅 (Claude Code / Pi)"류로
갱신 (src/renderer/settings/SettingsForm.tsx, FirstRunDialog.tsx). Rust 필드명
`claude_hooks_enabled`은 wire 호환성 유지를 위해 개명하지 않는다(주석만 보강).

대안(사용자가 원하면): `pi_hooks_enabled` 별도 추가 — 이 경우
`AGENT_OFFICE_PI_EXT` env 주입만 추가 게이트하면 된다(래퍼는 env 부재 시 자동
폴백이므로 다른 변경 불필요).

---

## 5. claude vs pi 구분 (라벨/캐릭터/통계)

**권장(결정 D3): v1 무구분.** 이유:

- 앱의 1급 개념은 "에이전트(캐릭터)"이고 세션·활동은 이미 agent_id로 귀속된다.
  같은 에이전트 터미널에서 claude를 쓰든 pi를 쓰든 "그 캐릭터가 일한다"는
  표현은 동일하게 성립한다.
- 알림 문구는 확장이 body에 실은 "Pi finished a task"로 자연 구분된다
  (hub.rs:199-206이 body message 우선).
- 라벨 파이프라인(프롬프트 원문→요약)은 CLI와 무관하게 동작한다.

다만 확장은 첫날부터 `&agent=pi`를 보내므로(§3.2), 후일 다음이 필요해지면
백엔드에서 쿼리 하나 읽는 것으로 켤 수 있다: (a) 에이전트별 통계의 CLI 구분,
(b) 캐릭터 옆 CLI 뱃지, (c) ActivityEvent에 optional `agent` 필드 추가.
이건 제품 결정으로 남긴다 (§8-Q2).

---

## 6. 엣지케이스

| 케이스 | 동작 | 조치 |
|---|---|---|
| pi 미설치 | 래퍼 함수는 정의되지만 `command pi` → command not found (래퍼 없을 때와 동일 메시지) | 없음. 탐지 불필요 |
| 훅 OFF | `AGENT_OFFICE_PI_EXT` 미주입 → 래퍼가 `command pi` 폴백 | 없음 (구조적 정합, §3.3) |
| 중첩 셸 | 중첩 zsh는 사용자 실제 rc를 읽어 래퍼 상실 — claude()와 동일한 기존 한계 (zsh_wrapper.rs:54-60이 ZDOTDIR 복원) | 동일 수용 |
| 사용자가 직접 `-e` | additive라 무충돌. 같은 파일 중복 지정 시 globalThis 가드 | §3.2/3.3 |
| `pi -p`/rpc/json 모드 | 확장은 모든 모드에서 로드 → 헤드리스 실행도 추적됨 (claude 래퍼도 -p 구분 없이 주입하므로 파리티) | S4 스파이크로 확인 |
| ESC/abort 중단 | agent_end 미발화 시 열린 턴은 다음 prompt 또는 세션 종료 settle이 정산 — Claude에서 Stop 미발화와 동일 안전망 | S2 스파이크로 확인 |
| 앱만 종료(훅 서버 다운) | fetch가 2s 타임아웃 후 catch — pi 무영향 (curl `\|\| true` 등가) | §3.2 |
| pi 버전 드리프트 | 확장은 pi 패키지 import 0개 + `pi.on` try/catch — 이벤트가 사라져도 해당 신호만 유실 | §3.2 |
| stop dedup 3s | 같은 메시지 3초 내 반복 시 두 번째 stop 알림 억제 → 턴 정산 지연 가능 — Claude와 동일한 기존 특성 (hub.rs:107-118) | 파리티 유지, 수정 안 함 |
| Windows | PS 래퍼 함수 추가로 커버, `-EncodedCommand` 방식 그대로. curl 이슈는 아예 없음(확장이 fetch 사용) | §3.3 |
| `pi` 이름 충돌(사용자 alias) | 래퍼가 사용자 rc *이후*에 정의되므로 승리 — claude()와 동일 원리 (zsh_wrapper.rs:44-45 주석) | 동일 수용 |

---

## 7. 단계별 구현 플랜 (Fable 주요설계 → Opus 하위설계 → Sonnet 실행)

### Phase 0 — 스파이크: 이벤트 발화 실측 (0.5일, Sonnet 단독 가능)
- 산출물: 로깅 확장(이벤트명+주요 필드를 stderr/파일로 덤프) + 관찰 기록.
  §2.3의 S1~S5를 시나리오별로 실행: 일반 프롬프트, 스트리밍 중 steer/followUp,
  슬래시 커맨드, ESC 중단, `pi -p "hello"`, Ctrl+D 종료.
- 검증: 관찰 기록이 §2.2 매핑과 일치하는지 확인. 불일치 시 이 문서의 매핑 표를
  갱신하고 Phase 3의 확장 본문에 반영 (Fable 재검토 게이트).
- 예시 하네스: `pi -e /tmp/spy.ts -p "1+1은?"` / 대화형 수동.

### Phase 1 — 백엔드: pi_extension.rs + env 주입 (Opus 하위설계 → Sonnet)
- `src-tauri/src/session/pi_extension.rs` 신설: `PI_EXTENSION_TS` 상수,
  `write_extension`/`ensure_extension` (zsh_wrapper.rs:67-81 미러).
- `manager.rs create()`: hooks_on 블록(220-237행)에 `AGENT_OFFICE_PI_EXT` 주입.
- 테스트: write 멱등성/파일 존재(zsh_wrapper 테스트 미러),
  `create_pushes_pi_ext_env_when_hooks_on` / `..._not_when_hooks_off`
  (manager.rs:1703-1732 기존 테스트 패턴 미러). 확장 소스 상수에 대한 정적
  어서션(필수 이벤트 문자열 4종, env 가드, globalThis 가드 포함 여부).
- 검증: `cargo test` (주의: macOS에서 bash_wrapper 1건 기존 실패는 무관).

### Phase 2 — 셸 래퍼 3종 (Opus 하위설계 → Sonnet)
- zsh_wrapper.rs ZSHRC / bash_wrapper.rs BASHRC / shells.rs PS 상수에 `pi()` 추가.
- 테스트: 기존 claude 래퍼 테스트 전부 미러(`zshrc_defines_a_pi_wrapper...`,
  PS `assert_ps_wrapper_args`에 `function pi` 어서션 추가). zsh 통합 테스트
  `real_zsh_resolves_claude_as_a_function...`(zsh_wrapper.rs:179-216)에
  `whence -w pi` 추가.
- 검증: `cargo test` + macOS 실셸 스모크(앱 터미널에서 `whence -w pi`).

### Phase 3 — 확장 본문 확정 (Fable 리뷰 → Sonnet)
- §3.2 스케치를 Phase 0 실측으로 보정해 확정, `PI_EXTENSION_TS`에 임베드.
- 검증: (a) Phase 0 하네스 재사용 — 앱 없이
  `AGENT_OFFICE_HOOK_URL=http://127.0.0.1:9/hook AGENT_OFFICE_SESSION=x pi -e <파일> -p "hi"`
  로 크래시/에러 없음 확인(서버 부재 내성), (b) 로컬 nc/간이 서버로 POST
  쿼리·body 형태 실측.

### Phase 4 — E2E + 설정 문구 (Sonnet)
- 실제 앱에서 pi 세션: 타임패널 working 전환, agent_end 후 idle + Stop 알림
  "Pi finished a task", 라벨에 프롬프트 원문 반영, 오늘 일한 시간 누적 확인.
- 설정 UI 문구 갱신(D2 확정 시): SettingsForm.tsx / FirstRunDialog.tsx +
  해당 테스트.
- 검증: vitest(`--dir src`) + 수동 E2E 체크리스트.

규모 추정: Rust ~250줄(확장 소스 상수 포함) + 래퍼 수정 ~50줄 + 테스트 ~300줄
+ 프런트 문구 소폭. 총 2~4일(스파이크 포함), 다운스트림 무수정이 리스크를
크게 줄인다.

---

## 8. 사용자 확인이 필요한 제품 결정

| # | 질문 | 권장안 |
|---|---|---|
| Q1 | 훅 opt-in 토글: 기존 단일 토글에 Pi 포함 vs `pi_hooks_enabled` 분리? | **단일 토글** (UI 문구만 "CLI 훅"으로 일반화). 분리는 스키마/조합 상태 비용 대비 실익 없음 (§4) |
| Q2 | claude/pi 시각 구분(뱃지/통계 분리)? | **v1 무구분**, 단 확장은 `&agent=pi`를 첫날부터 송신해 후일 확장 여지 확보 (§5) |
| Q3 | Pi 세션의 waiting 상태 부재 수용? | **수용**. Pi 코어에 권한/질문 이벤트가 없어 기술적으로 정직한 표현. 후일 Pi가 이벤트를 추가하면 POST 한 줄로 확장 (§2.1) |
| Q4 | Stop 알림 문구: "Pi finished a task"(영문, Claude 파리티) vs 한국어? | **영문 파리티** — 기존 "Claude finished a task"와 톤 통일 |

> **사용자 결정 확정 (2026-07-12):** Q1=단일 토글, Q2=v1 무구분(`&agent=pi` 송신), Q3=수용, Q4=영문 파리티. 전부 권장안 채택.

---

## 9. Phase 0 스파이크 실측 결과 (2026-07-12, pi v0.80.3)

스파이 확장(`/tmp/pi-spike/spy.ts`, 모든 라이프사이클 이벤트를 파일로 덤프, pi 패키지 import 0)으로
`pi -e spy.ts --no-context-files --no-session -p "..."` 실행. **§2.2 확정 매핑이 전부 실측과 일치 —
매핑 표 수정 없음.** 확장 본문(Phase 3)은 이 결과 위에서 확정한다.

### 관찰된 이벤트 시퀀스 (프롬프트 1개 + 툴 1회 호출, print 모드)
```
session_start(reason:startup) → input(source:interactive, text=프롬프트)
 → before_agent_start(prompt=프롬프트 원문) → agent_start(필드 type뿐)
 → turn_start(turnIndex=0) → [message_start/end, context]
 → tool_execution_start(toolName:bash) → tool_call → tool_result → tool_execution_end(toolName,result,isError)
 → turn_end(turnIndex=0) → turn_start(turnIndex=1) → ... → turn_end(turnIndex=1)
 → agent_end(keys=[type,messages] — message 문자열 없음) → session_shutdown(reason:quit)
```

### 확정 (매핑 검증됨)
- **S4 ✓ print 모드 발화**: 확장은 `-p`에서 로드되고 전 이벤트 발화. 헤드리스도 추적됨 → claude 래퍼와 파리티.
- **`before_agent_start`**: `prompt` 필드 존재 확인 → source=prompt + body `{"prompt":...}` 그대로 채택.
- **`tool_execution_end`**: `toolName`/`result`/`isError` 필드 확인 → source=tool 하트비트로 확정.
- **`agent_end`**: 프롬프트 전체 완료 시 **정확히 1회**, `message` 필드 **없음** → 확장이 body에 "Pi finished a task"를 직접 실어야 함(§2.2 예측대로).
- **turn_start/end는 LLM 호출 1회 단위**로 2회 발화(툴 루프 때문) → `agent_*`(1회) 채택이 옳음을 확증. `agent_start`엔 prompt 없음 → `before_agent_start` 채택이 옳음을 확증.
- **`input`은 before_agent_start보다 먼저** 발화하며 text 보유 → design의 "input 대신 before_agent_start" 근거 유지(followUp 회계).

### 잔여 (비치명 — 매핑 리스크 아님, 기존 안전망으로 커버)
- **S5 부분**: graceful quit → `session_shutdown(reason:quit)` ✓. 그러나 **SIGINT 강제종료(스트리밍 중 kill)에서는 session_shutdown 미발화**. → 열린 턴은 (a) 다음 `before_agent_start`의 close-and-reopen 또는 (b) PTY 세션 종료 settle이 정산 — **Claude에서 Ctrl+C로 Stop 미발화 시와 동일**. 단, 셸은 살아있고 pi만 죽는 경우 PTY settle이 안 걸리므로 열린 턴이 다음 pi 실행까지 working 유지될 수 있음(Claude와 동일 한계). 후속 개선 후보.
- **S3 미결(print 모드 한계)**: `-p "/help"`는 슬래시 커맨드가 아니라 일반 프롬프트로 처리되어 before_agent_start 발화. 슬래시 커맨드의 로컬 처리(before_agent_start 미발화)는 **대화형 TUI 전용** 동작이라 print 모드로 검증 불가 → Phase 4 대화형 E2E에서 수동 확인. 리스크 낮음.
- **S1/S2 미결(자동화 불가)**: steer/followUp 발화 횟수·ESC abort 시 agent_end 발화 여부는 대화형 PTY 필요 → Phase 4 수동 체크리스트로 이관. §2.2 매핑에는 영향 없음.

**결론: Phase 0 목표(유일한 미확정 리스크였던 이벤트 경계) 달성. Phase 1(백엔드) 진행 가능.**
