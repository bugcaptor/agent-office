# 실행 레시피 설계 (kbm #2rf)

상태: 구현 완료. 갱신: 2026-09-04.

프로젝트마다 있는 실행 방법(`npm run dev`, `cargo test`, `make lint` …)을
앱이 기억해 두고, 캐릭터 탭에서 두 번 클릭 안에 돌릴 수 있게 한다. 실행
방법을 알아내는 일은 앱이 하지 않고 **그 프로젝트에서 일하는 캐릭터에게
프롬프트로 맡긴다.** 결과는 앱 데이터에 경로별 파일로 남고, 같은 폴더를 쓰는
캐릭터들은 같은 것을 본다. 기능 전체는 **설정에서 켜야 보인다**(기본 꺼짐, §9).

## 1. 이 기능이 주는 것 — 그리고 안 주는 것

**주는 건 두 가지다.**

- 기억. 프로젝트 폴더마다 실행·테스트·빌드 명령 목록이 있고, 캐릭터가 조사해
  채운다. 사람이 손으로 더할 수도 있다.
- 실행. 목록에서 하나를 고르면 **그 캐릭터의 터미널 세션**에 명령이 들어간다.
  화면도, 작업 중 표시도, 세션 로그도 전부 지금 있는 것 그대로다.

**안 주는 건 "실행 전용 창"과 "조율"이다.** 별도 프로세스 관리자나 로그 패널은
만들지 않는다. dev 서버를 돌리면 그 캐릭터의 셸이 점유되고, 한 저장소에서 여러
캐릭터가 같은 것을 동시에 띄우려 해도 앱은 말리지 않는다. 자동으로 뜨는 것은
없고, 사람이 누른 것만 돈다. 조율은 사람이 한다.

## 2. "프로젝트"는 프로필의 작업 폴더다

이 앱에 프로젝트라는 개체는 없다. 폴더 경로가 그 자리를 이미 채우고 있다.

- 프로필의 `cwd`(`src/shared/types/profile.ts`)가 세션 작업 폴더다.
- 머리 위 라벨의 프로젝트명은 cwd의 basename이고, 세션이 워크트리 하위로
  들어가도 프로필 cwd를 정체성의 기준으로 잡는다(`labels/labelText.ts`
  `projectAnchorCwd`).
- 작업 폴더 팔레트는 `root = cwd`가 키다(`workdir/workdirTypes.ts`).
- 신호등의 프로젝트 모드는 폴더 경로 목록(`AppSettings.mascotLightsProjects`)이다.
- git 루트 계산은 어디에도 없다(`workdir/status.rs`는 브랜치만 본다).

그래서 **레시피의 키는 프로필 cwd를 정규화한 경로**다. 정규화는 틸드
확장(`manager.rs::expand_tilde`) → 후행 구분자 제거 → Windows면 소문자화와
`\` → `/`. 심링크 해소(canonicalize)는 하지 않는다 — 프로필에 적힌 문자열과
파일 이름이 1:1로 대응해야 사람이 찾을 수 있다.

같은 cwd를 쓰는 캐릭터는 같은 레시피를 본다. 한 캐릭터가 조사해 갱신하면 다른
캐릭터의 팔레트에도 그대로 뜬다. 모노레포 하위 폴더를 cwd로 둔 캐릭터는 그
폴더 기준으로 따로 조사·저장된다 — 루트의 스크립트를 억지로 보여 주지 않는다.

## 3. 저장 — 앱 데이터에 경로별 파일 두 개

```
<app_data>/run-recipes/
  agent-office-3f9a2c1b7e04.agent.json   ← 캐릭터가 쓴다. 앱은 읽기만
  agent-office-3f9a2c1b7e04.user.json    ← 앱이 쓴다(손 등록). 캐릭터는 모른다
```

파일 이름은 `<폴더 슬러그>-<정규화 경로 sha1 앞 12자>`다. 슬러그는 tmux
호스팅이 쓰는 `tmux_host::dir_slug`를 그대로 쓴다(사람이 폴더를 열어 봤을 때
어느 저장소 것인지 읽히게). 정체의 근거는 이름이 아니라 **파일 안의 `root`
필드**다 — 이름은 읽으라고 있는 것이고 파싱하라고 만든 형식이 아니다.

### 왜 파일이 둘인가

앱과 캐릭터가 **같은 파일을 쓰지 않게** 하기 위해서다. 캐릭터는 파일을 통째로
다시 쓴다. 그 안에 사람이 손으로 넣은 항목이 섞여 있으면 "이건 지켜라"는
규칙이 프롬프트에 들어가고, 안 지켜졌을 때 원인을 사람이 찾아야 한다. 캐릭터가
`Write` 도구로 쓰는 중에 앱이 손 등록을 저장하면 한쪽이 덮어써지는 것도
막아야 한다. 파일 하나에 주인 하나면 두 문제가 전부 사라진다. 팔레트는 둘을
합쳐 보여 주되 출처 뱃지를 붙인다.

- `*.agent.json`: 앱은 절대 쓰지 않는다. 지우기도 팔레트의 "조사 결과 비우기"
  한 곳에서만 한다.
- `*.user.json`: 앱만 tmp+rename으로 쓴다(`work_log_store.rs` 관례). 프롬프트에
  이 파일은 언급조차 하지 않는다.
- 캐릭터 둘이 같은 `*.agent.json`을 동시에 쓰는 경합은 막지 않는다 — 조사는
  사람이 눌러야 시작되고, 마지막에 쓴 쪽이 이긴다. 프롬프트가 "기존 파일을
  먼저 읽고 갱신하라"고 시키므로 보통은 합쳐진다.

### 왜 `AppSettings`에 레시피를 넣지 않는가

`AppSettings`는 TS·Rust 미러 + 계약 픽스처 + `ctl settings set` 표면까지 걸린
구조체다. 경로 키 맵을 여기 넣으면 설정 저장마다 통째로 왕복하고, 캐릭터가
쓸 수 있는 자리도 아니다. 메모·일기·작업로그처럼 **도메인 파일이 따로 있는
것**이 이 저장소의 관례다. `AppSettings`에는 켜고 끄는 플래그 하나만
들어간다(§9).

## 4. 스키마 — 캐릭터와 맺는 계약

`*.agent.json`:

```json
{
  "version": 1,
  "root": "/Users/me/dev/agent-office",
  "updatedAt": "2026-09-04T10:00:00+09:00",
  "recipes": [
    { "id": "dev",       "label": "개발 서버",     "command": "npm run tauri dev", "longRunning": true },
    { "id": "test-web",  "label": "프런트 테스트", "command": "npx vitest run --dir src" },
    { "id": "test-rust", "label": "Rust 테스트",   "command": "cargo test --manifest-path src-tauri/Cargo.toml",
      "note": "AGENTS.md 기준" }
  ]
}
```

| 필드 | 필수 | 뜻 |
| --- | --- | --- |
| `id` | O | 슬러그. 갱신해도 바뀌지 않는 키 |
| `label` | O | 팔레트에 보이는 이름 |
| `command` | O | 셸 문자열 그대로. 셸 호환은 `startupCommand`와 같은 계약 — 사용자·캐릭터 책임 |
| `cwd` | | 프로젝트 **상대** 하위 폴더. 모노레포용. 실행 시 서브셸로 감싼다(§6) |
| `longRunning` | | dev 서버·watch처럼 끝나지 않는 것. 뱃지 표시용 |
| `note` | | 한 줄 메모(근거 파일 등) |

- 모르는 키는 무시한다(전방 호환). `command` 없는 항목은 그 항목만 버리고
  나머지는 살린다.
- 파일이 없으면 "아직 조사 안 함", JSON이 깨지면 "파일이 깨졌다"로 팔레트에
  **상태**로 표시한다. 에러가 아니다 — `workdir_git_status`가 비저장소를
  `isRepo=false`로 표현하는 것과 같은 원칙이다.
- `*.user.json`은 같은 모양에서 `recipes` 항목이 `id`·`label`·`command`·
  `createdAt`만 갖는다. 앱이 만들고 앱이 읽으니 관대할 이유가 없다.

`lastRunAt`은 두 파일 어디에도 쓰지 않는다. 캐릭터 파일은 앱이 못 쓰고, 그
하나 때문에 세 번째 파일을 두는 것은 값을 안 한다. 실행 시각은 런타임 스토어에만
있다.

## 5. 조사 프롬프트

팔레트의 **"실행 방법 조사시키기"**를 누르면 그 캐릭터의 세션에 프롬프트가
들어간다. 파일이 아직 없으면 이 버튼이 팔레트의 첫 화면이다.

### 5.1 앱이 계산해 박는 것

- 프로젝트 폴더 절대 경로(정규화된 cwd).
- 써야 할 파일의 **절대 경로**(`<app_data>/run-recipes/<이름>.agent.json`).
  세션 env에 `AGENT_OFFICE_APP_DATA`가 있지만(`manager.rs`의 env 주입) 캐릭터에게
  env를 조합하라고 시키지 않는다 — 앱이 아는 것을 캐릭터에게 계산시키면 틀릴
  자리만 늘어난다. 프롬프트를 넣기 전에 앱이 `run-recipes/` 폴더를 만들어 둔다.
- 스키마(§4 그대로).

### 5.2 내용 요지

한국어(요지 — 정확한 문장은 프롬프트 프로필이 정본):

> 이 프로젝트(`<root>`)의 실행·테스트·빌드 방법을 조사해서 `<file>`에 아래
> 형식의 JSON으로 정리해 줘. 근거는 `package.json` scripts(패키지 매니저는
> 락파일로 판단), `Cargo.toml`, `Makefile`/`justfile`, `pyproject.toml`, CI
> 워크플로, 그리고 `AGENTS.md`·`README`에 사람이 적어 둔 명령 — 사람이 적어 둔
> 것이 있으면 그걸 우선해. 그 파일이 이미 있으면 먼저 읽고, 여전히 유효한
> 항목은 `id`를 그대로 두고, 없어진 것은 지우고, 새 것은 더해. 오래 도는
> 것(dev 서버, watch)은 `longRunning: true`. 명령을 실제로 실행하지는 말고
> 읽기만 해. 파괴적인 것(clean, reset, deploy, publish)은 넣지 마. 끝나면
> 몇 개를 적었는지 한 줄만 답해.

영어는 같은 항목을 담되 ko의 번역이 아니라 **영어에서 원하는 출력이 나오게 따로
쓴다**(`promptProfiles.ts` 헤더의 원칙). 담을 것: 대상 폴더와 파일 경로, 근거
파일 목록과 "사람이 적은 문서 우선", 기존 파일 읽고 `id` 보존, `longRunning`,
실행 금지·파괴적 명령 제외, 한 줄 보고.

### 5.3 언어와 위치

주입은 렌더러에서 하므로 프롬프트는 `src/renderer/i18n/promptProfiles.ts`에 새
프로필(`runRecipeProbe`)로 둔다. ko·en 두 개, 없는 언어는 그 파일의 규칙대로
en 폴백. 카탈로그 JSON에 넣지 않는 이유도 그 파일 헤더에 적힌 그대로다 —
모델 입력이라 "번역이 정확한가"가 아니라 "그 언어에서 원하는 출력이 나오는가"로
평가한다.

### 5.4 주입 방식

봇과 같은 경로다(`bot/runner.rs::inject`). 여러 줄을 한 줄로 접어
`writeInput(텍스트)` → `INJECT_SUBMIT_DELAY_MS` 대기 → `writeInput("\r")`. 두
번에 나누는 이유는 한 번에 보내면 Enter가 입력 이벤트에 섞여 무시되기
때문이고, 줄 끝이 CR인 이유는 PowerShell이 CR에서만 제출하기 때문이다
(`manager.rs`의 startup_command 주석).

### 5.5 캐릭터 CLI가 안 떠 있으면

**가드는 없다.** 프롬프트는 그대로 셸에 들어가고, 셸은 첫 낱말을 명령으로
해석해 실행에 실패한다(`zsh: command not found: 이`). 사람이 CLI를 먼저 띄우고
다시 누른다. 봇 시작에는 맨 셸 가드(`botGuard.ts::looksLikeAgentRunning`)가
있지만 여기서는 쓰지 않는다 — 봇은 사람이 안 보는 동안 도는 것이고, 이 버튼은
사람이 화면을 보며 누르는 것이다.

### 5.6 결과 반영

팔레트를 열 때마다 두 파일을 다시 읽고, 팔레트가 열린 채로 그 캐릭터의 정산된
턴 수(`appStore.timeTracking[agentId].turns`)가 늘면 한 번 더 읽는다. 이 값은
`notification-new`의 `source="stop"`과 셸의 `activity-event idle`에서 늘어난다.
파일 감시(watcher)는 두지 않는다 — 읽기는 작은 JSON 두 개라 싸고, 감시는
플랫폼별 예외가 딸려 온다.

## 6. 실행 — 그 캐릭터의 세션에 넣는다

레시피의 [실행]은 새 백엔드 커맨드 없이 렌더러에서 끝난다.

| 세션 상태 | 동작 | 근거 |
| --- | --- | --- |
| `idle` / `exited` | `createSession(agentId, { …프로필 옵션, startupCommand: cmd })` | 이미 있는 경로(`CreateSessionOptions.startupCommand`, `ipc/sessionOpts.ts`). tmux 호스팅이면 `send-keys`로 pane에 들어가는 것까지 따라온다 |
| `running` | `writeInput(cmd)` → 대기 → `writeInput("\r")` | §5.4와 같은 이유 |
| `starting` | 실행하지 않고 잠시 뒤 다시 누르라고 알린다 | 시작 중인 입력과 섞지 않는다 |
| 외부 논리 세션 | 실행하지 않고 입력 불가를 알린다 | 앱이 소유한 PTY가 없어 `writeInput`할 곳이 없다 |

`running`일 때 셸이 프롬프트에 있든, 에이전트 CLI가 떠 있든, 다른 명령이 돌고
있든 **구분하지 않는다.** CLI가 떠 있으면 명령 문자열이 CLI의 입력창에 들어가고
그 CLI가 알아서 한다(claude는 대개 그 명령을 실행해 준다). 다른 명령이 돌고
있으면 그 명령의 stdin으로 들어간다. 이게 맞는지 보는 것은 화면을 보고 있는
사람의 몫이다. 팔레트 위쪽에 지금 셸에서 도는 명령(`shell_activity`가 라벨에
넣은 것)을 보여 주는 이유가 이것이다 — 누르기 전에 보라고.

`cwd`가 있는 레시피는 POSIX/Git Bash/WSL에서
`( cd '<root>/<cwd>' && <command> )`로 감싼다. PowerShell은
`Push-Location` → `try` → `finally { Pop-Location }`으로 감싼다. 명령이 끝난 뒤
캐릭터 셸의 위치가 바뀌지 않아야 한다.

[중단]은 `writeInput("\x03")`이다. 사람이 Ctrl-C를 치는 것과 같은 경로라 따로
구분할 것이 없다.

### 왜 이게 맞는가

새로 만드는 인프라가 없다. 세션 로그(전사 기록), 작업 중 표시
(`shell_activity.rs` — 헤더에 `npm test`·`cargo build`가 사례로 적혀 있다),
알림, 웹 원격 미러링, tmux 호스팅, 종료 시 핸드오프가 전부 그대로 적용된다.
별도 실행 세션을 만들면 이 목록 하나하나에 "실행 세션은?"이라는 예외를 달아야
한다(§11).

## 7. UI

- **진입**: 탭 우클릭 메뉴(`terminal/AgentTabStrip.tsx`)의 "열기/보기" 그룹에
  **"실행…"** 항목 하나. 설정이 꺼져 있으면 **항목 자체가 없다**(disabled가
  아니라 배열에서 빠진다 — `ContextMenu`는 빈 구분선을 스스로 정리한다). 켜져
  있어도 `cwd` 없는 프로필은 비활성(`menu.workdir`과 같은 규칙). 문구는
  `shared/i18n/locales/{ko,en,fr,ja,zh-Hans,zh-Hant}/terminal.json`의 `menu.*`.
- **팔레트**: 작업 폴더 팔레트와 같은 오버레이 계층(z-index 40), 독립 zustand
  스토어 `runStore`(`workdirStore`와 같은 비커플링 관례). 위에서부터:
  1. 현재 작업 한 줄 + [중단] (없으면 비움). 별도 셸 명령 필드가 없으므로
     `taskLabels`의 최근 프롬프트·도구·응답을 쓴다. 셸 명령과 캐릭터 입력이
     섞이는 기존 한계가 있어 문구도 "현재 작업"으로 둔다.
  2. 조사 결과(`*.agent.json`) — 출처 뱃지 "조사", `longRunning` 뱃지. 파일
     없음/깨짐은 이 자리에 상태 문구
  3. 손 등록(`*.user.json`) — 출처 뱃지 "직접", 행마다 [삭제]
  4. 손 입력 한 줄 + [추가]
  5. 하단 [실행 방법 조사시키기] · [새로고침] · [조사 결과 비우기]
- 행 클릭 = 실행. 확인 모달은 없다(§6 — 판단은 사람이 화면을 보고 한다).
- 단축키는 없다. 오버레이가 `Cmd+1..9`·`Cmd+W`를 이미 잡고 있고
  (`subsystem-c-ui.md §13.5`) `Cmd+R`류는 안의 TUI가 쓰는 키일 수 있다.

## 8. 장기 실행 프로세스의 수명

전부 기존 것이다.

| 상황 | 동작 | 근거 |
| --- | --- | --- |
| 상태 표시 | 캐릭터 작업 중 + 라벨에 명령 | `shell_activity.rs` |
| 중지 | 팔레트 [중단] → `\x03` | 사람이 Ctrl-C 치는 것과 같음 |
| 앱 종료 | 기존 종료 모달("모두 종료 / 유지하고 종료") | `lib.rs` `ExitRequested` → `dispose_all`, sessiond·tmux 존속 |
| 앱 크래시 | sessiond/tmux가 살린다 | 기존 설계 |

## 9. 설정에서 켜야 보인다

`AppSettings.runRecipesEnabled: boolean`, **기본 `false`**. 꺼져 있으면 컨텍스트
메뉴 항목이 없고, 팔레트도 열리지 않으며, 백엔드 커맨드는 그대로 있되 부를
진입점이 없다. 설정 파일·`run-recipes/` 폴더는 건드리지 않는다 — 끄는 것은
숨기는 것이지 지우는 것이 아니다.

### 왜 opt-in인가

이 기능은 캐릭터 세션의 stdin에 글자를 넣는다(프롬프트도, 명령도). 그 자체가
새 능력은 아니지만(사람이 치는 것과 같은 경로), "메뉴에 있는 줄 몰랐던 버튼이
셸에 뭔가를 넣었다"는 놀람은 피해야 한다. 웹 원격·동료 대화·CLI 제어가 전부
기본 꺼짐인 것과 같은 이유다. 다만 2단계 승인(토큰)까지는 두지 않는다 —
누르는 사람이 화면 앞에 있고, 들어가는 문자열이 화면에 그대로 보인다.

### 플래그 하나를 더하면 닿는 곳 — 전부 같이 고친다

`talkEnabled`가 지나간 자리를 그대로 따라간다. **`npx tsc --noEmit`이 누락을
잡는 유일한 장치**다(AGENTS.md — vitest는 리터럴끼리 `toEqual`이라 양쪽에서
같이 빠져도 통과한다).

| 자리 | 할 일 |
| --- | --- |
| `src/shared/types/settings.ts` `AppSettings` | 필드 + 주석("기본 false, 켜면 탭 메뉴에 실행… 항목") |
| `src-tauri/src/persistence/settings_store.rs` `AppSettings` | `#[serde(default)] pub run_recipes_enabled: bool` + `Default`에 `false` — 구버전 `settings.json`에 키가 없어도 읽힌다 |
| `src-tauri/src/lib.rs`, `ipc/commands/tests.rs`, `tests/contract_fixtures.rs`의 리터럴 구조체 | 필드 추가(컴파일러가 잡는다) |
| `src/shared/contract-fixtures/get-app-settings-result.json` | `"runRecipesEnabled": false` — TS↔Rust 왕복 픽스처 |
| `src/renderer/store/appStore.ts` | 초기값 `false` + `updateAppSettings`의 키 유니언에 추가 |
| `src/renderer/settings/SystemTab.tsx` | `keepAwakeEnabled`·`sessionLogEnabled` 옆에 같은 모양의 체크박스 한 칸. 제어 탭이 아니라 시스템 탭인 이유: 승인·토큰·네트워크가 없는 단순 토글이라 제어 탭의 세 섹션(CLI 제어·웹 원격·동료 대화)과 성격이 다르다 |
| `src/shared/i18n/locales/{ko,en}/settings.json` | `system.runRecipesTitle` / `system.runRecipesHelp` |
| `src-tauri/src/control/routes.rs` `settings_set` | **막지 않는다.** `cliEnabled`·`talkEnabled`를 거절하는 이유는 에이전트가 자기 권한을 넓힐 수 있어서인데, 이 플래그는 켜도 사람이 메뉴를 눌러야 무언가가 일어난다. 거절 목록에 넣지 않는 것이 결정이다 |

## 10. 비범위

- 앱이 `package.json`·`Cargo.toml`·`Makefile`을 직접 파싱하는 자동 발견.
- `ctl run …` 서브커맨드와 `run` 전용 shim·스킬·권한 조각.
- 프로젝트 안 파일(`.agent-office/run.json`, `AGENTS.md` 섹션) 읽기·쓰기.
- 별도 실행 세션·로그 패널·프로세스 관리자.
- 맨 셸 가드, 실행 전 확인 모달, 에이전트 CLI 판정.
- 여러 캐릭터가 한 저장소를 쓸 때의 조율, 자동 실행.
- 맨 셸에서 `claude -p`/`codex exec` 원샷 조사.
- 인자·환경변수 편집 폼, 실행 결과 판정·알림, 실행 이력 통계.
- 세션 시작 시 자동 실행(그건 `startupCommand`가 한다).
- 웹 원격·`ctl` 노출, 레시피 내보내기/가져오기, 파일 감시.
- 설정 토글에 2단계 승인 붙이기.

## 11. 기각한 대안들

- **앱이 파일을 파싱하는 자동 발견**(`package.json` scripts·`Cargo.toml`·
  `Makefile` 타깃): 구현은 싸지만 정확도가 낮다. `npm run tauri dev`가 dev
  서버라는 것, `Makefile`의 어느 타깃이 사람이 실제로 쓰는 것인지, `AGENTS.md`에
  적힌 "이 명령으로 테스트하라"는 파서가 모른다. 그 프로젝트에서 일하는 캐릭터는
  안다. 소스가 늘 때마다(justfile, uv, gradle …) 파서를 더 짜야 하는 것도
  캐릭터에게 맡기면 사라진다.
- **프로젝트 안 파일 `.agent-office/run.json`**: 캐릭터가 cwd 안에 쓰니 어떤
  CLI의 샌드박스에서도 권한 문제가 없고 팀과 공유된다. 그러나 남의 저장소에
  이 앱 전용 파일이 생기고, "실행" 한 번에 저장소가 dirty가 되며, 커밋할지
  `.gitignore`에 넣을지를 프로젝트마다 정해야 한다. 이 앱은 메모·일기·작업로그
  전부 앱 데이터에 두고 저장소를 건드리지 않는 쪽으로 일관돼 왔다. 그 관례를
  깨지 않는다. `AGENTS.md`의 섹션을 파싱하는 변형은 사람 문서와 기계 계약이 한
  파일에 섞여 표 한 칸만 흔들려도 팔레트가 빈다는 점에서 더 나쁘다.
- **`ctl run set/add`로 캐릭터가 앱에 직접 쓰기**: 되지만 전제가 셋이다.
  (1) control 서버는 `cli_enabled`일 때만 뜨고 `auth` 미들웨어가 **모든**
  라우트에 걸려 있어(`control/mod.rs::router`, `control/token.rs::auth`) 사용자가
  CLI 제어를 켜고 승인해야 한다 — 기본 OFF인 보안 표면을 레시피 하나 때문에
  열라는 요구다. (2) 바이너리가 PATH에 없어(macOS는 앱 번들 안) talk처럼 OS
  temp에 shim을 만들고 절대 경로를 박아야 한다(`talk/skill.rs`). (3) 권한 사전
  승인과 스킬 주입은 claude에만 배선돼 있고(`manager.rs`의 `--plugin-dir`,
  `docs/agent-talk-design.md §6`) codex·pi·gemini는 매번 승인이다. 파일 하나
  쓰는 일에 이 셋을 다 들일 이유가 없다. 파일 방식이 흔들리면 그때 검증을
  앱으로 옮기는 길은 남아 있고, 필요한 발견·인증·shim은 전부 talk가 만들어 둔
  것을 쓴다.
- **별도 실행 세션(캐릭터당 PTY 하나 더) 또는 PTY 없는 프로세스 + 로그 패널**:
  `SessionManager.sessions`가 `agentId` 키, `SessionRegistry`가 sid→agentId,
  `TerminalRegistry`가 에이전트당 xterm 하나 — **1캐릭터 1세션**이 세 층에 걸린
  불변식이다(`subsystem-a-sessions.md §3.3`). 알림 라우팅·세션 로그·핸드오프에
  전부 예외 분기가 생긴다. PTY 없는 쪽은 `proc_runner`가 단발·stdout만·stderr
  버림이라 장기 실행·색·대화형이 안 되고, 결국 새 프로세스 관리자와 새 패널을
  짜게 된다. dev 서버가 캐릭터 셸을 점유하는 것을 받아들이면 이 비용이 통째로
  사라진다.
- **파일 하나에 `discovered`/`manual` 두 배열**: 앱과 캐릭터가 같은 파일을
  통째로 다시 쓰게 되어 한쪽이 덮어써진다. 파일을 둘로 나눴다(§3).
- **`lastRunAt`을 파일에 기록**: 캐릭터 파일은 앱이 못 쓰고, 그 하나 때문에
  세 번째 파일을 두는 건 값을 안 한다. 런타임에만 둔다.
- **설정 토글을 제어 탭에 두기 / `ctl settings set`에서 거절하기**: 승인·토큰·
  네트워크가 없는 단순 토글이라 제어 탭의 세 섹션과 성격이 다르고, 에이전트가
  켜 봐도 사람이 메뉴를 눌러야 무언가가 일어나므로 권한 상승이 아니다(§9).

## 12. 알려진 한계

1. **캐릭터가 앱 데이터 폴더에 쓸 때 권한을 물을 수 있다.** claude는 cwd 밖
   `Write`를 처음 한 번 승인받고, codex의 기본 샌드박스(workspace-write)는 cwd
   밖 쓰기를 막아 승인 요청이 뜨거나 실패한다. 사람이 승인한다. 승인 없이
   실패하면 팔레트는 "아직 조사 안 함" 그대로다 — 조용히 깨지지는 않는다.
2. **tmux 호스팅 세션은 작업 중 감지가 안 뜬다**(`tmux-hosting-design.md §12-1`).
   팔레트의 "도는 명령" 줄이 빈다.
3. 캐릭터 둘이 같은 저장소를 동시에 조사하면 마지막에 쓴 쪽이 이긴다(§3).
4. 세션이 `running`이면 무엇이 포그라운드든 명령을 넣는다(§6). CLI 입력창에
   들어가거나 다른 명령의 stdin에 들어갈 수 있다. 의도된 동작이다.
5. 정규화가 심링크를 풀지 않으므로 같은 폴더를 다른 경로 문자열로 적은 두
   프로필은 다른 프로젝트로 취급된다.
6. 설정을 끄면 진입점만 사라진다. 이미 돌고 있는 명령은 그대로 돈다(그건 그
   캐릭터의 셸에서 사람이 시킨 것과 같은 상태다).

## 13. 구현 맵

| 영역 | 파일 |
| --- | --- |
| 설정 플래그(§9의 표 전부) | `shared/types/settings.ts`, `persistence/settings_store.rs`, `lib.rs`·`ipc/commands/tests.rs`·`tests/contract_fixtures.rs` 리터럴, `shared/contract-fixtures/get-app-settings-result.json`, `renderer/store/appStore.ts`, `renderer/settings/SystemTab.tsx`, `locales/{ko,en}/settings.json` |
| 경로 정규화·파일 이름·파서(순수, 픽스처 테스트: 없음/깨짐/일부 불량) | `src-tauri/src/run_recipes/{mod,paths,agent_file}.rs` |
| 손 등록 저장소(tmp+rename) | `src-tauri/src/run_recipes/user_store.rs` |
| 커맨드 | `src-tauri/src/run_recipes/commands.rs` — `run_recipes_read(root)`(두 파일 합본 + 상태), `run_recipes_user_save(root, recipes)`, `run_recipes_agent_clear(root)`, `run_recipes_probe_target(root)`(프롬프트에 박을 파일 경로 계산 + 폴더 생성) |
| 계약 타입 | `src/shared/types/run.ts`, `src-tauri/src/types.rs` 미러, `shared/__tests__/contract.test.ts` 픽스처 |
| 프롬프트 프로필(ko/en) | `src/renderer/i18n/promptProfiles.ts` |
| 스토어·실행 판정·주입 | `src/renderer/run/runStore.ts`, `run/execute.ts`(세션 상태 분기, 서브셸 감싸기, 두 번 쓰기) |
| UI | `src/renderer/run/RunPalette.tsx`, `run/run.css`, `terminal/AgentTabStrip.tsx` 메뉴 항목(`runRecipesEnabled`로 조건부 포함) |
| 문구 | `src/shared/i18n/locales/{ko,en,fr,ja,zh-Hans,zh-Hant}/terminal.json`(`menu.run`), `…/run.json` 신설 |
| 등록 | `src-tauri/src/lib.rs` `generate_handler!`, `src/shared/ipc.ts`, `renderer/ipc/tauriApi.ts` |
