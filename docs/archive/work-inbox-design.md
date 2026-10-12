# 업무 수신함 — 구현 계획

에이전트나 외부 프로그램이 업무 패널에 태스크를 남기는 입력 규약(수신함 폴더)과, 모든 에이전트 세션에 그 규약을 쓰는 방법(`office-task` 명령 + 시스템 프롬프트에 덧붙이는 공통 안내문)을 제공하는 계획이다.

상태: 이력(archived, 2026-10-13) — 구현 완료. 현재 규약·구현 위치·실기 인수는 [업무 패널](../work-panels-design.md#업무-수신함)이 정본이다. 구현 중 바뀐 점: Pi는 확장 대신 `--append-system-prompt` 플래그, Kilo는 `experimental.chat.system.transform` 훅, agy는 env 폴백. Codex `-c writable_roots`가 사용자 배열을 덮어씀을 실측했고 `-p` 프로필 레이어는 합치지 않는다.

관련 문서: [업무 패널](../work-panels-design.md)(업무 목록·저장 규약), [동료 대화](../agent-talk-design.md) §6(shim·권한 사전 승인 배선), [IDE 페르소나](../ide-persona-design.md)(persona 덧붙이기 채널), [CLI 제어](../cli-control-design.md)(`maybe_run_cli` 분기).

## 목표와 범위

- **입력 규약**: 앱 데이터 폴더의 `work-inbox/`에 정해진 형식의 파일을 넣으면 업무 목록에 할 일로 들어간다. 누가 넣었는지는 따지지 않는다. 에이전트든 cron이든 별개 프로그램이든 같은 규약을 쓴다.
- **에이전트 도구**: 앱이 띄운 세션의 에이전트가 다음 태스크를 남길 수 있도록 `office-task` 명령을 주고, 사용법을 담은 **공통 안내문을 모든 CLI의 시스템 프롬프트에 덧붙인다.** CLI별 스킬은 만들지 않는다. 기본 담당은 남긴 에이전트 자신의 캐릭터다.
- **받아들이기**: 렌더러가 수신함을 주기적으로 확인해 업무 스토어에 넣고 기존 저장 규약으로 저장한 뒤 파일을 지운다.

범위 밖:

- 자동 보내기(실행). 가져온 업무는 항상 `todo`이고 보내기는 지금처럼 사람이 누른다.
- 발신자 인증·사칭 방지. 같은 사용자 권한의 프로세스는 누구나 태스크를 넣을 수 있다. 할 일 한 줄이 추가될 뿐 실행되지 않으므로 받아들인다.
- 기존 업무의 수정·완료 처리, 출처 표시(후속 과제 참고).
- CLI별 스킬. 사용자가 직접 치는 `/task` 명령은 없다. "다음 할 일로 남겨 줘"라고 말하면 안내문에 따라 에이전트가 처리한다.
- 웹 원격·데스크톱 원격·마스코트. 업무 패널과 같이 로컬 데스크톱에서만 처리한다.

## 수신함 규약

### 위치

- `<app_data>/work-inbox/`. 앱이 시작할 때 만든다.
- 앱이 띄운 세션에는 `AGENT_OFFICE_WORK_INBOX`로 절대 경로를 알려준다.
- 외부 프로그램용으로는 사용 안내([guide.md](../guide.md))에 OS별 앱 데이터 경로를 적는다.

### 파일 형식

- 파일 하나가 업무 하나다. 확장자는 `.md`, 인코딩은 UTF-8(BOM 허용)이다. 줄바꿈은 LF/CRLF 모두 받아 LF로 바꾼다.
- 선택 frontmatter: 파일 첫 줄이 정확히 `---`이면 다음 `---` 줄까지를 `key: value` 줄로 읽는다. YAML 전체를 지원하지 않는다. 값 앞뒤 공백은 무시한다.
- frontmatter 다음(없으면 파일 처음)의 첫 비어 있지 않은 줄이 제목이다. 앞의 `# `는 떼어 낸다. 나머지 줄이 본문이고 앞뒤 빈 줄은 다듬는다.
- 제목만 있고 본문이 없어도 된다. 둘 다 비면 거부한다.

```markdown
---
assignee: 하나
---
# 로그인 화면 회귀 테스트 추가
세션 만료 뒤 재로그인 흐름이 테스트에 없다.
`src/auth/` 수정 범위만 다룬다.
```

| frontmatter 키 | 내용 |
| --- | --- |
| `assignee` | 담당 캐릭터. 프로필 id 또는 이름. 생략하면 미배정 |
| 그 외 | 무시한다(향후 확장 예약). 거부 사유가 아니다 |

### 담당 판정

렌더러가 가져오는 시점의 프로필 목록으로 판정한다.

1. 프로필 id와 정확히 일치하면 그 캐릭터.
2. 아니면 이름 일치(앞뒤 공백 무시, 대소문자 구분). 일치하는 프로필이 정확히 하나일 때만 배정한다.
3. 없거나 여럿이면 **미배정으로 받는다.** 배정만 틀렸을 뿐 내용은 유효하므로 거부하지 않는다. 프로필 이름에 고유 제약이 없으므로 모호할 때 추측하지 않는다.

에이전트·모델 지정(`agent`)은 비워 둔다. 보낼 때 담당 캐릭터의 `defaultAgent`를 따른다.

### 쓰기 규약 (생산자 쪽)

- 다른 이름으로 끝까지 쓴 뒤 같은 폴더 안에서 `.md`로 rename한다. 앱은 `.`으로 시작하는 파일과 `.md`가 아닌 파일을 읽지 않는다. 권장 임시 이름은 `.<최종 이름>.tmp`다.
- 파일 이름은 겹치지 않게 짓는다. 가져오는 순서는 파일 이름순이므로 `20261012T091500Z-<임의값>.md`처럼 시각을 앞에 두기를 권장한다.
- 파일 크기 상한은 64 KiB다.

### 처리 규약 (앱 쪽)

- **확인 주기**: 업무 패널이 켜져 있고 업무 목록 로드에 성공한 동안 3초마다 확인한다. 한 번에 최대 50개를 처리한다.
- **가져오기**: 이름순으로 `todo` 묶음의 끝에 추가한다. 선택·편집 초안·패널 접힘 상태는 바꾸지 않는다.
- **삭제 시점**: 업무 스냅샷 flush가 성공한 뒤에만 원본 파일을 지운다. 저장에 실패하면 파일을 남겨 두고 다음 주기에 다시 시도한다.
- **중복 방지**: 업무 id를 `inbox-` + (파일 이름과 내용의 해시 앞 16자)로 정한다. 같은 id의 업무가 이미 있으면 추가하지 않고 파일만 지운다. flush 성공과 삭제 사이에 앱이 죽어도 같은 태스크가 두 번 생기지 않는다. 사용자가 그 사이 업무를 지웠다면 다시 생기지만 받아들일 만한 경우로 본다.
- **거부**: UTF-8이 아니거나, 64 KiB를 넘거나, frontmatter가 닫히지 않았거나, 제목과 본문이 모두 비면 `work-inbox/rejected/`로 옮기고 같은 이름에 `.error`를 붙인 파일에 사유 코드 한 줄을 남긴다. `rejected/`는 앱이 자동으로 비우지 않는다.
- **꺼져 있을 때**: 업무 패널이 꺼져 있거나 앱이 실행 중이 아니면 파일은 그대로 쌓이고, 켜지면 순서대로 들어간다.
- **잔여 임시 파일**: 수정된 지 24시간이 지난 `.`으로 시작하는 `.tmp` 파일은 확인할 때 지운다.

## 에이전트 도구

### `office-task` 명령

동료 대화의 `office-talk`처럼 OS temp의 공백 없는 경로(`<temp>/agent-office/bin/office-task`, Windows는 `.cmd`)에 shim을 둔다. shim은 앱 바이너리의 오프라인 서브커맨드 `agent-office task`를 실행한다. control 서버와 토큰이 필요 없으므로 CLI 제어가 꺼져 있어도 동작한다.

```
office-task [--to <이름|id> | --unassigned] <제목> [본문 | -]
```

- 본문 자리에 `-`를 주면 stdin에서 읽는다. 여러 줄 본문은 heredoc으로 넘긴다.
- **수신함 경로**: `AGENT_OFFICE_WORK_INBOX` → `AGENT_OFFICE_APP_DATA/work-inbox` 순서로 찾고, 둘 다 없으면 비영 종료한다.
- **담당**: `--to`가 있으면 그 값, `--unassigned`면 비움, 둘 다 없으면 `AGENT_OFFICE_AGENT_ID`(세션 캐릭터의 프로필 id)를 `assignee`에 쓴다. 세션 밖에서 실행해 그 변수가 없으면 미배정으로 쓰고 stderr에 알린다.
- 쓰기 규약대로 임시 파일 → rename으로 기록하고 최종 경로를 출력한다. "수신함에 넣었다"까지만 보장하며, 업무 목록 반영(앱 실행·패널 켜짐 여부)은 확인하지 않는다.
- 제목·본문이 모두 비거나 64 KiB를 넘으면 쓰지 않고 비영 종료한다.

### 세션 환경변수

`SessionManager::prepare_session_plan`이 세션 셸에 다음을 추가한다(`AGENT_OFFICE_SESSION`·`AGENT_OFFICE_APP_DATA` 옆). 업무 패널 설정과 무관하게 항상 싣는다. 수신함 규약은 패널이 꺼져 있어도 유효하기 때문이다.

| 변수 | 값 |
| --- | --- |
| `AGENT_OFFICE_AGENT_ID` | 세션 캐릭터의 프로필 id |
| `AGENT_OFFICE_WORK_INBOX` | 수신함 절대 경로 |
| `AGENT_OFFICE_TASK` | `office-task` shim 절대 경로. 안내문을 덧붙일 채널이 없는 CLI에서 사용법을 알려줄 때 쓴다 |

- `prepare_session_plan`에 agent id 인자를 추가한다. 외부 터미널 attach 경로(`external.rs`)도 같은 함수를 쓰므로 함께 넘긴다.
- TTS 확인 문구용 claude 자식 프로세스는 세션이 아니므로 `tts/cli.rs`의 `STRIPPED_ENV`에 세 변수를 추가한다.

### 공통 안내문 주입

스킬 대신 짧은 안내문(약 100~200토큰)을 모든 세션의 시스템 프롬프트에 **덧붙인다.** CLI마다 스킬 형식과 설치 위치가 달라도 안내문 하나를 공유하고, 기본 지침을 덮어쓰지 않는다.

**안내문 원본**: `src-tauri/src/office_instructions.rs`(신규)의 상수 하나다. `office-task`의 절대 경로를 끼워 넣어 만든다. 담을 내용은 다음과 같다.

- 사용자가 나중에 할 일로 남기라고 했거나, 이번 작업 범위 밖의 후속 작업을 발견했을 때 Agent Office 업무 목록에 태스크를 남길 수 있다는 점
- 사용법: `<shim> "제목" - <<'EOF'` … `EOF`
- 기본 담당은 본인이고 다른 동료에게 넘길 때만 `--to`를 쓴다는 점
- 지금 턴에 끝낼 일은 남기지 않고, 후속 작업 하나에 태스크 하나만 남긴다는 점
- 다른 사람이 읽어도 이해되도록 본문에 맥락(파일·이유)을 쓴다는 점
- 남긴 뒤 사용자에게 한 줄로 알린다는 점

**주입 조건**: 업무 패널(`workPanelsEnabled`)이 켜졌을 때만 넣는다. 세션을 띄울 때 고정되므로, 실행 중에 설정을 바꿔도 이미 떠 있는 세션은 그대로다. 수신함 규약과 환경변수는 패널 설정과 무관하게 유지한다.

**persona와 합치기**: 지금은 `prepare_session_plan`이 persona가 비어 있지 않을 때만 덧붙이기 인자를 단다(`manager.rs`의 `AGENT_OFFICE_PERSONA`). 이를 "덧붙일 지시문 = persona + 빈 줄 + 사무실 안내문"(있는 것만)으로 바꿔, 둘 중 하나라도 있으면 주입한다. 환경변수 이름 `AGENT_OFFICE_PERSONA`는 그대로 둔다. 래퍼 함수 호환과 IDE 페르소나 경로 때문이다. Codex의 `developer_instructions`는 값이 하나뿐이라 합치기가 필수다. Claude도 같은 값 하나로 넣는다.

| CLI | 덧붙이기 채널 | 상태 |
| --- | --- | --- |
| Claude | `--append-system-prompt` | 이미 persona에 사용 중. 합친 값으로 교체 |
| Codex | `-c developer_instructions="…"` (`codex_persona_config`) | 이미 사용 중. 합친 값으로 교체 |
| Pi | 앱 소유 확장(`pi_extension.rs`)의 에이전트 시작 이벤트에서 시스템 프롬프트에 덧붙이거나 CLI 플래그를 쓴다 | 지원하는 훅·플래그 실측 필요 |
| Kilo | 앱 소유 플러그인(`kilo_plugin.rs`)의 시스템 프롬프트 변환 훅 | 지원 훅 실측 필요 |
| agy | 미확인 | 실측 필요. 채널이 없으면 환경변수만 제공 |

- Pi 확장과 Kilo 플러그인은 지금 관찰(observer)이 켜졌을 때만 실린다. Kilo는 사용자가 자기 `KILO_CONFIG`를 쓰고 있으면 아예 싣지 않는다. 안내문 주입 때문에 관찰이 꺼진 세션에도 싣게 할지, 관찰과 무관한 별도 플래그 경로를 쓸지는 실측 결과를 보고 정한다.
- 채널이 없는 CLI는 `AGENT_OFFICE_TASK` 등 환경변수만 제공한다. 사용 안내에 "프로젝트 AGENTS.md에 한 줄 적기" 예시를 싣는다.

### 권한과 샌드박스

- **Claude 권한**: 세션 설정 조각의 `permissions.allow`에 `Bash(<office-task shim>:*)`를 추가해 실행 확인 창이 뜨지 않게 한다. 동료 대화의 `settings_fragment`·`write_talk_only_settings` 경로를 일반화한다. 조건은 `talkEnabled`(talk shim) 또는 `workPanelsEnabled`(task shim)이고, 켜진 기능의 shim만 승인한다. 관찰이 꺼져 있어 훅 설정 파일이 없으면 권한 조각만 담은 파일을 따로 만들어 `--settings`로 물리는 기존 방식을 그대로 쓴다.
- **Codex 샌드박스**: 기본 `workspace-write` 샌드박스는 작업 폴더 밖 쓰기를 막는다. 그래서 `office-task`가 앱 데이터 폴더에 쓰려면 승인 요청이 뜬다. `-c sandbox_workspace_write.writable_roots=[…]`로 수신함을 추가한다.
  - 이 `-c`는 사용자가 설정한 같은 배열을 **덮어쓴다.** `~/.codex/config.toml`의 기존 값을 읽기만 해서 합친 배열을 넘긴다(쓰지 않는다).
  - 사용자가 `read-only` 샌드박스면 쓸 수 없고 승인이 필요하다. 이건 받아들인다.
  - 합치는 방식과 프로필별 설정의 우선순위는 실측 필요.
- **Claude 샌드박스**: 사용자가 Bash 샌드박스를 켰다면 같은 문제가 있다. 세션 설정 조각으로 수신함 쓰기를 허용할 수 있는 키가 있는지 실측하고, 없으면 승인 요청이 뜨는 것을 받아들인다.
- Pi·Kilo·agy는 샌드박스 동작을 실측 때 함께 확인한다.

## 받아들이기 (구현)

### 네이티브

`src-tauri/src/persistence/work_inbox.rs` (신규)

- `scan(root, now) -> Vec<InboxEntry>`: `.md` 파일을 이름순으로 최대 50개 읽고 파싱한다. 거부 대상은 그 자리에서 `rejected/`로 옮긴다. 24시간 지난 임시 파일을 지운다.
  - `InboxEntry { file_name, key, title, body, assignee: Option<String> }`
  - `key`는 파일 이름과 원본 바이트로 만든 해시 앞 16자(hex)다.
- `ack(root, file_names)`: 지정한 파일을 지운다. 이름에 경로 구분자·`..`가 있거나 `.md`가 아니면 무시한다. 렌더러 입력으로 수신함 밖 파일이 지워지는 것을 막는다. 이미 없으면 성공으로 본다.
- 파서는 순수 함수(`parse(name, bytes) -> Result<Parsed, RejectReason>`)로 분리해 단위 테스트한다.

IPC (`src-tauri/src/ipc/commands/work_panels.rs`에 추가, `lib.rs`의 handler 목록에 등록)

| 커맨드 | 내용 |
| --- | --- |
| `scan_work_inbox` | `Vec<InboxEntry>` 반환(카멜케이스) |
| `ack_work_inbox` | `fileNames: string[]` 삭제 |

`agent-office task` 서브커맨드 (`src-tauri/src/work_inbox_cli.rs` 신규, `maybe_run_cli`에서 분기)

- GUI·tokio 런타임을 띄우지 않는다. 인자 파싱, 경로·담당 결정, 원자적 쓰기만 한다.
- frontmatter 값은 한 줄로 정규화한다(줄바꿈 제거). `--to` 값에 줄바꿈이 있으면 거부한다.

### 렌더러

`src/renderer/workpanels/workInbox.ts` (신규)

- `resolveAssignee(raw, profiles) -> string | undefined`: 담당 판정 규칙을 구현하는 순수 함수.
- `startWorkInbox()` / `stopWorkInbox()`: 3초 주기 확인 루프. 업무 패널 설정이 켜지면 시작하고 꺼지면 멈춘다. 한 번에 한 주기만 돌도록 직렬화한다.
- 한 주기의 순서:
  1. `scan`
  2. 스토어 `importInbox`
  3. `flush`
  4. 성공하면 `ack`(이미 있는 id로 건너뛴 파일 포함)
  5. 실패하면 `ack`하지 않는다

`workItemStore.ts`

- `importInbox(entries: { id, title, body, assigneeAgentId? }[]) -> string[]`: `loaded`이고 오류가 없을 때만 동작한다. 이미 있는 id는 건너뛴다. `todo` 끝 순서로 추가하고 `selectedId`는 건드리지 않는다. `dirty`로 표시하고 반영한(또는 건너뛴) id를 반환한다.
- 기존 `add`는 선택을 옮기고 초안 형식을 받으므로 재사용하지 않는다.

`lifecycle.ts`: 패널 활성화와 종료 흐름에 시작·정지를 연결한다. 종료 시 진행 중인 주기를 기다린 뒤 기존 flush로 넘어간다.

화면 문자열은 추가하지 않는다. 가져온 업무는 기존 카드로 보이고, 저장 실패는 기존 미저장 표시·재시도를 쓴다.

## 단계

1. **수신함 받아들이기**: 파서·scan·ack, IPC, 렌더러 루프·`importInbox`, 담당 판정. 이 단계만으로 외부 프로그램이 규약대로 파일을 넣어 쓸 수 있다.
2. **`office-task`**: `agent-office task` 서브커맨드, shim 생성, 세션 환경변수 3종, TTS env 제거.
3. **공통 안내문 (Claude·Codex)**: 안내문 상수, persona와 합치기, Claude 권한 조각 일반화, Codex `writable_roots` 합치기.
4. **공통 안내문 (Pi·Kilo·agy)**: 채널·샌드박스를 실측하고, 되는 곳은 주입하고 안 되는 곳은 환경변수 폴백으로 둔다.
5. **문서**: 규약·구현 위치를 [업무 패널](../work-panels-design.md)에 합친다. [guide.md](../guide.md)에 수신함 경로·형식과 다른 CLI용 AGENTS.md 예시를 추가하고, [docs/README.md](../README.md) 인덱스를 갱신한다.

단계마다 `npx tsc --noEmit`, `npx vitest run --dir src`, `cargo test --manifest-path src-tauri/Cargo.toml`를 통과시키고 커밋한다.

## 테스트

네이티브

- 파서: frontmatter 유무, `# ` 제거, 제목만 있는 파일, BOM·CRLF, 닫히지 않은 frontmatter, 알 수 없는 키 무시, 빈 파일, 비 UTF-8, 64 KiB 초과.
- scan: 이름순·50개 상한, `.`으로 시작하는 파일과 `.md`가 아닌 파일 무시, 거부 파일의 `rejected/` 이동과 `.error` 사유, 오래된 임시 파일 정리, 같은 내용이면 key가 같음.
- ack: 경로 탈출 이름 무시, 없는 파일 성공.
- `agent-office task`:
  - 담당 결정 세 갈래(`--to`, `--unassigned`, 환경변수)
  - 환경변수가 없을 때의 경로 폴백과 실패
  - stdin 본문
  - 원자적 쓰기(임시 파일이 남지 않음)
  - 산출 파일이 파서로 그대로 읽힘(왕복)
- 세션 계획: 세 환경변수 주입, attach 경로 포함.
- 덧붙일 지시문: persona만 / 안내문만 / 둘 다 / 둘 다 없음별 Claude·Codex 인자. 업무 패널이 꺼지면 안내문이 빠짐. 안내문에 shim 절대 경로가 들어감. Codex TOML 이스케이프 유지.
- 권한 조각: 기능 조합(talk만 / task만 / 둘 다 / 둘 다 꺼짐)별 `permissions.allow`.
- Codex `writable_roots`: 사용자 설정 배열이 없을 때 / 있을 때 합치기, 설정 파일이 깨졌을 때 수신함만 넣기.

렌더러

- `resolveAssignee`: id 일치, 이름 단일 일치, 이름 중복, 없음, 공백.
- `importInbox`: 중복 id 건너뜀, `todo` 끝 순서, 선택·편집 초안 불변, 로드 실패 시 무동작.
- 루프: flush 실패 시 ack하지 않음, 성공 시 건너뛴 파일까지 ack, 패널 끄기에 정지, 주기 겹침 없음.

실기 인수

- [ ] 설치 앱에서 외부 셸로 파일을 넣어 업무 카드가 생기고 파일이 지워지는지 확인.
- [ ] 패널을 끈 상태에서 쌓인 파일이 켤 때 들어오는지 확인.
- [ ] 인증된 Claude 세션에서 "다음 할 일로 남겨 줘"에 안내문대로 `office-task`가 실행되고, 권한 확인 없이 본인 캐릭터에게 배정되는지 확인. persona가 있는 캐릭터에서 말투가 유지되는지도 확인.
- [ ] Codex 세션에서 같은 요청이 샌드박스 승인 없이 들어오는지 확인.
- [ ] Pi·Kilo·agy에서 채널 실측 결과대로 동작(안내문 주입 또는 환경변수 폴백)하는지 확인.

## 후속 과제

- 출처 표시: `source` frontmatter와 `WorkItem.origin` 필드, 카드의 작은 출처 라벨.
- 기존 업무 갱신(상태 변경·완료)을 위한 규약 확장.
- 도착 알림(토스트·말풍선).
