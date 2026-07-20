# REBUILD-PLAN — 기술부채 상환·설계 최신화 계획

작성: 2026-07-20 (v0.5.5 기준, main clean). 성격: **재작성이 아니라 부채 상환**.
원칙: AGENTS.md 준수 — docs/ 정본주의, main 직접 작업, 한국어 한 줄 커밋, 임의 푸시 금지.
검증 게이트: 매 단계 후 `npx vitest run --dir src` + `cargo test --manifest-path src-tauri/Cargo.toml`.

---

## 1. 요약 (Executive Summary)

60여 개 이슈를 빠르게 소화하며 기능은 안정적으로 동작하지만, 구조와 문서에 엔트로피가 쌓였다.
백엔드는 파일 단위 모놀리식(최대 `session/manager.rs` 4,865줄)이 문제인데, 조사 결과 그 실체의
절반 이상은 **인라인 테스트**여서 분리 부담이 겉보기보다 작다. 진짜 구조 부채는 (1) 한 파일에
여러 도메인이 뭉친 `ipc/commands.rs`·`workdir.rs`, (2) 세션 존속의 **v1 fd-핸드오프와 v2 브로커
이중 경로 공존**, (3) Rust `types.rs` ↔ TS `shared/types.ts`의 수동 이중 관리다. 문서는 13개 중
다수의 상태줄이 표류했고("Phase 1 구현 중", "코드가 정본" 자인 등), 정본 subsystem 문서 3개가
구현 전 산출물(코드 스켈레톤·태스크 분해)을 그대로 안고 있다. 이 계획은 행위 보존 리팩터링을
우선하고, 설계 변경은 별도 결정 항목으로 격리한다.

## 2. 현황 진단

### 2.1 백엔드 모놀리식 — 절반은 인라인 테스트

전체 Rust 33,246줄. 거대 파일의 실코드/테스트 경계(조사로 확인):

| 파일 | 전체 | 실코드 | 인라인 테스트 | 비고 |
|---|---|---|---|---|
| `src-tauri/src/session/manager.rs` | 4,865 | ~1,388 | ~3,477 (71%) | `mod tests` 1389행~, `mod real_pty_smoke` 3331행~ |
| `src-tauri/src/sessiond/daemon.rs` | 2,025 | ~960 | ~1,064 | `handle_connection` 한 함수가 534–886행(350줄) |
| `src-tauri/src/ipc/commands.rs` | 1,849 | ~860 | ~989 | 커맨드 50여 개, 전 도메인 혼재 |
| `src-tauri/src/notification/hub.rs` | 1,607 | ~580 | ~1,026 | `impl NotificationHub` 단일 impl 104–519행 |
| `src-tauri/src/workdir.rs` | 1,367 | ~1,016 | ~350 | git 러너+파서+diff+커맨드 8개가 한 파일 |
| `src-tauri/src/types.rs` | 1,179 | — | — | 계약 타입 집합(§2.4) |
| `src-tauri/src/session/pty_factory.rs` | 1,041 | — | — | |

즉 "4,865줄 모놀리식"의 실코드는 1,400줄 수준이다. 테스트 분리만으로도 탐색성·diff 가독성이
크게 회복되고, 이는 100% 행위 보존(코드 이동)이다.

책임 혼재의 실례:
- `manager.rs` 실코드 안에: OutputSink(백로그/채널 attach·detach), Session 구조, 생성·설치
  (`create_with_profile`/`install_session`), 입출력·리사이즈·dispose, **v1 핸드오프**
  (`handoff_all`/`handoff_one`), **v2 브로커 핸드오프**(`handoff_all_broker`), 입양 v1/v2
  (`adopt_detached`/`adopt_one`/`adopt_detached_broker`/`adopt_one_broker`), 스냅샷 업로드,
  exit 처리·이벤트 방출, output pump — 최소 4개의 응집 단위가 한 파일.
- `ipc/commands.rs`: 세션(create/dispose/handoff/adopt), 영속화(load/save_state), 초상화·
  스프라이트 미디어, 요약·스프라이트 생성, 설정, control, bot, 외부 앱 열기, 일기·작업로그·
  세션이벤트, resume 목록, usage — 10개 도메인의 커맨드가 평면 나열.
- `workdir.rs`: 제네릭 git 러너(`run_git`), porcelain v2 파서(`parse_porcelain_v2` 등 6개 파서
  함수), diff/히스토리/커밋로그/difftool, 파일 리스팅, tauri 커맨드 8개.
- 루트 평면 모듈 과밀: `lib.rs`에 20개 모듈이 평면 선언(`workdir`, `markdown`, `shell_export`,
  `vscode`, `session_events` 등 루트 단일 파일 다수).

### 2.2 이중 경로: 세션 존속 v1 vs v2

`lib.rs`의 `make_pty_factory`(215행~)가 확인해주듯 브로커 v2는 아직
`AGENT_OFFICE_SESSION_BROKER=v2` **opt-in이고 기본 off**다. 그 결과 manager.rs에:
- v1 fd-핸드오프 경로(`handoff_all`→`handoff_one`, `adopt_detached`→`adopt_one`)와
  v2 브로커 경로(`handoff_all_broker`, `adopt_detached_broker` 등)가 `if self.broker_mode`로 분기
- `#[cfg(unix)]`/`#[cfg(not(unix))]` 중복 시그니처 3쌍(`handoff_all` 666/712행,
  `upload_snapshots` 885/912행, `adopt_detached` 1052/1092행)

v2는 PR #46 머지 + 후속 결함 3건(#48/#50/#49) 수정까지 끝난 상태다. v1 제거는 **설계 변경**이라
이 계획에서는 "결정 항목"으로만 올린다(§3 R-8). 제거 전까지는 두 경로를 각각의 파일로 분리해
공존 비용을 낮추는 것(행위 보존)이 현실적이다.

### 2.3 죽은 코드 — 마커성 부채는 거의 없음

`TODO|FIXME|HACK` 실주석 0건(검색 히트는 전부 테스트 데이터 `"Grep: TODO"`),
`#[allow(dead_code)]` 실사용 1건. 단 그 1건(`pty_factory.rs:505`)의 주석이
"lib.rs의 scaffold `#[allow(dead_code)]`로 crate 전체 침묵"을 언급하는데 **lib.rs에 그런 allow는
이미 없다** — 주석 자체가 표류. 죽은 코드 문제의 실체는 마커가 아니라 §2.2의 이중 경로다.

### 2.4 타입 이중 관리: Rust `types.rs` ↔ TS `shared/types.ts`

- `src/shared/types.ts` 886줄: 타입 60여 개 + `AgentOfficeApi`(전체 IPC 표면) 단일 파일.
- `src-tauri/src/types.rs` 1,179줄: 같은 계약의 Rust 쪽 절반. `SessionState`,
  `NotificationSource`, `ActivityKind`, `BotPhase`, `PersistedState`, `CreateSessionRequest`,
  `OutputChunk` 등이 양쪽에 존재하고 `#[serde(rename_all = "camelCase"/"lowercase")]` 규약에
  기대어 **수동 동기화**된다. workdir 계열(`GitDiffResult`, `GitCommitEntry` 등)은 Rust 쪽이
  `workdir.rs`에 따로 있어 3중 분산.
- 컴파일러가 어긋남을 못 잡는다. 필드 하나 추가 시 사람이 두(세) 곳을 기억해야 함.

### 2.5 프런트엔드 — 구조 양호, types.ts만 비대

TS 총 39,239줄, 테스트 파일 113개. `src/renderer/`는 24개 기능 폴더(office, terminal, workdir,
usage, bot 없음—store 경유, …)로 잘 모듈화, `App.tsx` 6.7K. 구조 개편 불필요.
비대한 것은 `src/shared/types.ts`(886줄) 하나 — 도메인별 분할이 자연스럽다(§3 R-6).

### 2.6 문서 표류

docs/ 13개 문서, 총 6,446줄. 확인된 표류:

| 문서 | 상태 |
|---|---|
| `session-event-timeseries-design.md` | 본문에 **"코드가 정본"** 자인 — 사실상 이력 문서 |
| `session-broker-v2-design.md` | 상태줄 "Phase 1 구현 중" — 실제로는 머지+후속 결함 3건 수정 완료 |
| `session-handoff-design.md` | v1 설계. v2와의 관계(공존 중, v1이 기본) 미기술 |
| `pi-support-design.md` | observer/adapter 리팩터로 상수·필드명 구식(기록 확인됨) |
| `session-analytics-design.md` | 상태줄 "설계 확정, 구현 착수" — 구현 완료 후 미갱신 |
| `subsystem-a-sessions.md`(1,847줄) | §3 "핵심 코드 스켈레톤"(267–1221행, 약 950줄), §7 Cargo.toml 사본, §8 태스크 분해 — 구현 전 산출물이 정본 안에 잔존 |
| `subsystem-b-office.md`(1,419줄) / `subsystem-c-ui.md`(1,194줄) | 동일 패턴(§6/§8 구현 작업 분해 등). c는 §10처럼 이슈별 증분 추가로 갱신돼 상대적으로 살아있음 |
| usage 계열 3개 산개 | `usage-limits` ← `claude-usage-live-fetch`(기반 참조) + `session-analytics` 일부 |

정본이어야 할 subsystem 문서가 "설계 당시 스냅샷"이고, 최신 결정은 개별 design 문서와 이슈에
흩어져 있다 — AGENTS.md의 "장기 유지 설계는 docs/ 정본" 원칙과 실태가 어긋난 상태.

## 3. 리팩터링 로드맵

표기: [행위보존]=동작 불변 보장 목표 / [설계변경]=동작·계약이 바뀜, 별도 결정 필요.
규모: S(반나절), M(1–2일), L(수일). 모두 이슈 단위로 독립 처리 가능.

### R-1. 인라인 테스트 모듈 분리 [행위보존] — 규모 M, 위험 최저, 우선순위 1
- **무엇**: `manager.rs`(3,477줄), `daemon.rs`(1,064줄), `hub.rs`(1,026줄), `commands.rs`(989줄)의
  `#[cfg(test)] mod tests`를 `#[path]` 지정 또는 서브모듈 파일(`manager/tests.rs`,
  `manager/real_pty_smoke.rs` 등)로 이동.
- **왜**: 실코드 가독성 즉시 회복. 이후 리팩터(R-2~R-5)의 diff가 실코드만 남아 검토 가능해짐.
- **어떻게**: 순수 파일 이동 + `use super::*` 경로 조정. 로직 변경 0.
- **안전장치**: 테스트 자체가 검증 수단 — 이동 후 cargo test 전량 통과 확인. 회귀 위험 사실상 없음.
- 파일별 개별 커밋 가능(이슈 1개, 커밋 4개).

### R-2. `ipc/commands.rs` 도메인별 분할 [행위보존] — 규모 M, 위험 하
- **무엇**: `ipc/commands/` 디렉터리로 전환: `session.rs`(create/dispose/handoff/adopt/io),
  `persistence.rs`(state/turns/diary/worklog/events), `media.rs`(portrait/sprite/생성),
  `settings.rs`(+control 승인), `bot.rs`, `usage.rs`, `misc.rs`(외부앱·export·pick_directory).
  기존 `commands::*` 경로는 `pub use` 재수출로 유지해 `lib.rs`의 handler 등록부 무변경.
- **왜**: 50여 개 커맨드 평면 나열이 신규 기능마다 병목·충돌 지점.
- **위험/안전장치**: 커맨드 이름 문자열(매크로 생성)이 바뀌지 않음을 프런트 vitest(ipc 모킹
  113개 테스트 파일)로 교차 검증. 낮음.

### R-3. `workdir.rs` → `workdir/` 모듈화 [행위보존] — 규모 S~M, 위험 하
- **무엇**: `workdir/git_runner.rs`(run_git·sanitize_rel_path·valid_commit),
  `workdir/status.rs`(porcelain v2 파서 계열), `workdir/diff.rs`(diff/히스토리/커밋로그/difftool),
  `workdir/listing.rs`, `workdir/commands.rs`(tauri 커맨드 8개), 타입은 `workdir/model.rs`.
- **왜**: 파서(순수 함수)와 프로세스 실행과 IPC 표면이 한 파일 — 테스트·재사용 경계와 불일치.
- **안전장치**: 파서 함수들은 기존 단위 테스트(~350줄)가 그대로 커버. 낮음.

### R-4. `session/manager.rs` 실코드 분할 [행위보존] — 규모 M, 위험 중
- **무엇**: R-1 이후 남는 ~1,388줄을
  - `session/output.rs` — `OutputSink`·`spawn_output_pump`·`snapshot_offset`
  - `session/handoff_v1.rs` — `handoff_all`/`handoff_one`/`adopt_detached`/`adopt_one` (+cfg 짝)
  - `session/handoff_broker.rs` — `*_broker` 갈래
  - `session/manager.rs` — 생성·설치·io·dispose·exit 처리 코어만 잔존(~600줄 예상)
- **왜**: §2.1의 4개 응집 단위 분리. 특히 v1/v2 경로를 파일로 갈라두면 R-8(v1 제거) 때
  "파일 삭제"로 끝난다.
- **위험/안전장치**: 이 파일은 테스트가 가장 두터운 곳(스모크 포함 3,400줄+). `Session` 내부
  필드 가시성 조정(pub(crate)) 정도 외 로직 불변 유지. 중간 위험이나 커버리지로 상쇄.

### R-5. `sessiond/daemon.rs` `handle_connection` 분해 [행위보존] — 규모 M, 위험 중
- **무엇**: 534–886행 350줄 단일 함수를 요청 오피코드별 핸들러 함수로 분해. 여력이 되면
  `daemon/`으로 링버퍼·세션엔트리·리더 분리.
- **왜**: 브로커 v2의 심장부인데 결함 수정(#48/#50/#49)이 모두 이 근방이었다 — 앞으로도
  손댈 확률 최고 지점.
- **위험/안전장치**: 데몬 프로토콜 테스트 ~1,064줄 존재. 프로토콜 바이트 불변 유지가 원칙.

### R-6. `src/shared/types.ts` 도메인 분할 [행위보존] — 규모 S, 위험 최저
- **무엇**: `shared/types/`로 분할(session, notification, bot, usage, workdir/git, settings,
  markdown, api) 후 `shared/types.ts`는 배럴 재수출로 유지 — import 경로 무변경.
- **왜**: 886줄 단일 파일이 모든 기능의 충돌 지점. 프런트는 이 외에는 건강.

### R-7. 잔부채 청소 [행위보존] — 규모 S
- `pty_factory.rs:501-505`의 구식 주석("lib.rs scaffold allow") 정정 및 allow 필요성 재확인.
- `notification/hub.rs` 단일 impl(415줄) 내부를 관심사(보류·재개감시·dedup) 순으로 재배열
  또는 분리 — R-1 후 판단.

### R-8. 브로커 v2 기본화 + v1 경로 제거 [설계변경] — 규모 L, 위험 상, **별도 결정 필요**
- **무엇**: `AGENT_OFFICE_SESSION_BROKER` opt-in을 기본 on으로 승격 → 안정 확인 기간 →
  v1 fd-핸드오프 경로(`handoff_v1.rs`, sessiond의 v1 입양 갈래) 삭제.
- **왜**: 이중 경로가 manager/daemon 복잡도의 최대 원천(§2.2). 단 v2 눈검증·실사용 안정성
  확인이 선행 조건이고, Windows 상시 브로커(#9 v2 후보)와의 관계 정리도 필요.
- **안전장치**: 2단계(기본화 → 릴리스 하나 이상 묵힌 뒤 제거). R-4로 파일 격리가 선행되면
  제거 diff가 명확해짐. 이 항목만은 사용자 승인 후 착수.

### R-9. Rust↔TS 타입 동기화 안전망 [설계변경(경량)] — 규모 M, **결정 필요**
- **옵션 A(권장, 저위험)**: 계약 테스트 — Rust 쪽에서 대표 타입들의 serde JSON 샘플을
  고정(fixture)으로 내보내고, vitest에서 TS 타입으로 파싱 검증(또는 zod 스키마). 코드 생성
  도입 없이 어긋남을 CI에서 잡는다.
- **옵션 B(중위험)**: `ts-rs` 파생으로 `types.rs`에서 TS를 생성해 `shared/types/generated/`에
  체크인, 수동 타입을 점진 대체. 빌드 파이프라인 추가 부담.
- v0.5.x 안정 단계에서는 A로 시작하고 B는 별도 판단을 권한다.

## 4. 문서 정리 계획

원칙(AGENTS.md 재확인): docs/ = 정본 지식, 이슈 = 과정, 위키 = 포털. 여기에 한 가지를 추가한다 —
**정본은 "현재 구조" 서술이어야 하며, 구현 전 스냅샷은 이력이다.**

1. **상태줄 일제 갱신** (S): 13개 문서 머리에 `상태: 정본 | 이력(archived) | 부분표류` 한 줄
   통일. 즉시 갱신 대상: broker-v2("구현 완료·기본 off·opt-in"), session-analytics("구현 완료"),
   handoff("v1 정본, v2와 공존 관계 명시").
2. **이력 문서 아카이브** (S): `docs/archive/` 신설 후 이동 —
   `session-event-timeseries-design.md`(자인된 사문), 그리고 R-8 완료 시
   `session-handoff-design.md`. 삭제하지 않는 이유: 이슈에서 링크하는 결정 근거이기 때문.
3. **usage 계열 통합** (S~M): `usage-limits-design.md` + `claude-usage-live-fetch-design.md`를
   `usage-design.md` 하나로 병합(전자를 본문, 후자를 §로), analytics 문서는 이벤트 소비자로
   상호 링크만.
4. **subsystem 정본 최신화** (L, 3분할 가능): 각 subsystem-*.md에서
   - 코드 스켈레톤(subsystem-a §3, 약 950줄)·Cargo.toml 사본(§7)·구현 태스크 분해(§8) 제거
   - "현재 파일 레이아웃 + 계약(커맨드·이벤트 문자열) + 핵심 결정과 근거"로 재편.
     특히 §2(렌더러 경계 매핑)와 결정 요약 절은 유지 가치가 높다.
   - R-2~R-4의 새 파일 구조를 반영해 리팩터 완료 직후 갱신(코드→문서 순서 고정).
5. **pi-support-design.md 표류 처리** (S): 상수·필드명 구식 사실을 머리에 명기하고 현행
   observer/adapter 구조 절을 짧게 추가(전면 재작성 안 함).
6. **docs/README.md 인덱스** (S): 정본 3 + 활성 design + archive 구분표. 위키 Home에서
   이 인덱스만 링크.
7. **동기화 규칙**: "구조를 바꾸는 커밋은 해당 subsystem 문서 §을 같은 이슈에서 갱신"을
   AGENTS.md 원칙 목록에 한 줄 추가 제안(사용자 승인 후).

## 5. 비목표 (Non-Goals)

- **기능 추가·UX 변경 없음.** 이 계획의 모든 커밋은 사용자 관점 무변화가 목표.
- **프런트 구조 개편 없음.** renderer 24개 폴더 구조는 건강 — types.ts 분할(R-6)만.
- **프레임워크·의존성 교체 없음.** Tauri v2, PixiJS, zustand, portable-pty 유지.
- **Windows 상시 브로커(#9) 구현 없음.** R-8 결정 시 관계만 정리.
- **대규모 재작성 없음.** manager/daemon은 분해하되 알고리즘·프로토콜 불변. "행위 보존"이
  기본값이고, 설계 변경(R-8, R-9)은 별도 승인 없이는 착수하지 않는다.
- **테스트 재작성 없음.** 기존 테스트는 이동만 하고 그대로 안전망으로 쓴다.

## 6. 실행 순서 제안

리스크 낮고 이득 빠른 순. 각 단계는 독립 이슈·독립 커밋, 완료 조건은 두 테스트 스위트 전량 통과.

| 단계 | 내용 | 규모 | 근거 |
|---|---|---|---|
| 1 | R-1 테스트 분리 (4파일) | M | 위험 0에 가깝고 이후 모든 diff를 읽기 쉽게 만듦 |
| 2 | R-6 types.ts 분할 + R-7 잔부채 | S | 독립적·즉효. 병렬 가능 |
| 3 | R-3 workdir 모듈화 → R-2 commands 분할 | M+M | 파서·커맨드 경계가 명확해 안전. R-2가 R-3의 타입 이동을 이어받음 |
| 4 | R-4 manager 분할 → R-5 daemon 분해 | M+M | 가장 두터운 테스트 위에서 진행. v1/v2 파일 격리로 R-8 준비 완료 |
| 5 | 문서 1·2·3·5·6 (상태줄·아카이브·usage 병합·인덱스) | S~M | 코드와 무관, 언제든 삽입 가능 |
| 6 | 문서 4 subsystem 정본 최신화 (a→c→b) | L | 3~4단계에서 확정된 새 구조를 반영해야 하므로 마지막 |
| 7 | **결정 회부**: R-8 브로커 기본화, R-9 타입 안전망(옵션 A 권장) | — | 설계 변경이므로 사용자 판단 후 별도 이슈로 |

부수 규칙: 단계당 이슈 1개(과정 기록), 커밋은 한국어 한 줄, main 직접, 푸시는 지시가 있을 때만.
