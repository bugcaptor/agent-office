# docs/ 인덱스

갱신: 2026-07-20. 원칙(AGENTS.md): **docs/ = 정본 지식, 이슈 = 작업 과정, 위키 = 포털(Home만)**. 정본은 "현재 구조" 서술이어야 하며, 구현 전 스냅샷·사문화된 설계는 `archive/`로 옮긴다(삭제하지 않는 이유: 이슈가 링크하는 결정 근거). 상태 표기는 `정본 | 이력(archived) | 부분표류` 3종.

## 서브시스템 정본 (3)

| 문서 | 내용 | 상태 |
|---|---|---|
| [subsystem-a-sessions.md](subsystem-a-sessions.md) | Rust 백엔드 — 세션·알림·영속화. 파일 레이아웃, 동시성 모델, IPC 커맨드·이벤트 계약, 엣지 케이스, 알림 고도화(#39/#41), 설정 복구(#40) | 정본 (2026-07-20 현행화) |
| [subsystem-b-office.md](subsystem-b-office.md) | 오피스 씬 & 절차적 픽셀 캐릭터 (PixiJS). 결정성 원칙·씬 구조·캐릭터 생성기 | 정본 — §1~§4 코드 스케치는 부분표류(코드가 정본) |
| [subsystem-c-ui.md](subsystem-c-ui.md) | 렌더러 UI·상태관리·TerminalHost + 작업 폴더/커밋 로그 브라우저(§10) | 정본 (2026-07-20 현행화) |

## 활성 design 문서

| 문서 | 내용 | 상태 |
|---|---|---|
| [session-handoff-design.md](session-handoff-design.md) | v1 종료 시점 fd-핸드오프(현재 기본 경로) — 앱 종료 후 터미널 존속·재실행 입양 | 정본 — 구현 완료(#7). v2와 공존 |
| [session-broker-v2-design.md](session-broker-v2-design.md) | v2 상시 브로커(스폰부터 데몬이 PTY 소유) — 크래시 생존, 프로토콜 v2 | 정본 — 구현 완료·기본 off(`AGENT_OFFICE_SESSION_BROKER=v2` opt-in), 결함 #48/#50/#49 수정 완료 |
| [session-analytics-design.md](session-analytics-design.md) | 세션 활동 분석 패널 — 시계열 재구성·일별 스택 차트 | 정본 — 구현 완료 |
| [usage-design.md](usage-design.md) | 구독 사용량(rate limit) 표시 — 캐시 미러(#22) + Claude 실시간 조회(#33) 통합본 | 정본 — 구현 완료 |
| [bot-mode-design.md](bot-mode-design.md) | 캐릭터 봇 모드 — Gitea 이슈 폴링·프롬프트 주입·PR 완료 판정 | 정본 — 구현 완료(#57/#58/#61) |
| [cli-control-design.md](cli-control-design.md) | `agent-office ctl` 외부 CLI 제어 — 로컬 axum 서버 + 2단계 승인 | 정본 — 구현 완료(#55) |
| [claude-session-resume-design.md](claude-session-resume-design.md) | Claude native 세션 ID 캡처·`--resume` 이어하기 | 정본 — 구현 완료(#20) |
| [pi-support-design.md](pi-support-design.md) | Pi(pi.dev) CLI 작업 상태 감지 — Pi 확장 + 셸 래퍼 | 부분표류 — 구현 완료(#8)이나 §1 file:line 근거가 observer 리팩터로 구식. 현행 구조는 문서 §0.5 |

## archive/ — 이력 (이슈 링크 보존용, 갱신하지 않음)

| 문서 | 사유 |
|---|---|
| [archive/session-event-timeseries-design.md](archive/session-event-timeseries-design.md) | 수집 계층 구현 완료 후 코드가 정본이 됨(본문 자인). 소비자는 session-analytics 문서 |
| [archive/usage-limits-design.md](archive/usage-limits-design.md) | `usage-design.md`로 병합됨 (이슈 #22 링크 보존) |
| [archive/claude-usage-live-fetch-design.md](archive/claude-usage-live-fetch-design.md) | `usage-design.md` §6으로 병합됨 (이슈 #33 링크 보존) |

## 관련 (docs/ 밖)

- `AGENTS.md` — 작업 규칙 정본 (루트)
- `REBUILD-PLAN.md` — 2026-07-20 기술부채 상환 계획(리팩터 R-1~R-9, 문서 정리 계획 §4). 실행 완료 후 이력화 예정
