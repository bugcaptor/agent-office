# docs/ 인덱스

갱신: 2026-07-20. 원칙(AGENTS.md): **docs/ = 정본 지식, 이슈 = 작업 과정, 위키 = 포털(Home만)**. 정본은 "현재 구조" 서술이어야 하며, 구현 전 스냅샷·사문화된 설계는 `archive/`로 옮긴다(삭제하지 않는 이유: 이슈가 링크하는 결정 근거). 상태 표기는 `정본 | 이력(archived) | 부분표류` 3종.

## 사용 안내

| 문서 | 내용 | 상태 |
|---|---|---|
| [guide.md](guide.md) | 사용 안내 — 주요 기능, Claude/Codex 연동, 봇 모드, CLI 조종, 사용 팁, 개발자용 명령(빌드·서명·테스트) | 정본 (2026-08-21 신설, README에서 이관) |

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
| [web-hosting-design.md](web-hosting-design.md) | 웹 호스팅(kbm #7m) — 브라우저로 접속해 상태 확인·터미널 조작. peer 리스너에 `/web` + WS RPC allowlist, 폰 우선 경량 클라이언트 | 정본 — Phase 1 구현 완료, 실기기 눈검증 대기 |
| [session-analytics-design.md](session-analytics-design.md) | 세션 활동 분석 패널 — 시계열 재구성·일별 스택 차트 | 정본 — 구현 완료 |
| [session-log-design.md](session-log-design.md) | 터미널 전사 상시 기록(30일·2GB) + 세션 로그 보기 + 회고·학습자료 생성 | 정본 — 구현 완료, 눈검증 대기 |
| [usage-design.md](usage-design.md) | 구독 사용량(rate limit) 표시 — 캐시 미러(#22) + Claude 실시간 조회(#33) 통합본 | 정본 — 구현 완료 |
| [bot-mode-design.md](bot-mode-design.md) | 캐릭터 봇 모드 — Gitea 이슈 폴링·프롬프트 주입·PR 완료 판정 | 정본 — 구현 완료(#57/#58/#61) |
| [cli-control-design.md](cli-control-design.md) | `agent-office ctl` 외부 CLI 제어 — 로컬 axum 서버 + 2단계 승인 | 정본 — 구현 완료(#55) |
| [external-session-attach-design.md](external-session-attach-design.md) | 외부 터미널/tmux 세션에 캐릭터 붙이기 — PTY 없는 논리 세션·`ctl attach` eval 스크립트·tmux 클라이언트 스폰 | 정본 — 구현 완료(kbm #2by), 눈검증 대기 |
| [tmux-hosting-design.md](tmux-hosting-design.md) | 프로필 설정으로 tmux 세션 자동 호스팅 — 소환 시 앱이 직접 tmux 세션을 만들고 이름·gc·수명을 관리 | 정본 — 구현 완료(kbm #2pc) |
| [run-recipes-design.md](run-recipes-design.md) | 실행 레시피 — 프로젝트(프로필 cwd)별 실행 방법을 캐릭터가 조사해 앱 데이터에 저장, 팔레트에서 그 캐릭터 세션에 주입. 설정 opt-in | 초안 (2026-09-04 신설, kbm #2rf) — 구현 전 |
| [employee-of-the-month-design.md](employee-of-the-month-design.md) | 이 달의 우수사원 — 월간 결정적 선정·수상 기록 영속화·LLM 수상 소감·시상 화면·씬 연출 | 정본 (2026-08-24 신설, kbm #2hx) — 구현 중 |
| [i18n-design.md](i18n-design.md) | UI 다국어(한국어/영어) — i18next 카탈로그·언어별 프롬프트 프로필·입력 판정 규칙·하드코딩 한글 금지 장치 | 정본 (2026-08-25 신설) — 구현 완료 |
| [claude-session-resume-design.md](claude-session-resume-design.md) | Claude native 세션 ID 캡처·`--resume` 이어하기 | 정본 — 구현 완료(#20) |
| [tts-confirm-line-design.md](tts-confirm-line-design.md) | 알림 대사 TTS — 질문·완료 알림 문구를 캐릭터 말투 대사로 리라이트(API/claude CLI) + ElevenLabs 합성·archetype 보이스 캐스팅/수동 지정, 소리 3분할 설정 | 정본 — 구현 완료, 눈검증 대기 |
| [pi-support-design.md](pi-support-design.md) | Pi(pi.dev) CLI 작업 상태 감지 — Pi 확장 + 셸 래퍼 | 부분표류 — 구현 완료(#8)이나 §1 file:line 근거가 observer 리팩터로 구식. 현행 구조는 문서 §0.5 |

## archive/ — 이력 (이슈 링크 보존용, 갱신하지 않음)

| 문서 | 사유 |
|---|---|
| [archive/session-event-timeseries-design.md](archive/session-event-timeseries-design.md) | 수집 계층 구현 완료 후 코드가 정본이 됨(본문 자인). 소비자는 session-analytics 문서 |
| [archive/usage-limits-design.md](archive/usage-limits-design.md) | `usage-design.md`로 병합됨 (이슈 #22 링크 보존) |
| [archive/claude-usage-live-fetch-design.md](archive/claude-usage-live-fetch-design.md) | `usage-design.md` §6으로 병합됨 (이슈 #33 링크 보존) |
| [archive/peer-session-share-design.md](archive/peer-session-share-design.md) | 앱↔앱 세션 공유(kbm #7k) — 웹 호스팅(#7m)에 흡수돼 완료·폐기. **호스트 인프라(출력 tap·링버퍼·오프셋 회계·페어링·이벤트 미러)는 그대로 계승**됐고 그 설계 근거가 여기 있다. 앱↔앱 뷰어(`peer/viewer.rs`)만 동결 |

## 관련 (docs/ 밖)

- `AGENTS.md` — 작업 규칙 정본 (루트)
- `REBUILD-PLAN.md` — 2026-07-20 기술부채 상환 계획(리팩터 R-1~R-9, 문서 정리 계획 §4). 실행 완료 후 이력화 예정
