# 프로젝트 점검·기능 정리 계획

- 상태: 진행 중 (2026-10-14) — A·B·C·D 기록, G 완료(§7.G, 14건)
- 목적: 프로젝트의 문제점을 찾아 줄을 세우고, 잘 쓰지 않게 된 기능을 골라 제거해 유지 비용을 줄인다.
- 실행 단위: agent office 업무 보드 태스크(§6). 각 태스크 본문이 이 문서의 절을 가리킨다.

## 1. 원칙

- **코드보다 질문이 먼저다.** §2의 관점과 가설을 고정한 뒤 코드를 본다. 결과는 "예상 vs 실제"로 기록해 확증 편향을 막는다.
- **증거 없는 문제는 문제로 올리지 않는다.** 모든 발견에는 재현 절차나 `파일:줄`이 붙는다.
- **제거는 사람이 결정한다.** 사용 신호(§3)는 후보를 고르는 근거일 뿐이다. 제거 목록은 사용자 승인 후에만 실행한다.
- **제거 대상은 정밀 조사하지 않는다.** 없어질 코드를 깊게 읽는 비용을 아낀다.
- 저장소 규칙(AGENTS.md)을 따른다: `main`에서 로컬 커밋, 푸시 금지, 화면 문자열은 i18n 카탈로그, 검증은 `npx tsc --noEmit` + `npx vitest run --dir src` + `cargo test --manifest-path src-tauri/Cargo.toml`. (`vscode-ext` 테스트는 C40 제거로 없어짐)

## 2. 점검 관점 — 유능한 엔지니어가 볼 것

| 관점 | 질문 |
|---|---|
| 목적 적합성 | 핵심 흐름(캐릭터 소환 → 세션 → 업무 보내기 → 완료 → 선행 해제)이 끝까지 동작하는가 |
| 상태 소유권 | 업무·캐릭터·세션·설정의 원본이 하나인가. UI·`office-task`·`ctl`·웹 원격이 같은 상태를 각자 고치지 않는가 |
| 동시성·무결성 | 여러 에이전트가 동시에 `office-task`를 호출할 때의 쓰기 경합, 의존성 순환·고아 선행, 가져오기 ID 충돌, 설정 파일 손상 복구 |
| 실패 모드 | 에이전트/PTY 비정상 종료, 앱 재시작 복구, v1 핸드오프·v2 브로커 공존, 임시 디렉터리(`$TMPDIR/agent-office/bin`) 소실 |
| 신뢰 경계 | 업무 본문 = 에이전트 지시문(가져온 JSON의 프롬프트 인젝션), 로컬 axum 서버·웹 원격·tailnet 인증, CLI 인자 이스케이프, 봇 모드 외부 이슈 본문 주입 |
| 권한·책임 | "사람 지시 시에만 완료" 같은 규칙이 코드로 강제되는가, 안내문에만 의존하는가 |
| 유지보수성 | 거대 파일·중복 로직·죽은 코드, 기능 플래그 잔존, docs와 코드의 표류 |
| 검증 체계 | 핵심 로직의 테스트 유무, 타입 검사 공백, 실기 검증 대기로 남은 항목 |
| 운영성 | 로그·에러 메시지만으로 원인 추적이 되는가 |
| 성능·자원 | 폴링 주기, 장시간 실행 메모리, 전사 기록(30일·2GB)·세션 로그(300일) 디스크 사용, 빌드 산출물 |

## 3. 기능 사용 신호와 제거 기준

### 3.1 신호 (여러 개를 겹쳐 판단한다)

1. **사용자 응답** — 가장 강한 신호. 기능 목록을 보여 주고 "최근 쓴 적 있음/없음/모름"을 받는다.
2. **실사용 흔적** — 앱 데이터(설정 opt-in 값, 세션 이벤트, 기능별 저장 파일)에 쓴 흔적이 있는가.
3. **변경 이력** — `git log`상 마지막 의미 있는 변경 시점, 버그 수정만 이어지는지.
4. **문서 상태** — `docs/README.md`의 "기본 off", "실험", "동결", "미착수", "눈검증 대기" 표기.
5. **결합 비용** — 그 기능 때문에 다른 코드가 분기·플래그·호환층을 들고 있는가.

### 3.2 문서상 1차 후보 (판정 아님, 조사 출발점)

| 영역 | 근거 |
|---|---|
| 세션 v1 핸드오프 vs v2 브로커 | 두 경로 공존, v2는 기본 off |
| ~~앱↔앱 뷰어(`peer/viewer.rs`)~~ | 코드 이미 없음, 잔여 파일 삭제(§7.D) |
| IDE 세션 관찰(spike) | 실험 구현 |
| 봇 모드(Gitea 폴링) | 특정 인프라 의존 |
| 터미널 입력 자동화, 실행 레시피 | opt-in 부가 기능 |
| 마스코트 창·조명, TTS 대사 | 부가 연출 |
| VSCode 확장 | 별도 패키지, V2·V3 미착수 |
| Pi·Kilo·Antigravity 감지 | CLI별 어댑터 유지 비용 |
| 이 달의 우수사원, 호감도 | 구현 중 / 구현 전 |

### 3.3 판정

- **제거**: 사용자 미사용 확인 + 결합 비용 있음
- **보류**: 사용 여부 불명 → 다음 점검까지 관찰
- **유지**: 사용 중이거나 핵심 흐름의 일부
- **축소**: 기능은 유지하되 대체된 경로·플래그·호환층만 제거

### 3.4 제거 절차 (기능 하나당 커밋 하나)

1. 진입점 목록: UI, IPC 커맨드·이벤트, `ctl` 하위 명령, 웹 원격 API, `vscode-ext` 의존, 설정 필드, 앱 데이터 파일, i18n 키, 테스트
2. 저장된 사용자 설정·데이터의 하위 호환: 사라진 필드가 들어 있는 설정 파일을 읽어도 깨지지 않게 한다(무시 또는 마이그레이션)
3. 코드·테스트·i18n 키 삭제 → 전체 검증 명령 통과
4. docs: 해당 설계 문서를 `docs/archive/`로 옮기고 `docs/README.md` 표를 갱신(삭제하지 않음 — 이슈 링크 보존)

## 4. 단계

| 단계 | 내용 | 산출물 |
|---|---|---|
| A. 블랙박스 사용 점검 | 앱을 실행해 핵심 흐름과 경계 상황(동시 완료, 순환 의존, 깨진 JSON 가져오기, 에이전트 강제 종료, 앱 재시작)을 해 본다 | §7에 관찰 기록 |
| B. 구조 조감 | 디렉터리 지도, 파일 크기 상위, `git log` 변경 빈도 상위, 테스트 분포, 의존성 목록, docs 표류 목록. 정독하지 않는다 | §7에 표 |
| C. 기능 목록·사용 신호 수집 | 기능 전체 목록을 만들고 §3.1 신호 2~5를 채운다 | §7에 기능별 신호 표 |
| D. 제거 대상 확정 | C 결과를 사용자에게 보여 주고 §3.3 판정을 받는다 | 확정 목록 + 기능별 제거 태스크 |
| E. 위험 영역 정밀 조사 | A·B에서 걸린 곳만 깊게 본다. 우선순위: 동시성·영속성 → 신뢰 경계 → 프로세스 수명주기. 제거 대상은 제외 | 증거 붙은 발견 목록 |
| F. 분류·후속 태스크화 | 심각도 × 발생 가능성으로 정렬, 증상/근본 원인 구분, 확인된 문제마다 수정 태스크 생성 | §7 발견 표 + 보드 태스크 |
| G. 제거 실행 | D의 기능별 제거 태스크를 §3.4대로 수행 | 기능별 로컬 커밋 |
| H. 낡은 문서 정리 | §4.1대로 docs와 코드의 표류를 바로잡는다 | 문서 커밋 + `docs/README.md` 상태 갱신 |
| I. 마무리 | 전체 검증, 이 문서를 이력(`archive/`)으로 전환 | 최종 커밋 |

### 4.1 낡은 문서 정리 (H)

- **Codex로 수행한다**: 모델 `gpt-6.1-sol`, 추론 강도 `high`. 예:
  `codex exec -m gpt-6.1-sol -c model_reasoning_effort=high -C <저장소> "<지시>"`
  (또는 `/codex:rescue`에 같은 모델·강도를 지정). Codex 결과는 그대로 커밋하지 않고 diff를 검토한 뒤 커밋한다.
- 대상: `docs/*.md` 전부, `README.md`, `AGENTS.md`의 프로젝트 지침 부분, `vscode-ext`의 문서.
- 판정 기준(`docs/README.md`의 3종 표기를 따른다):
  - **정본** — 현재 코드와 일치. 날짜만 갱신
  - **부분표류** — 바로잡을 수 있으면 고치고, `file:line` 근거가 낡았으면 심볼 기준으로 바꾼다
  - **이력** — 구현 전 스냅샷·폐기 설계·완료된 계획 → `docs/archive/`로 이동(삭제 금지, 이슈 링크 보존)
- 끊긴 참조(없는 파일·없는 설정·없는 명령)를 모두 찾아 고친다. 예: §7.0의 `REBUILD-PLAN.md`.
- "눈검증 대기"·"구현 중"처럼 오래 머문 상태 표기는 사실 확인 후 갱신하거나 잔여 항목을 태스크로 뺀다.
- G에서 제거된 기능의 문서는 G가 archive로 옮겼는지 확인만 한다.

## 5. 의존 관계

```
A ─┐
B ─┼─────────► E ─► F ───────┐
C ─┴─► D ──────┘             ├─► I
       └──────► G ─► H ──────┘
B ─────────────────► H
```

- E는 D 뒤: 제거 대상을 빼고 조사하기 위해서다.
- G와 E는 병렬 가능. 같은 파일을 건드리면 G를 먼저 커밋한다.
- H는 B(표류 목록)와 G(제거 완료) 뒤: 사라질 기능의 문서를 고치는 헛수고를 막는다.

## 6. 업무 보드 태스크

| 단계 | 태스크 id | 선행 |
|---|---|---|
| A. 블랙박스 사용 점검 | t-cfb46523 | — |
| B. 구조 조감과 문서 표류 목록 | t-ff214f52 | — |
| C. 기능 목록과 사용 신호 수집 | t-81076be1 | — |
| D. 제거 대상 확정 (사용자 승인) | t-b3381017 | C |
| E. 위험 영역 정밀 조사 | t-0a5da455 | A, B, D |
| F. 발견 분류와 수정 태스크화 | t-b40ba5de | E |
| G. 확정 기능 제거 실행 | t-fb1d64e4 | D |
| H. 낡은 문서 정리 (Codex `gpt-6.1-sol` high) | t-ea542576 | B, G |
| I. 마무리 | t-893439bd | F, H |

D와 F는 실행 중에 하위 태스크(기능별 제거, 문제별 수정)를 더 만든다. D가 만든 기능별 제거 태스크는 §7.D.2.

## 7. 기록

(각 단계 산출물을 여기에 쌓는다.)

### 7.0 착수 전 관찰

- `docs/README.md`가 루트 `REBUILD-PLAN.md`를 참조하지만 파일이 없다 — docs 표류 사례(B에서 확인).

### 7.A 블랙박스 사용 점검 (2026-10-13, 설치본 `/Applications/agent-office.app` 실행 중 상태)

방법: 코드를 읽지 않고 실행 중인 앱에 `office-task` CLI와 업무 수신함(`<app_data>/work-inbox/*.md`, guide.md 규약)으로 입력을 넣고, `office-task --list --all`과 `<app_data>/work-items.json`으로 결과를 봤다. 테스트 업무는 제목에 `[A테스트]`를 붙였다. 아래 `$T` = `$AGENT_OFFICE_TASK`, `$I` = `$AGENT_OFFICE_WORK_INBOX`.

#### 7.A.1 발견 (예상 vs 실제)

| # | 관점 | 상황 · 재현 | 예상 | 실제 | 판단 |
|---|---|---|---|---|---|
| A1 | 실패 모드·동시성 | 앱 실행 중 `/Applications/agent-office.app/Contents/MacOS/agent-office --help` | 사용법 출력 후 종료, 또는 단일 인스턴스로 거절 | 도움말 없이 **두 번째 앱 인스턴스가 떠서** 멈춤(`session-log gc: 기간 11 개 / 용량 0 개 삭제` 출력). 이 인스턴스가 `<app_data>/observer-port`를 자기 포트(56421)로 덮어썼고, 종료 후에도 파일은 죽은 포트를 가리켰다(살아 있는 앱은 49966). `observer/{claude,kilo,pi,agy}`·`claude-plugin`·`$TMPDIR/agent-office/bin/office-talk`도 다시 썼고 세션 로그 GC를 돌렸다 | **높음**. 단일 인스턴스 보호 없음 + 알 수 없는 인자에서 GUI 기동. `observer-port` 재발견에 기대는 forwarder/pi/kilo/agy 훅이 앱 재시작 전까지 끊길 수 있다. (점검 중 `observer-port`는 49966으로 손으로 되돌렸다) |
| A2 | 동시성·무결성 | `X=$($T "x"); $T --list` 바로 실행 | 만든 업무가 목록에 보임 | 즉시 `--list`엔 없고 약 3초 뒤 보임. 그런데 아직 반영 전인 X에 `--done X`는 성공(rc 0), `--after X`도 유효 | 비동기 반영이 CLI 출력에 드러나지 않음. 안내문의 `A=$(…); … --after "$A"` 패턴은 동작함. **낮음** |
| A3 | 동시성·무결성 | `--after t-ghost0000`(없는 id), 수신함 `after:` 자기 자신, 수신함 순환(A after B, B after A를 같은 스캔에 투입) | 거절 또는 경고 | 모두 rc 0/거절 파일 없음. 없는 선행·자기 선행은 **조용히 버려짐**. 순환 쌍은 처리 순서에 따라 한쪽 선행만 남아(B after A) 순환은 생기지 않았지만 A의 `after`는 말없이 사라짐 | 순환은 막히나 입력 손실을 알리지 않음. **중간** |
| A4 | 권한·책임 | `$T --to 없는사람 "t"` | 오류 또는 경고 | rc 0, `@unassigned`로 생성. 오타가 미배정으로 묻힘 | **중간** |
| A5 | 무결성 | `$T "" "본문"` | 빈 제목 거절 | rc 0, 제목 `""`인 업무 생성 | **문제 아님**. 기존 실사용 업무에도 제목 없이 본문만 있는 카드가 여럿 있다(본문만으로 업무가 성립하는 설계로 보임) |
| A6 | 권한·책임 | 선행 Q 미완 상태에서 후속 R에 `$T --done R` | 거절하거나 경고 | 완료됨. 목록에 `[done] … after: Q (blocked)`처럼 모순 표시 | **중간**. "선행 해제" 모델과 어긋남 |
| A7 | 권한·책임 | 커널 세션에서 `$T --to 박좀비 "t"` 만든 뒤 `$T --done <그 id>` | (규칙상) 사람 지시 시에만, 최소한 담당자 확인 | 다른 캐릭터 담당 업무도 바로 완료. "사람 지시 시에만 완료"는 **안내문에만 의존**, 코드 강제 없음 | 설계 판단 필요(E에서). **중간** |
| A8 | 동시성·무결성 | 수신함에 기존 업무 id로 `id: t-4e20c6b0` 파일 투입 / `done: t-nothere1` 파일 투입 | `rejected/`로 옮기고 `.error`에 사유 | 두 파일 모두 **흔적 없이 사라짐**(rejected 없음, 기존 업무 불변) | 형식 오류(`empty`/`invalid-utf8`/`unclosed-frontmatter`)는 rejected로 가는데 의미 오류는 버려짐. **중간** |
| A9 | 실패 모드 | `$TMPDIR/agent-office/bin/office-task` 삭제 후 15초 관찰 | 자동 재생성(또는 다음 세션에서) | 15초 내 재생성 안 됨(복원함). 파일 mtime으로 보아 세션 생성 때 다시 씀 | macOS의 `/var/folders` 정리 시 실행 중 세션은 CLI를 잃음. **낮음** |
| A10 | 운영성 | `<app_data>/panic.log` | 비어 있음 | 2026-10-01 `session_log::transcript::TranscriptFilter::erase_line` 인덱스 범위 밖 패닉(`transcript.rs:462`, `len 0 index 0`) 기록. 스레드 `session-log-<agentId>` | 세션 로그 스레드가 죽으면 그 세션 기록이 끊길 수 있음. E에서 확인. **중간** |
| A11 | 상태 소유권 | (`cliEnabled` 켬) `agent-office ctl create atest-probe --name 점검A봇 --cwd <dir> --startup-command cat` — 없는 캐릭터 id | 거절, 또는 캐릭터를 만들고 목록에 보임 | rc 0, `{"sessionId":…,"state":"running"}`. 실제 PTY(`zsh → cat`)가 떴지만 `ctl list`/`--json`에 **나타나지 않음**(프로필 없음). `--name`·`--role`은 무시된 듯 | 캐릭터 없는 숨은 세션. `dispose`로만 정리 가능. **중간** |
| A12 | 운영성 | 위 숨은 세션의 셸을 `kill -9`한 뒤 `ctl send atest-probe "x" --enter` / `ctl dispose nobody-xyz`(없는 id) / 같은 id 두 번 `dispose` | 오류 rc≠0 | 모두 `ok`, rc 0 | 실패를 성공으로 보고. 자동화가 오판한다. **중간** |

#### 7.A.2 정상 동작 확인 (예상 = 실제)

- 기본 흐름(CLI): 생성 → `--after` 후속 `(blocked)` → 선행 `--done` → 후속 blocked 해제.
- 동시 생성 40건 병렬: id 40개 고유, 보드 40건 반영, `work-items.json` 유효 JSON.
- 동시 완료: 41건 병렬 `--done` + 그중 5건 중복 `--done` 동시 → 전부 완료, 공통 선행 1개를 기다리던 후속 10건 모두 해제, 파일 손상 없음. 중복 완료는 rc 0(`… is already done` 안내).
- `--done 없는id 있는id` 혼합 → 전체 거절(rc 1), 있는 쪽도 바뀌지 않음(원자적).
- 인자 이스케이프: 제목 `"q" 's' $(whoami) \`id\``, 본문 줄바꿈·`$HOME` 그대로 저장(셸 해석 없음).
- 본문 64 KiB 초과 → `office-task: task exceeds 65536 bytes`, rc 1. (제목 5,000자는 허용)
- 앱 데이터 env 없이 실행 → `AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set`, rc 1.
- 수신함 형식 오류: 빈 파일 `empty`, 깨진 UTF-8 `invalid-utf8`, 닫히지 않은 frontmatter `unclosed-frontmatter`로 `rejected/`에 사유와 함께 이동.
- 자원(3시간 36분 가동): RSS 약 158 MB, app_data 118 MB(session-events 53 MB, session-logs 45 MB), 리슨 포트는 127.0.0.1 훅 포트 하나(웹 원격·ctl 꺼짐).
- 에이전트 강제 종료(ctl로 만든 시험 세션): 에이전트 프로세스(`cat`) `kill -9` → 셸은 살아 있고 입력 주입 계속 가능. 셸 `kill -9` 후 같은 id로 `ctl create` → 새 세션 id로 다시 뜸. `dispose` 후 앱 자식 프로세스에 잔여·좀비 없음.
- `agent-office ctl --help`는 정상 출력(A1의 최상위 `--help`와 대비).
- 요약기: 세션이 움직이는 동안 `opencode run …`이 30초에 2회꼴로 앱 자식으로 뜬다(비용·자원 관찰용, 판단 보류).

#### 7.A.3 미수행 · 정리

- **앱 재시작 복구 / 설정 파일 손상 복구**: 점검 세션 자체가 설치본 앱의 자식 PTY라 앱을 끄면 점검이 끊긴다. 사용자 지시로 설치본은 끄지 않았다. 별도 app_data를 쓰는 격리 인스턴스가 필요하다(A1 때문에 같은 app_data로 두 번째 인스턴스를 띄우는 것은 금지).
- **GUI 전용 항목은 사용자 판단으로 생략**: 캐릭터 소환(GUI), 업무 '보내기', 가져오기 팝업(깨진/낯선 JSON), A11 숨은 세션의 화면 표시.
- **시험 데이터 정리에서 드러난 점(상태 소유권, E 입력)**: 외부에서 업무를 지울 길이 없다(`office-task`·수신함·`ctl` 모두 삭제 없음, UI는 한 건씩 확인 대화상자). 업무 목록의 원본은 렌더러 메모리이고 `work-items.json`은 부팅 때 한 번만 읽은 뒤 통째로 덮어쓰므로, 앱이 켜진 동안 파일을 고치면 다음 저장에 되살아난다. 그래서 시험 업무 66건은 `--done`으로 닫아 두고, 앱을 끈 상태에서 파일에서 지우는 스크립트로 정리했다.

### 7.B 구조 조감 (2026-10-13, `main` `cc8fbc3` 기준, 정독 없음)

방법: `git ls-files`·`wc -l`·`git log --name-only`·정규식 집계만 썼다. 줄 수는 추적 파일 기준, Rust는 인라인 `#[cfg(test)] mod tests {` 앞까지를 본체로 셌다. 저장소 이력은 2026-07-09부터 455커밋이다.

#### 7.B.1 디렉터리 지도

| 영역 | 파일 | 줄 | 비고 |
|---|---:|---:|---|
| `src/renderer/` | 594 | 95,152 (테스트 포함) | 39개 하위 폴더. 큰 순: `office` 14,965 · `terminal` 8,177 · `settings` 5,311 · `store` 5,266 · `ipc` 5,009 · `profile` 4,775 · `workdir` 4,229 · `diary` 3,427 · `usage` 3,298 · `awards` 2,908 · `mascot` 2,795 · `agent` 2,729 · `workpanels` 2,596 |
| `src/shared/` | 135 | 3,229 (+ i18n JSON 82개, 계약 fixture 29개) | 프런트·백엔드 공유 타입(`types/`, 배럴 `types.ts`)과 i18n 카탈로그 |
| `src/web/` | 21 | 3,412 | 웹 원격 클라이언트(`vite.web.config.ts` → `dist-web/`) |
| `src-tauri/src/` | 201 | 약 95,400 (인라인 테스트 포함) | 모듈 큰 순: `session` 16,797 · `observer` 8,285 · `automation` 7,745 · `webremote` 5,535 · `usage` 5,206 · `persistence` 4,955 · `ipc` 4,822 · `session_log` 4,016 · `control` 3,913 · `sessiond` 3,622 · `summarizer` 3,513 · `tts` 3,366 · `ide_sessions` 3,362 · `workdir` 2,030 · `notification` 1,918 · `bot` 1,616. 최상위 단일 파일 8,840줄(`types.rs` 2,208 · `lib.rs` 1,306 · `work_send.rs` 678 · `state.rs` 659) |
| `src-tauri/tests/` | 3 | — | IPC 계약 fixture 대조(`contract_fixtures.rs`) 등 |
| `vscode-ext/` | 19 | — | 별도 패키지 v0.1.0, 소스 7 + 테스트 4 |
| `scripts/` | 19 | — | 빌드·서명·청소 + `ide-session-spike/`(실험) |
| `docs/` | 43 | — | 활성 36 + `archive/` 6 + 인덱스 |

업무 기능의 위치(E 입력): 수신함 파싱 `src-tauri/src/persistence/work_inbox.rs`, CLI `src-tauri/src/work_inbox_cli.rs`, 저장 `persistence/work_panels_store.rs`, 보내기 `work_send.rs`, 업무 목록 원본은 렌더러 `src/renderer/workpanels/`(§7.A.3의 소유 구조와 일치).

#### 7.B.2 파일 크기 상위 20 (본체 줄, 테스트 파일·인라인 테스트 제외)

| # | 파일 | 본체 | 전체 |
|---:|---|---:|---:|
| 1 | `src-tauri/src/session/manager.rs` | 1,544 | 1,544 (테스트는 `manager/tests.rs` 2,660 · `real_pty_smoke.rs` 1,686 별도) |
| 2 | `src-tauri/src/automation/definition_runner.rs` | 1,445 | 1,508 |
| 3 | `src/renderer/store/appStore.ts` | 1,424 | 1,424 |
| 4 | `src/renderer/automation/AutomationPalette.tsx` | 1,232 | 1,232 |
| 5 | `src-tauri/src/types.rs` | 1,151 | 2,208 |
| 6 | `src-tauri/src/lib.rs` | 1,111 | 1,306 |
| 7 | `src-tauri/src/observer/event.rs` | 1,100 | 2,241 |
| 8 | `src-tauri/src/sessiond/daemon.rs` | 1,063 | 1,063 (+`mod tests;` 별도) |
| 9 | `src-tauri/src/ide_sessions/transcripts.rs` | 855 | 1,193 |
| 10 | `src/renderer/terminal/AgentTabStrip.tsx` | 824 | 824 |
| 11 | `src-tauri/src/usage/gemini_live.rs` | 800 | 1,114 |
| 12 | `src-tauri/src/session/inject.rs` | 795 | 1,225 |
| 13 | `src-tauri/src/session/pty_factory.rs` | 787 | 1,120 |
| 14 | `src-tauri/src/observer/codex_usage.rs` | 764 | 1,390 |
| 15 | `src-tauri/src/session/external.rs` | 740 | 740 |
| 16 | `src/renderer/office/OfficeScene.ts` | 733 | 733 |
| 17 | `src-tauri/src/control/client.rs` | 732 | 931 |
| 18 | `src/renderer/usage/usageView.ts` | 703 | 703 |
| 19 | `src/renderer/ipc/tauriApi.ts` | 703 | 703 |
| 20 | `src/renderer/mascot/MascotApp.tsx` | 688 | 688 |

전체 줄로만 보면 `observer/event.rs`·`types.rs`·`control/mod.rs`(1,546, 본체 253)가 위에 오지만 절반 이상이 인라인 테스트다. 바이트 기준 최대는 아이콘·폰트(`icon.icns` 711 KB, `Galmuri11.woff2` 505 KB)로 코드 문제 아님.

#### 7.B.3 변경 빈도 상위 20 (전체 이력 커밋 수, 현존 파일)

| # | 파일 | 커밋 | 최근 90일 |
|---:|---|---:|---:|
| 1 | `src-tauri/src/lib.rs` | 105 | 92 |
| 2 | `src-tauri/Cargo.toml` | 67 | 47 |
| 3 | `src/renderer/store/appStore.ts` | 63 | 55 |
| 4 | `src/renderer/ipc/tauriApi.ts` | 63 | 57 |
| 5 | `src-tauri/Cargo.lock` | 62 | 42 |
| 6 | `src/shared/ipc.ts` | 60 | 55 |
| 7 | `src-tauri/src/session/manager.rs` | 57 | 44 |
| 8 | `package.json` | 55 | — |
| 9 | `src/shared/types.ts` | 52 | 39 |
| 10 | `src-tauri/src/types.rs` | 51 | 40 |
| 11 | `src-tauri/src/ipc/commands.rs` | 49 | — |
| 12 | `src/renderer/__tests__/bootstrap.test.ts` | 48 | 43 |
| 13 | `src-tauri/tauri.conf.json` | 48 | — |
| 14 | `src/shared/__tests__/contract.test.ts` | 47 | 43 |
| 15 | `src/renderer/store/__tests__/appStore.test.ts` | 46 | 40 |
| 16 | `src/renderer/ipc/__tests__/tauriApi.test.ts` | 46 | 42 |
| 17 | `package-lock.json` | 45 | — |
| 18 | `src-tauri/src/ipc/commands/tests.rs` | 42 | 42 |
| 19 | `src/renderer/labels/__tests__/summarizer.test.ts` | 41 | — |
| 20 | `src/shared/types/api.ts` | 40 | 40 |

읽기: 기능 하나를 더할 때마다 IPC 배선 사슬(`lib.rs` 커맨드 등록 → `ipc/commands*` → `types.rs` ↔ `shared/types*` → `shared/ipc.ts` → `tauriApi.ts` → `appStore.ts` + 각 계약·부트스트랩 테스트)을 통째로 건드린다. 크기 상위와 빈도 상위가 겹치는 `lib.rs`·`appStore.ts`·`session/manager.rs`·`types.rs`가 결합 지점이다(E·유지보수성 관점 후보, C의 "결합 비용" 신호 입력).

#### 7.B.4 테스트 분포

| 영역 | 테스트 | 비고 |
|---|---|---|
| `src/renderer` | 202파일 · 약 2,765건 | 두꺼운 곳: `office` 452 · `terminal` 271 · `store` 183 |
| `src/shared` | 3파일 · 14건 | 계약 테스트(`contract.test.ts`)가 Rust `tests/contract_fixtures.rs`와 fixture 29개를 공유 |
| `src/web` | 5파일 · 48건 | |
| `src-tauri` | `#[test]`/`#[tokio::test]` 1,795건, `#[ignore]` 38건 | 모듈 큰 순: `session` 281 · `persistence` 147 · `observer` 140 · `usage` 122 · `automation` 101 · `summarizer` 99 · `tts` 97 · 최상위 파일 183. `#[ignore]`는 실 PTY 스모크(`real_pty_smoke.rs` 9, `automation/pty_smoke.rs` 4, `work_send/pty_smoke*.rs`)와 실제 CLI·네트워크 의존(`usage/*_live`, `summarizer/*`) — 평소 `cargo test`에서 돌지 않는다 |
| `vscode-ext` | 4파일 · 43건 | 별도 `npm test` |

얇은 곳(크기 대비): `src/renderer/automation` 2,005줄에 18건(`AutomationPalette.tsx` 1,232줄), `src-tauri/src/work_send*` 1,034줄에 일반 테스트 소수 + ignore 스모크, `mascot` 2,795줄에 1파일(63건), `workdir` 4,229줄에 2파일(64건). 업무 보내기(`work_send`)는 핵심 흐름인데 실 PTY 스모크가 ignore라 자동 검증 공백이다(E 후보).

#### 7.B.5 의존성

| 패키지 | 런타임 | 개발 |
|---|---|---|
| 루트 `package.json` (0.7.1) | `@tauri-apps/api` 2 · `plugin-notification` · `plugin-opener` · `@xterm/xterm` 5.5 + `addon-fit` · `addon-serialize` · `pixi.js` 8 · `react`/`react-dom` 18 · `zustand` 5 · `i18next` 25 + `react-i18next` 16 · `marked` 18 · `dompurify` 3 · `nanoid` 5 | `vite` 7 · `vitest` 2 · `typescript` 5.8 · `jsdom` 25 · `@testing-library/react` 16 · `@napi-rs/canvas` · `@tauri-apps/cli` 2 · 타입 |
| `src-tauri/Cargo.toml` (0.7.1) | `tauri` 2(`macos-private-api`) + 플러그인 dialog·notification·opener·window-state · `tokio` · `axum` 0.7(ws) · `tokio-tungstenite` 0.24(rustls) · `reqwest` 0.12(blocking, rustls) · `portable-pty` 0.8 · `parking_lot` · `rusqlite` 0.31 · `rust-embed` · `ignore` · `serde`/`serde_json`/`toml` · `chrono` · `uuid` · `sha1_smol`/`sha2`/`base64`/`flate2` · `sys-locale` · `tempfile`, unix `nix` 0.25·`libc`, windows `windows-sys` 0.61 | `tokio`(test-util) |
| `vscode-ext/package.json` (0.1.0) | 없음 | `typescript` 5.4 · `vitest` **1.6**(루트는 2.x) · `@types/node` 20(루트 26) |

`npm run` 스크립트 25개 중 `spike:ide`·`spike:ide:test`는 실험 스파이크(§3.2 후보) 전용.

#### 7.B.6 docs·코드 표류 목록 (H 입력)

자동 검사: 활성 docs·루트 `README.md`·`vscode-ext/README.md`의 링크와 백틱 경로를 추적 파일과 대조했고, 앱 데이터 파일명·과거 이름 언급 같은 오탐은 손으로 걸렀다. 재현 스크립트는 남기지 않았다(정규식 1회성).

**끊긴 참조**

| # | 위치 | 참조 | 실제 |
|---|---|---|---|
| D1 | `docs/README.md` §관련, 이 문서 §4.1·§7.0 | `REBUILD-PLAN.md` | 파일 없음(§7.0 확인) |
| D2 | `docs/README.md` archive 표 peer 행, 이 문서 §3.2 | "앱↔앱 뷰어(`peer/viewer.rs`)만 동결" | `src-tauri/src/peer/` 자체가 없다. 웹 원격 부활 때 viewer를 걷어내고 `peer/`→`webremote/`로 개명(`web-remote-design.md` §4·M1). §3.2 후보에서 빼야 한다 |
| D3 | `src-tauri/Cargo.toml` `tokio-tungstenite` 주석 | "피어 세션 공유 뷰어 쪽 WS 클라이언트, TLS feature를 켜지 않아 `ws://` 전용, axum과 tungstenite 0.21 공유" | 실제 feature는 `rustls-tls-native-roots`, 사용처는 `remote_client/transport.rs`·`webremote/mod.rs`, lock의 tungstenite는 0.24 하나. 주석 3가지가 모두 낡음(코드 주석 표류) |
| D4 | `subsystem-c-ui.md:684` | `src/renderer/profile/wordlists.ts` | `src/renderer/i18n/wordlists.ts`(`aa18f5e`에서 이동) |
| D5 | `subsystem-c-ui.md:1070` | `src/renderer/assets/fonts/`, "Electron 오프라인 보장" | 폰트는 `src/renderer/styles/fonts/`, 런타임은 Tauri |
| D6 | `subsystem-c-ui.md:1172` | `src/renderer/store/appStore.test.ts`, `notification/dedupe.test.ts`, `terminal/TerminalRegistry.test.ts` | 모두 `__tests__/` 아래로 이동 |
| D7 | `pi-support-design.md` | `file:line` 56개 중 15개는 없는 파일, 1개는 줄 수 초과 | README가 이미 "부분표류"로 표기. 심볼 기준 전환 대상 |
| D8 | `mascot-window-design.md`(14개 중 2개 없는 파일), `mascot-lights-design.md`(17개 중 1개 줄 수 초과) | `file:line` | 같은 처리 |
| D9 | `docs/README.md` §관련(docs/ 밖) | `remote-server-design.md` | docs/ 안 문서인데 "docs/ 밖" 절에 링크 없는 형태로 붙어 있음 |
| D10 | `docs/README.md` 인덱스 | — | `ide-persona-design.md`, `vscode-display-api.md`가 인덱스에 없음 |
| D11 | `docs/README.md` §관련, `docs/guide.md:113` | `AGENTS.md`를 "작업 규칙 정본"으로 지칭 | `AGENTS.md`·`CLAUDE.md`는 `.gitignore`(38·39행)로 추적 제외. 클론에는 정본이 없다. §4.1이 H 대상에 넣은 "AGENTS.md 프로젝트 지침"은 저장소 밖 파일이다 — 추적할지 결정 필요 |

**낡은 상태 표기**

| # | 문서 | 표기 | 사실 |
|---|---|---|---|
| S1 | 이 문서 머리·`docs/README.md` 행 | "계획 — 착수 전" | A·B 기록됨 → "진행 중"으로 고침(이 커밋) |
| S2 | `web-remote-design.md` 머리 | "구현 진행 중" | README는 "M1~M3 구현 완료, M4 미착수". 두 표기 불일치 |
| S3 | `docs/README.md` 우수사원 행 | "구현 중" | 문서 머리는 "정본", `src/renderer/awards/`(10파일) 마지막 변경 2026-08-26. 완료 여부 확인 후 갱신 |
| S4 | `affinity-design.md` | "기획 확정 — 구현 전"(2026-09-05) | 코드에 `affinity` 흔적 0건. 표기는 사실이나 5주째 미착수 → C의 판정(보류/이력) 대상 |
| S5 | 눈검증 대기 4건 | `session-log-design`, `external-session-attach-design`, `tts-confirm-line-design`, `vscode-character-extension-plan`(V1) | 각각 2026-08~09 이후 상태 변화 없음. 확인하거나 잔여를 태스크로 |
| S6 | `subsystem-a-sessions.md`·`subsystem-c-ui.md` 머리 | "2026-07-20 현행화" | 이후 세션 브로커·작업 패널·웹 원격·업무 수신함 등이 들어왔고 변경 빈도 상위 파일이 이 두 영역이다. 표기 날짜 이후 표류 가능성 높음(D4~D6이 일부 증거) |
| S7 | `session-handoff-design.md`·`session-broker-v2-design.md` | 상태 갱신 2026-07-20 | v2 기본 off 그대로. §3.2 후보 판정(C·D) 후 처리 |
| S8 | 머리에 상태줄 없음 | `agent-talk`, `antigravity-support`, `kilo-support`, `pi-support`, `mascot-lights`, `macos-signing`, `session-log`, `subsystem-c-ui`, `tts-confirm-line` | 상태는 README에만 있어 문서만 열면 알 수 없다. 표기 방식 통일 여부 결정 |

H 메모: D4~D6·D9·D10은 기계적 수정, D2·D3·D11·S2~S5는 사실 확인이나 결정이 필요하다. D2는 C(§3.2 후보 목록)에도 반영해야 한다.

### 7.C 기능 목록과 사용 신호 (2026-10-13, `main` `f1135c9` 기준, 판정 없음)

방법: 진입점은 설정 필드(`AppSettings`)·하단 바·`main.rs` 실행 모드(`serve`·`--observer-forward`·`--sessiond`·`ctl`·`task`)·`ctl` 하위 명령·`vscode-ext`·CLI별 훅/플러그인 모듈에서 모았다. 신호 2는 설치본 app_data(`~/Library/Application Support/com.bugcaptor.agent-office`, 이하 `<ad>`)의 설정 현재값·파일 mtime·`session-events/v1`(2026-07-11~)과 각 CLI의 자체 기록에서 셌다. 신호 3은 기능 경로에 대한 `git log`(전체/최근 30일)와 기능 이름이 들어간 커밋 중 마지막 것(= 마지막 고유 변경)이다. 경로 전체 기준 "마지막 커밋"은 업무 패널·수신함 작업(10-12~13)이 여러 기능을 한꺼번에 건드려 대부분 최근으로 보이므로 쓰지 않았다. 신호 5의 줄 수는 본체+테스트, "외부 참조"는 기능 경로 밖 비테스트 코드 파일 중 기능 식별자 정규식에 걸린 파일 수(근사, 기능 소유 저장소·IPC 파일도 포함됨)다. 집계 스크립트는 1회성이라 남기지 않았다.

한계: GUI 열람만 하는 기능(분석·타임라인·저장소 점검·문서 팔레트·셸 내보내기·캐릭터 번들)은 저장 흔적이 없어 신호 2가 비어 있다 → D의 사용자 응답이 유일한 근거다. Windows 설치본의 app_data는 이 기기에서 볼 수 없다.

#### 7.C.1 기능별 신호

열: ② 실사용 흔적 · ③ 변경 이력(최초 → 마지막 고유 변경, 커밋 수/30일) · ④ 문서 상태 · ⑤ 결합 비용(본체+테스트 줄, 외부 참조 파일, 주요 결합점). 설정 칸은 `기본값 → 현재값`.

**핵심 흐름**

| # | 기능 | 진입점 | 설정 | ② 흔적 | ③ 이력 | ④ 문서 | ⑤ 결합 |
|---|---|---|---|---|---|---|---|
| C1 | 업무 패널·수신함·`office-task`·업무 보내기·내보내기/가져오기 | 좌측 패널, `task` 모드, `work-inbox/` | `workPanelsEnabled` off → **on** | `work-items.json` 99건, 10-13 갱신. `session-summaries/v1/2026-10.jsonl` 94 KB | 10-12 → 10-13, 16/16 | 정본, 실기 인수 잔여 | 2,662+2,104. `automation/cli_transition.rs`(기동 receipt, `<ad>/automation-cli-returns/` 10월 11건)와 봇 주입 표지 `BotPromptArms`를 빌려 씀 |
| C2 | 관찰 서버·Claude/Codex 훅(알림·상태) | `observerEnabled`, `--observer-forward` | off → **on** | `observer/claude/*.settings.json` 114개, 10-13 | 07-12 → 09-11 | subsystem-a 정본(07-20 현행화) | 핵심. `observer/event.rs` 1,100줄이 CLI별 분기를 모두 품음(C33~C38) |
| C3 | Claude 세션 이어하기 | 자동(`--resume`) | — | `claude-resume.json` 10-13 | 07-17 → 07-19, 3 | 정본 | 86+129 |
| C4 | 알림·알림음 | 자동, 소리 탭 | `notifySoundEnabled` on → on | `session-events` notification 1,979건 | 07-09 → 09-05, 22 | 정본 | 260+2,188 |
| C5 | 세션 v1 핸드오프(종료 시 sessiond로 넘김) | 앱 종료/재실행 | — (기본 경로) | `sessiond.sock` 10-13 18:24. `sessiond.log` 0바이트(07-17) | 07-20 → 08-30(tmux 연동), 2 | 정본, v2와 공존 | 603+0(전용 테스트 없음, `real_pty_smoke` ignore). `sessiond/daemon.rs`를 v2와 공유 |

**기록·보기**

| # | 기능 | 진입점 | 설정 | ② 흔적 | ③ 이력 | ④ 문서 | ⑤ 결합 |
|---|---|---|---|---|---|---|---|
| C6 | 세션 로그(전사 30일·2GB) + 보기·회고 | 탭 메뉴, 세션 로그 대화상자 | `sessionLogEnabled` on → on | `session-logs/v1` 552파일 46 MB, 10-13 | 07-25 → 08-29, 12/1 | 정본, **눈검증 대기** | 3,036+1,719, 외부 29. `session_log::transcript` 패닉 이력(A10) |
| C7 | 머리 위 라벨 요약기 | 자동 | `summarizerEnabled` off → **on**(`opencode`) | 요약기 `opencode run`이 30초에 2회꼴로 뜸(§7.A.2) | 07-09 → 09-04, 60/2 | subsystem-c | 2,255+3,489, 외부 27. provider 9종(claude·codex·agy·gemini·opencode·openrouter·pi·kilo·anthropic) 각각 모듈 |
| C8 | 캐릭터 일기·작업 기록 | 일기 대화상자, 자동 작성 | `diaryEnabled` off → **on** | `diaries/` 7파일 10-13, `worklogs/` 10-13 | 07-19 → 07-25 고유, 35/1 | (전용 문서 없음, #56·#65) | 1,696+1,813, 외부 37 |
| C9 | 포스트잇 메모 | 탭 메뉴 | — | `memos/` 7명, 10-12 | 07-30 → 08-22, 5/0 | (전용 문서 없음) | 930+952 |
| C10 | 세션 시간 타임라인·실작업 시간 | 상단 패널 | — | `session-times.jsonl` 700 KB, 10-13(기록은 항상 쌓임, 열람 흔적 없음) | 07-09 → 08-25, 9/0 | (subsystem-c) | 642+598, 외부 10 |
| C11 | 세션 활동 분석(일별 차트·추정 비용) | 하단 바 "분석" | — | 원천 `session-events` 53 MB 10-13. 열람 흔적 없음 | 07-17 → 09-30(단가 갱신), 12/1 | 정본 | 1,392+1,098. 모델 단가표 수동 갱신 필요 |
| C12 | 구독 사용량 표시(위젯·플로트·세션 비용) | 하단 바, 플로트 | `usageFloatEnabled`·`sessionCostEnabled` on → on | 표시 전용, 저장 없음 | 07-17 → 09-07(agy 추가), 31/0 | 정본 | 6,186+3,989(최대 기능). provider 4종(claude·codex·antigravity·gemini) 각자 비공식 경로 |
| C13 | 작업 폴더 브라우저·커밋 로그·git 상태 | 탭 메뉴, 팔레트 | `gitStatusEnabled` on → on | 표시 전용 | 07-19 → 09-09, 14/0 | subsystem-c §10(부분표류 D4~D6) | 4,789+1,753 |
| C14 | Everything(es.exe) 파일 색인 백엔드 | 설정 | `fileIndexBackend` walker → walker | macOS에선 쓸 수 없음(Windows 전용). Windows 흔적 확인 불가 | 07-21 → 07-21(버그 수정), 7/0 | (이슈 #67) | 509+324, 외부 18(`markdown.rs`·workdir 목록 분기) |
| C15 | 문서(마크다운) 팔레트·뷰어 | 팔레트 | — | 열람 흔적 없음 | 07-18 → 09-09, 11/0 | — | 1,812+1,107 |
| C16 | 터미널 버퍼 외부 에디터로 내보내기 | 탭 메뉴 | `externalEditor` system | 흔적 없음 | 07-18 → 07-18, 1 | (#42) | 203+157, 외부 8 |
| C17 | 저장소 일괄 점검 | 오피스 서버 오브젝트 | — | 흔적 없음(저장 없음) | 09-06 → 09-06, 3/0 | 정본 | 478+147, 외부 8 |

**캐릭터·연출**

| # | 기능 | 진입점 | 설정 | ② 흔적 | ③ 이력 | ④ 문서 | ⑤ 결합 |
|---|---|---|---|---|---|---|---|
| C18 | 초상·스프라이트·미니미 편집 + codex 이미지 생성 | 프로필 편집 | — | `portraits/`·`sprites/`·`minimis/` 7명 모두 08-22 이후 갱신 없음 | 07-09 → 08-28, 15/0 | subsystem-b | 1,740+875. codex CLI `image_gen` 의존 |
| C19 | 캐릭터 번들 내보내기/가져오기 | 프로필 메뉴 | — | 흔적 없음 | 07-24 → 08-25(i18n) | (#77) | 소형(번들 타입+참조 파일 합 482줄) |
| C20 | 씬 테마 9종 | 하단 바 | — | 현재값은 렌더러 저장(미확인) | 07-09 → 08-28, 11/1 | subsystem-b | 518+251 |
| C21 | 마스코트 창 + 신호등 조명 | 설정 | `mascotEnabled` off → **on**, `mascotLightsMode` off → **agents** | 설정 켜짐(창 상시) | 07-23 → 09-12, 19/0 | 정본 ×2(file:line 표류 D8) | 2,012+793, 외부 20. Cargo `macos-private-api` feature가 이것 때문에 켜짐 |
| C22 | 이 달의 우수사원 | 하단 바 "시상", 씬 연출 | — (항상) | `awards.json` 14개월분, 10-01에 2026-09 선정. 초상 스냅샷 2026-09.png = 08.png(같은 크기) | 08-24 → 08-26, 6/0 | 문서 "정본", README "구현 중"(S3) | 1,606+1,423, 외부 24(`OfficeScene.ts` 112회, `awards_store.rs`). 봇 주입 시간 차감 규칙으로 봇과 결합 |
| C23 | 알림 대사 TTS(리라이트+ElevenLabs) | 소리·음성 탭 | `ttsEnabled` off → off | `tts-cache/` 85파일 13 MB, 마지막 **08-02**. `tts-keys.json` 09-04. `ELEVENLABS_API_KEY` 미설정 | 07-30 → 08-21 고유, 9/2(타 기능 덕) | 정본, **눈검증 대기** | 2,155+1,336, 외부 33(`settings_store.rs` 80회) |
| C24 | 키보드 타건음 | 소리 탭, 프로필 | `typingSoundEnabled` on → **off** | 프로필 6명이 `keyboardSound` 지정했으나 전역 off | 07-11 → 08-21, 13/0 | — | 981+1,258(sound 전체) |
| C25 | 휴가 모드·출퇴근·책상 배정 | 보스 책상, 하단 바 | `vacationMode` false | `profiles.json` 7명 중 6명 `assignedDeskIndex` | 휴가 07-17, 책상 07-09 → 08-25 | subsystem-b/c | 소형 |
| C26 | 동료 대화(Agent Talk) | 하단 바 대화 로그, `ctl talk` | `talkEnabled` off → off | `talks/2026-08-22.jsonl` 1개(2 KB)뿐 | 08-22 → 08-24, 9/2(타 기능 덕) | 정본, M3 미착수 | 1,322+614, 외부 43(`lib.rs` 53회, `BottomBar`, `ctl`) |
| C27 | 에이전트 호감도 | — | — | 코드 0 | 기획 09-05 | 기획 확정·구현 전(S4) | 없음 |

**opt-in 부가·자동화**

| # | 기능 | 진입점 | 설정 | ② 흔적 | ③ 이력 | ④ 문서 | ⑤ 결합 |
|---|---|---|---|---|---|---|---|
| C28 | 실행 레시피 | 팔레트 | `runRecipesEnabled` off → **on** | `run-recipes/` 4개, 마지막 **09-05** | 09-04 → 09-04, 2/0 | 정본 | 1,509+636, 외부 14 |
| C29 | 터미널 입력 자동화(v1/v2) | 자동화 팔레트 | — | `automations/` 정의 3개(09-09), `automation-runs/` 19건 **09-08~09-09**뿐. `session_started` shell=automation 2건(09-07) | 09-08 → 09-09 고유, 3/1 | 정본 | 4,116+4,829(테스트 최다 축), 외부 35(`appStore` 94회, `AgentTabStrip`). **`cli_transition.rs`는 C1이 씀** — 걷어낼 때 분리 필요 |
| C30 | 봇 모드(Gitea 이슈 폴링) | 프로필 `bot` | — (프로필별) | 4명에 `bot.slug` 설정, `GITEA_TOKEN` 설정됨. 그러나 `bot-state.json` 커서 **07-19** 이후 갱신 없음 | 07-19 → 08-26, 10/0 | 정본 | 1,369+296, 외부 44(`inject.rs`, `recording_events` `origin:"bot"`, `work_send`, awards). "기계 주입 프롬프트" 표지를 C1·C29·웹 원격이 같이 씀 |
| C31 | 작업 중 잠자기 방지 | 시스템 탭 | `keepAwakeEnabled` off → **on** | 설정 켜짐 | 07-21 → 07-21, 1 | (#68) | 387+181, 외부 11 |

**원격·제어**

| # | 기능 | 진입점 | 설정 | ② 흔적 | ③ 이력 | ④ 문서 | ⑤ 결합 |
|---|---|---|---|---|---|---|---|
| C32 | `ctl` CLI 제어(로컬 axum + 승인) | `agent-office ctl` | `cliEnabled` off → **on** | `control-token` 07-19, `control-port` 10-13. 점검 A에서 켬(그 전 상태 불명) | 07-19 → 08-22 고유, 23/1 | 정본 | 2,241+1,569. `vscode-ext`(C40)와 `ide`(C39)가 이 서버를 씀 |
| C33 | 외부 터미널/tmux 세션 붙이기 | `ctl attach`/`detach` | — | `session_started` shell=observed 8건, 마지막 **09-12**(개발일) | 08-16 → 09-12(IDE 관찰 쪽 변경), 11/1 | 정본, **눈검증 대기** | 819+766, 외부 46. `session/external.rs`를 C39와 공유 |
| C34 | tmux 자동 호스팅 | 프로필 `tmuxHost` | 프로필별 | 7명 중 **0명** 설정 | 08-30 → 08-30, 3/0 | 정본 | 366+306, 외부 32(`manager.rs` 74회, control routes) |
| C35 | 웹 원격(tailnet 브라우저) | 설정 웹 원격 탭, `dist-web` | `webRemoteEnabled` off → off | `web-remote-tokens.json` **08-22** | 08-05 → 09-12(serve 연동), 27/2 | 정본 M1~M3, M4 미착수(S2) | 5,614+3,364(+`src/web` 3,412), 외부 23. **C36의 `serve`가 웹 원격 컨텍스트 위에 섬** |
| C36 | 독립 세션 서버 `serve` + 데스크톱 원격 연결 | `agent-office serve`, 하단 바 "연결" | `remoteServerConnectionEnabled` off → off | `remote-connection.json` **09-12**(개발일) | 09-12 → 09-12, 4/0 | `remote-server-design.md`(README 인덱스 밖, D9) | 1,717+394, 외부 15. 렌더러에 `RemoteApp`·`setTauriApiDelegate` 대체 경로 |
| C37 | 세션 v2 상시 브로커 | env `AGENT_OFFICE_SESSION_BROKER=v2` | 기본 off | 셸 env·launchctl·실행 중 앱 env 모두 **미설정** | 07-17 → 09-12, 15/0 | 정본, 기본 off(S7) | 2,200+1,703, 외부 16. `sessiond` 데몬을 C5와 공유 → 걷어내면 "축소" 형태 |

**IDE·VSCode**

| # | 기능 | 진입점 | 설정 | ② 흔적 | ③ 이력 | ④ 문서 | ⑤ 결합 |
|---|---|---|---|---|---|---|---|
| C38 | (점검 B D2 보정) 앱↔앱 뷰어 `peer/viewer.rs` | — | — | 코드 없음. **`<ad>/peer-tokens.json`(08-05)만 남음 — 읽는 코드 0건** | 폐기 | archive | 없음. §3.2 후보 목록에서 빼야 함 |
| C39 | IDE 세션 관찰 + IDE 말투(persona) | 하단 바 "IDE 세션" | `ideConnectionEnabled` off → off | shell=observed 마지막 09-12(개발일) | 09-12 → 09-12(하루에 9커밋), 9/0 | **실험 구현**(스파이크) | 2,760+2,181(+`scripts/ide-session-spike/`, `npm run spike:ide*`), 외부 38(`external.rs` 79회) |
| C40 | VSCode 확장 `agent-office-logs` V1 | `vscode-ext/` 별도 패키지 | — | **미설치**. 설치된 것은 다른 확장 `bugcaptor.agent-office-companion-0.1.0`(09-12, 이 저장소 이력에 없음) | 08-18 → 08-18, **1커밋** | V1 눈검증 대기, V2·V3 미착수 | 1,574+598, 앱 코드 참조 0(제어 서버 API만 사용). vitest 1.6 별도 의존 |

**CLI별 어댑터** (app 내 실사용 기준은 `session-events`의 `usage.tokens.model`, CLI 자체 기록은 보조)

| # | CLI | 앱 쪽 구현 | ② 흔적 | ③ 이력 | ④ 문서 | ⑤ 결합 |
|---|---|---|---|---|---|---|
| C41 | Claude | 훅 settings 주입, 플러그인, 사용량 실시간·폴백, 요약 provider | claude-opus-5-5 10월 7,523건 | 상시 | 정본 | 핵심 |
| C42 | Codex | 훅, rollout 사용량, 요약 provider, 이미지 생성 | gpt-6.1-sol 10월 3,032건 | 상시 | 정본 | 핵심 |
| C43 | Antigravity(`agy`) | 셸 훅·hooks 파일, 이어하기, 실시간 사용량(`agy` print 모드 실행), 요약 provider | 앱 내 gemini 모델 마지막 **09-15**, `agy-resume.json` 09-15. agy 자체 기록 마지막 대화 10-07(앱 밖일 수 있음) | 07-30 → 09-07, 8/0 | 정본 | 869+783 + `observer/event.rs` agy 분기 148회(외부 44, 최다) |
| C44 | Pi | 확장 주입·셸 래퍼, 자동화 모델/thinking, 요약 provider | `~/.pi/agent/sessions` 10월 22개(10-13 agent-office 포함). 앱 세션에서 쓴 것인지 구분 못 함 | 07-12 → 09-10, 5/0 | **부분표류**(D7) | 245+131 + 외부 24 |
| C45 | Kilo | `KILO_CONFIG` 플러그인 주입, 래퍼, IDE 사용량, 요약 provider | `kilo.db` 세션 25개, 마지막 **10-01**(deepseek 이벤트 10-01과 일치). 09-10~11 다수는 개발 스파이크 디렉터리 | 09-10 → 10-13(안내문), 5/2 | 정본 | 853+540, 외부 30(`event.rs` 54회) |
| C46 | Gemini CLI | 실시간 사용량(`gemini_live.rs` 800줄, 비공식 엔드포인트), 요약 provider | gemini CLI 자체 기록 마지막 **2026-04-19**. 파일 머리 주석: 개인 무료 티어는 08월 Antigravity 이관 뒤 `Ineligible`로 값이 안 나오는 것이 정상 | 07-30 → 08-30, 5/0 | usage-design §12 | 943+466, 외부 30(`usageView`·`pricing`) |
| C47 | 요약 provider opencode·openrouter | 요약기 하위 | opencode 현재 사용 중, `OPENROUTER_API_KEY` 설정됨 | 08-21 → 09-04 | — | 각 160~190줄 |

#### 7.C.2 D 입력 — 흔적 기준 정렬 (판정 아님)

- **앱 데이터에 최근(10월) 흔적**: C1~C10, C12, C21(설정), C22(자동 선정), C31(설정), C32(점검 A가 켬), C41·C42, C45(10-01).
- **마지막 흔적이 개발·도입 직후에서 멈춤**: C23 TTS(08-02), C26 동료 대화(08-22 하루), C28 실행 레시피(09-05), C29 터미널 자동화(09-09), C30 봇 모드(07-19), C33 외부 붙이기·C39 IDE 관찰·C36 원격 연결(모두 09-12 개발일), C35 웹 원격(08-22), C43 agy(앱 내 09-15).
- **흔적 0**: C34 tmux 호스팅(설정한 프로필 0), C37 v2 브로커(opt-in env 없음), C40 VSCode 확장(미설치), C46 Gemini(로컬 CLI 04-19 이후 미사용), C27 호감도(코드 없음), C38 peer 잔여 파일.
- **저장 흔적이 원래 없음 → 사용자 응답 필요**: C11, C13~C17, C19, C20, C25, C44(앱 밖 사용과 구분 불가).
- **결합 비용이 큰 축(외부 참조 30+ 또는 본체 4,000+)**: C29 자동화(C1이 일부 모듈을 씀), C30 봇(주입 표지 공유), C35 웹 원격(C36이 의존), C26 동료 대화, C39 IDE 관찰(C33과 `external.rs` 공유), C43 agy(`event.rs` 분기 최다), C12 사용량(provider별 비공식 경로).

#### 7.C.3 부수 관찰 (E 입력)

- `<ad>/peer-tokens.json`은 읽는 코드가 없는 고아 파일이다(C38).
- 점검 A 시험 잔여: `<ad>/worklogs/atest-probe*.json`, `<ad>/session-logs/v1/atest-probe*/`가 남아 있었다(§7.A.3 정리 범위 밖) → 사용자 승인으로 삭제함(2026-10-13).
- `~/.gemini/antigravity-cli/log/`에 print 모드(프롬프트 6자) 실행 로그가 하루 60~150개, 누적 2,134개·52 MB 쌓여 있다. 앱의 agy 실시간 사용량 조회(`usage/antigravity_live.rs`, 실행당 `agy` 기동)로 보이나 호출 주기는 확인하지 않았다(성능·자원 관점).
- `diaryEnabled`·`summarizerEnabled`는 크레딧을 쓰는 opt-in인데 둘 다 켜져 있다 — 요약기 비용은 §7.A.2의 `opencode run` 빈도와 함께 E에서 본다.
- `automation-cli-returns/`는 이름과 달리 10월 항목이 업무 보내기(C1)의 기동 receipt다. 자동화(C29)를 정리할 때 디렉터리 이름·모듈 위치를 같이 정해야 한다.

### 7.D 제거 대상 확정 (2026-10-13, 사용자 인터뷰 판정)

§7.C.1의 기능별 신호를 4개씩 묶어 사용자에게 묻고 §3.3 판정을 받았다. 진행 중에 사용자가 "gemini와 agy 사용량은 계측기에서 아예 빼고 CLI 연결은 남긴다"고 지시해서, 처음 '제거'로 답한 C46을 '축소'로 다시 확인받았다. 같은 지시에 따라 C43(처음 답은 유지)과 C12도 '축소'로 바꿨다.

#### 7.D.1 판정

| 판정 | 기능 |
|---|---|
| **제거** | C23 TTS, C26 동료 대화, C27 호감도(기획만, 문서 archive), C29 터미널 입력 자동화, C30 봇 모드, C33 외부 터미널/tmux 붙이기, C34 tmux 자동 호스팅, C35 웹 원격, C36 `serve`·데스크톱 원격 연결, C37 세션 v2 브로커, C39 IDE 세션 관찰, C40 VSCode 확장 `vscode-ext/` |
| **축소** | C43 agy·C46 Gemini: 실시간 사용량 계측만 제거하고 CLI 연결(훅·이어하기·요약 provider·감지)은 유지. C12 구독 사용량: 위 둘의 provider가 빠지고 claude·codex만 남음 |
| **보류** | C28 실행 레시피 |
| **유지** | C1~C11, C13~C22, C24, C25, C31, C32, C41, C42, C44, C45, C47(opencode·openrouter 모두) |
| 처리 완료 | C38 앱↔앱 뷰어 잔여: 읽는 코드가 없는 `<ad>/peer-tokens.json`을 사용자 승인으로 삭제함(2026-10-13). §3.2 후보에서 뺀다 |

#### 7.D.2 기능별 제거 태스크 (상위 G `t-fb1d64e4`, 모두 D `t-b3381017` 뒤)

| 기능 | 태스크 | 추가 선행 | 이유 |
|---|---|---|---|
| C37 v2 브로커 | t-7c0707d4 | — | sessiond는 v1과 공유하므로 v1이 쓰는 부분은 남김 |
| C34 tmux 호스팅 | t-7bcdfc0a | C37 | 세션 매니저를 같이 건드림 |
| C39 IDE 관찰 | t-839f59c8 | — | |
| C33 외부 붙이기 | t-c2f13d3a | C39, C34 | `session/external.rs` 공유, 매니저 공유 |
| C36 `serve` | t-63eb83fd | — | |
| C35 웹 원격 | t-5ba293f0 | C36 | `serve`가 웹 원격 컨텍스트 위에 섬 |
| C30 봇 | t-b7948eb9 | — | 주입 표지(`BotPromptArms`)는 C1이 쓰므로 남기고, 우수사원의 봇 시간 차감 규칙 정리 |
| C29 자동화 | t-5521dd4b | C30 | `cli_transition.rs`·`automation-cli-returns/`는 C1 쪽으로 옮겨 남김 |
| C26 동료 대화 | t-3c623ead | — | |
| C23 TTS | t-be4fc2c6 | — | |
| C40 VSCode 확장 | t-013c2b3c | — | AGENTS.md·§1의 vscode-ext 검증 안내도 갱신 |
| C27 호감도 | t-325c7ff6 | — | 문서만 |
| C43 agy 사용량 | t-4992ad62 | — | agy print 모드 로그 누적(§7.C.3)이 멈추는지 확인 |
| C46 Gemini 사용량 | t-c2d60217 | C43 | 사용량 파일 공유 |

### 7.G 제거 실행 (2026-10-14~)

§3.4 절차대로 기능 하나당 로컬 커밋 하나. 하위 호환: 설정 파일의 사라진 필드는 serde가 무시한다(`AppSettings`에 `deny_unknown_fields` 없음 — 제거 커밋마다 확인).

| 기능 | 태스크 | 결과 |
|---|---|---|
| C27 호감도 | t-325c7ff6 | 문서만 `archive/`로 이동(코드 0) |
| C40 VSCode 확장 | t-013c2b3c | `vscode-ext/` 패키지·`.gitignore` 항목 삭제, 계획 문서 archive. 앱 코드 참조 0이라 코드 변경 없음. 제어 서버 `/v1/*`·`/v1/display/*`(설치된 별도 확장 companion이 씀)는 유지 |
| C36 `serve`·원격 연결 | t-63eb83fd | `remote_server.rs`(serve 모드)·`remote_client/`(원격 창 IPC 9개)·`remoteServerConnectionEnabled` 설정·렌더러 원격 창(`RemoteApp`·`remoteApi`·`isRemoteWindow` 분기·`tauriApi` delegate)·i18n `remote` 네임스페이스·`scripts/serve.sh`·스모크 스크립트 삭제. C37보다 먼저 했다(`serve`가 v2 브로커 `new_strict`를 씀). serve 전용 웹 원격 내부(headless·journal·`office.*` RPC)는 C35에서 모듈째 제거 |
| C35 웹 원격 | t-5ba293f0 | `webremote/`(serve 전용 headless·journal 포함)·`ipc/commands/{web_remote,tailscale}.rs`·`src/web`·`dist-web`·`vite.web.config.ts`·`npm run web:*`·`webRemote*` 설정 3개·페어링 승인 창·`WebRemoteSection`·i18n `settings.webRemote`/`pairing`/tailscale 오류 키 삭제. 웹 원격만 쓰던 `CompositeEvents`·출력 tap(`OutputTap`)·`InjectSource::WebRemote`·Cargo `rust-embed`/`mime_guess`/`tokio-tungstenite`/`futures-util`/axum `ws` feature도 제거 |
| C37 v2 브로커 | t-7c0707d4 | `session/{broker_pty,handoff_broker}.rs`·sessiond v2 메시지(Spawn/DataAttach/Attach/Resize/Wait/KillAll/UpdateSnapshot)·데몬 브로커 테이블·`SessionManager.broker_mode`/세션 `broker_*` 필드·`PtySpawnOptions`의 브로커 전용 필드·IPC `session_broker_mode`/`upload_session_snapshots`·렌더러 주기 스냅샷 업로드와 `renderedBytes` 회계·Cargo `flate2` 삭제. `PROTO_VERSION` 2→1: 데몬이 Hello를 `min`으로 수락하므로 업데이트 직후 떠 있는 옛 데몬(proto 2)과도 v1으로 통하고, 옛 데몬의 `broker`/`attached` 필드는 serde가 무시한다(회귀 테스트 추가). `subsystem-a-sessions.md`의 브로커 서술은 H에서 정리 |
| C34 tmux 호스팅 | t-7bcdfc0a | `session/tmux_host.rs`·`SessionManager`의 tmux 러너/호스팅 분기·`Session.hosted_tmux`·`AgentProfile`/`CreateSessionRequest`/`SessionOpts`의 `tmuxHost`·프로필 체크박스·i18n `terminal.tmuxHost*` 삭제. 레시피 파일 이름 규칙(`dir_slug`)은 기존 파일 호환을 위해 `run_recipes/paths.rs`로 그대로 옮김. 저장된 프로필의 `tmuxHost`는 serde가 무시 |
| C39 IDE 관찰·말투 | t-839f59c8 | `ide_sessions/`(Kilo IDE 사용량 포함)·`ipc/commands/ide_sessions.rs`·제어 서버 `/v1/observed/*`·`session/external.rs`의 관찰 연결 부분·`ideConnectionEnabled`와 CLI의 opt-in 보호 분기(`preserve_connection_opt_ins`)·마스코트의 IDE 포커스 이동·렌더러 `ide/`·하단 바 "연결" 버튼·설정 `ConnectionSection`·`scripts/ide-session-spike*` 삭제. 표시 API는 설치된 companion 확장과의 호환을 위해 `focus`의 `intent`를 무시하도록 바꾸고 `ide-session-focus` 표지를 뺐다. 옛 settings.json의 제거된 키들이 읽히는지 회귀 테스트 추가 |
| C33 외부 붙이기 | t-c2f13d3a | `ctl attach`/`detach`(+`--tmux`)·`/v1/attach`·`/v1/detach`·`control/tmux.rs`·`session/{external,attach_script}.rs`·`SessionManager.externals`와 5초 PID 스윕·IPC `detach_external_session`·`SessionStateEvent.external`·렌더러 외부 세션 안내 패널과 `SessionRuntime.kind` 삭제. 관찰 연결용이던 `observed_lifecycle` 락은 create·dispose·on_exit 순서 보장으로 남겨 `lifecycle`로 이름을 바꿈. C39가 남긴 `.ide-persona-*` CSS도 정리 |
| C30 봇 모드 | t-b7948eb9 | `src-tauri/src/bot/`·IPC `bot_*`·`AgentProfile.bot`/`BotConfig`·`BotStatus`류와 계약 픽스처·자동화/업무 보내기의 봇 상호 배제 분기·`GITEA_*` env 캡처·렌더러 봇 메뉴/오버레이/배지/입력 잠금(`isBotDriven`·`imeBridge.inputBlocked`)·맨 셸 가드 확인창·프로필 봇 입력칸·i18n 삭제. 주입 헬퍼 `single_line`/`INJECT_SUBMIT_DELAY_MS`는 `session/inject.rs`로 옮김. 기계 주입 표식(`BotPromptArms`·`origin:"bot"`)은 업무 보내기·자동화·동료 대화가 계속 다는 것이라 유지(값 이름은 저장 기록 호환). 우수사원의 차감 규칙도 그 표식 기준이라 유지하고, 열 제목만 "봇 제외"→"자동 입력 제외"로 바꿈 |
| C29 터미널 자동화 | t-5521dd4b | `src-tauri/src/automation/`(정의·러너·저장소·마커)·IPC `automation_*` 15개·`Automation*` 타입과 계약 픽스처/테스트·렌더러 자동화 팔레트·배너·타임아웃 결정창·탭 배지/폴링·스토어 슬라이스·부팅 시드·i18n 삭제. 업무 보내기(C1)가 쓰던 `cli_transition.rs`와 기동 문장 생성(`launch_command_for`·effort 검증)은 `work_send/{cli_transition,launch}.rs`로 옮기고, 모델 목록 IPC는 `work_agent_models`로 바꿈. 기동 receipt 디렉터리 `automation-cli-returns/`는 일회성 파일이라 이름을 그대로 둠(에러 코드 `automation-*`도 렌더러 카탈로그와 맞물려 유지). 주입 출처는 `InjectSource::{Talk, WorkSend}`만 남김. C30이 남긴 봇 CSS도 정리 |
| C26 동료 대화 | t-3c623ead | `src-tauri/src/talk/`·`control/talk.rs`·`/v1/talk/*`·`ctl talk`(+세션 신원 헤더·롱폴링)·IPC `talk_*`·`TalkEvent`/`talk-message`·`talk*` 설정 3개·`AgentProfile.talkReceive`·세션 유휴 추적(`idle_ms`)·렌더러 대화 로그/설정/하단 바 대화 항목/프로필 수신 체크·오피스 말풍선(`TextBubbleOverlay`) 삭제. `office-task` shim이 함께 쓰던 권한 전용 설정 파일·원자 쓰기 헬퍼는 `office_instructions.rs`로, 세션 배선은 `SessionManager::with_office_settings`로 옮김. 기존 앱 데이터의 `talks/`·`claude-plugin/`과 OS temp의 `office-talk` shim은 읽는 코드가 없는 잔여물로 남는다(E 입력) |
| C23 TTS | t-be4fc2c6 | `src-tauri/src/tts/`(리라이트·ElevenLabs 합성·보이스 캐스팅·CLI 리라이트)·IPC `tts_*`·`tts*` 설정 4개와 마이그레이션·`AgentProfile.voiceId`·렌더러 발화 큐(`voiceQueue`)·`playVoice`·설정 `TtsSection`·프로필 `VoiceField`·백엔드 오류 코드 4개·i18n `settings.tts`/`profile.voice` 삭제. 요약기(Anthropic·OpenRouter)가 같이 쓰던 키 저장소는 `key_store.rs`(`ApiKeyStore`)로 떼어 IPC `api_key_status`/`set_api_keys`로 남김 — 파일 이름 `tts-keys.json`은 저장된 키 호환을 위해 유지하고 옛 `elevenlabs` 칸은 다음 저장에서 사라진다. 설정 탭 이름 "소리·음성"→"소리". Anthropic 키 입력 UI는 TTS 섹션에만 있었으므로 함께 사라졌다(저장값·`ANTHROPIC_API_KEY` env는 계속 쓰임) |
| C43 agy 사용량 | t-4992ad62 | `usage/antigravity_live.rs`(`agy -p /usage` print 모드 조회)·`Provider::Antigravity`·스냅샷 `antigravity`/`antigravityLive` 필드와 계약 픽스처·렌더러 `describeAntigravityLiveStatus`/AG 뱃지/다이얼로그 갈래·i18n `usage.antigravityLive`·`usage.dialog.empty.antigravity` 삭제. 폴링마다(최대 15분 간격) 띄우던 `agy` print 모드 프로세스가 없어져 그 로그 누적도 멈춘다(요약 provider를 agy로 둔 경우의 요약 호출은 남음). agy 훅·이어하기·턴 토큰 집계·요약 provider·감지는 유지 |
| C46 Gemini 사용량 | t-c2d60217 | `usage/gemini_live.rs`(Code Assist `retrieveUserQuota` 직접 조회·Keychain/파일 자격증명 읽기·토큰 갱신)·`Provider::Gemini`·스냅샷 `gemini`/`geminiLive` 필드와 계약 픽스처·`load_usage_snapshot_with_live`의 `home` 인자·렌더러 `describeGeminiLiveStatus`/GM 뱃지/다이얼로그 갈래·i18n `usage.geminiLive`·`usage.dialog.empty.gemini` 삭제. C12 사용량은 Claude·Codex만 남는다. 라벨만 있는 `unknown` 창 표기와 Gemini 요율표(agy 추정 비용이 씀)는 일반 기능이라 유지. Gemini 요약 provider·감지는 유지 |

**G 마무리(2026-10-14).** 14건 모두 로컬 커밋(푸시 안 함). 매 커밋마다 `npx tsc --noEmit`·`npx vitest run --dir src`·`cargo test` 통과, `cargo check --tests` 새 경고 없음.

E·H로 넘기는 것:
- **앱 데이터 잔여물(읽는 코드 없음)**: `talks/`, `claude-plugin/`, OS temp의 `office-talk` shim, peer 파일(C38). `tts-keys.json`은 요약기 키 저장소로 계속 쓰이므로 잔여물이 아니다. `automation-cli-returns/`는 업무 보내기가 이름 그대로 쓴다.
- **UI 공백**: Anthropic API 키 입력칸이 TTS 섹션에만 있었다. 저장된 키와 `ANTHROPIC_API_KEY` env는 계속 쓰이지만(요약기·Claude 모델 목록), 앱 안에서 새로 넣거나 지울 곳이 없다. 필요하면 요약 탭에 OpenRouter처럼 칸을 두는 후속 작업.
- **문서**: `subsystem-*`·`usage-design`·`antigravity-support-design` 등 정본에 남은 제거 기능 서술은 H에서 정리.
