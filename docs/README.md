# docs/ 인덱스

상태: 정본 (2026-10-14) — 문서 인덱스.

갱신: 2026-10-14. 원칙: **docs/ = 정본 지식, 이슈 = 작업 과정, 위키 = 포털(Home만)**. 정본은 "현재 구조" 서술이어야 하며, 구현 전 스냅샷·사문화된 설계는 `archive/`로 옮긴다(삭제하지 않는 이유: 이슈가 링크하는 결정 근거). 상태 표기는 `정본 | 이력(archived) | 부분표류` 3종.

## 사용 안내

| 문서 | 내용 | 상태 |
|---|---|---|
| [guide.md](guide.md) | 사용 안내 — 주요 기능, Claude/Codex 연동, CLI 조종, 사용 팁, 개발자용 명령(빌드·서명·테스트) | 정본 (2026-10-14) — 개발자 절이 작업 규칙 정본 |

## 서브시스템 문서 (3)

| 문서 | 내용 | 상태 |
|---|---|---|
| [subsystem-a-sessions.md](subsystem-a-sessions.md) | Rust 백엔드 — 세션·알림·영속화. 파일 레이아웃, 동시성 모델, IPC 커맨드·이벤트 계약, 엣지 케이스, 알림 고도화(#39/#41), 설정 복구(#40) | 정본 (2026-10-14) — 제거 모듈·핸드오프 계약 현행화 |
| [subsystem-b-office.md](subsystem-b-office.md) | 오피스 씬 & 절차적 픽셀 캐릭터 (PixiJS). 결정성 원칙·씬 구조·캐릭터 생성기 | 부분표류 (2026-10-14) — §1~§4 초기 코드 스케치, 구현은 코드 기준 |
| [subsystem-c-ui.md](subsystem-c-ui.md) | 렌더러 UI·상태관리·TerminalHost + 작업 폴더/커밋 로그 브라우저(§10) | 부분표류 (2026-10-14) — 제거 모듈·이동 경로 정리, 초기 코드 스케치 잔여 |

## 활성 design 문서

| 문서 | 내용 | 상태 |
|---|---|---|
| [work-panels-design.md](work-panels-design.md) | 업무 패널 — opt-in 좌우 고정 패널. 좌측 업무 목록(담당 캐릭터·에이전트/모델 지정, 보내기로 CLI 기동+첫 프롬프트, 상태 모니터링), 우측 전 캐릭터 세션 로그(300일·필터·검색). 업무 수신함(`work-inbox/` 파일 규약, `office-task` 명령, CLI 시스템 프롬프트 공통 안내문) 포함 | 정본 (2026-10-14) — 구현 완료, 설치 앱·인증된 CLI 실기 인수 잔여 |
| [antigravity-support-design.md](antigravity-support-design.md) | Antigravity CLI(agy) 작업 상태 감지 — 셸 훅 + SQLite 턴 토큰·추정 비용 수집(실시간 한도 조회 제거) | 정본 (2026-10-14) — 구현 완료, CLI 눈검증 잔여 |
| [macos-signing.md](macos-signing.md) | macOS 코드서명 — 자체 인증서 기반·인증서 관리·DMG 배포 절차 | 정본 (2026-10-14) — 운영 절차 |
| [mascot-lights-design.md](mascot-lights-design.md) | 신호등/마스코트 조명 — 작업·대기 상태 표시, 라벨·얼굴·타일 모드 | 정본 (2026-10-14) — 구현 완료, 화면 재확인 잔여 |
| [mascot-window-design.md](mascot-window-design.md) | 마스코트 창 — 투명 픽셀 창·씬·상태 표식 | 정본 (2026-10-14) — 구현 완료, H에서 화면 재확인하지 않음 |
| [session-handoff-design.md](session-handoff-design.md) | v1 종료 시점 fd-핸드오프(현재 기본 경로) — 앱 종료 후 터미널 존속·재실행 입양 | 정본 (2026-10-14) — 구현 완료(#7), v1만 유지 |
| [session-analytics-design.md](session-analytics-design.md) | 세션 활동 분석 패널 — 시계열 재구성·일별 스택 차트 | 정본 (2026-10-14) — 구현 완료 |
| [session-log-design.md](session-log-design.md) | 터미널 전사 상시 기록(30일·2GB) + 세션 로그 보기 + 회고·학습자료 생성 | 정본 (2026-10-14) — 구현 완료, 화면 확인 잔여 |
| [usage-design.md](usage-design.md) | 구독 사용량(rate limit) 표시 — Claude·Codex 캐시/실시간 조회(#22/#33, kbm #2h8) | 정본 (2026-10-14) — Claude·Codex 구현 완료 |
| [cli-control-design.md](cli-control-design.md) | `agent-office ctl` 외부 CLI 제어 — 로컬 axum 서버 + 2단계 승인 | 정본 (2026-10-14) — 구현 완료(#55) |
| [run-recipes-design.md](run-recipes-design.md) | 실행 레시피 — 프로젝트(프로필 cwd)별 실행 방법을 캐릭터가 조사해 앱 데이터에 저장, 조사 프롬프트는 캐릭터 세션에 주입하고 실행은 별도 프로세스. 설정 opt-in | 정본 (2026-10-14, kbm #2rf) — 구현 완료 |
| [repository-audit-design.md](repository-audit-design.md) | 저장소 일괄 점검 — 오피스 서버 오브젝트에서 작업 기록·프로필 cwd의 Git 상태를 조치 우선순위로 확인하고 외부 터미널 열기 | 정본 (2026-10-14) — 구현 완료, OS 터미널 실기 잔여 |
| [employee-of-the-month-design.md](employee-of-the-month-design.md) | 이 달의 우수사원 — 월간 결정적 선정·수상 기록 영속화·LLM 수상 소감·시상 화면·씬 연출 | 정본 (2026-10-14, kbm #2hx) — 구현 완료 |
| [i18n-design.md](i18n-design.md) | UI 다국어(6개 언어, 프롬프트 ko/en) — i18next 카탈로그·언어별 프롬프트 프로필·입력 판정 규칙·하드코딩 한글 금지 장치 | 정본 (2026-10-14) — 구현 완료 |
| [claude-session-resume-design.md](claude-session-resume-design.md) | Claude native 세션 ID 캡처·`--resume` 이어하기 | 정본 (2026-10-14) — 구현 완료(#20) |
| [pi-support-design.md](pi-support-design.md) | Pi(pi.dev) CLI 작업 상태 감지 — Pi 확장 + 셸 래퍼 | 정본 (2026-10-14) — 구현 완료(#8) |
| [kilo-support-design.md](kilo-support-design.md) | Kilo Code CLI(kilo/kilocode) 작업 상태 감지 — KILO_CONFIG 플러그인 주입 + 셸 래퍼 env 대입 | 정본 (2026-10-14) — 구현 완료, Windows·도구 인자 실측 잔여 |
| [vscode-display-api.md](vscode-display-api.md) | 남아 있는 인증된 `/v1/display/*` 표시 API — 별도 companion 호환 계약(로그 뷰어 패키지는 제거됨) | 정본 (2026-10-14) — 구현 완료 |

## archive/ — 이력 (이슈 링크 보존용, 갱신하지 않음)

| 문서 | 사유 |
|---|---|
| [archive/health-check-cleanup-plan.md](archive/health-check-cleanup-plan.md) | 프로젝트 점검·기능 정리 계획 A~I — 완료된 계획(2026-10-14). 블랙박스 점검·기능 사용 신호·제거 판정(§7.D)·제거 14건(§7.G)·위험 발견과 수정 태스크(§7.E·§7.F)·문서 정리(§7.H)·최종 검증(§7.I)의 결정 근거 보존. 후속은 보드 태스크 |
| [archive/pi-support-design-snapshot.md](archive/pi-support-design-snapshot.md) | 초기 Pi 설계·구버전 이벤트/행번호 근거·버전별 실측 스냅샷(이슈 #8). 현행 계약은 `pi-support-design.md`로 재정리(2026-10-14) |
| [archive/usage-design-provider-history.md](archive/usage-design-provider-history.md) | 제거된 agy·Gemini 실시간 계측 서술을 포함한 통합본(#22/#33, kbm #2h8/#2j4). 제거된 provider 결정 근거 보존, 현행은 Claude·Codex만(2026-10-14) |
| [archive/session-event-timeseries-design.md](archive/session-event-timeseries-design.md) | 수집 계층 구현 완료 후 코드가 정본이 됨(본문 자인). 소비자는 session-analytics 문서 |
| [archive/usage-limits-design.md](archive/usage-limits-design.md) | `usage-design.md`로 병합됨 (이슈 #22 링크 보존) |
| [archive/claude-usage-live-fetch-design.md](archive/claude-usage-live-fetch-design.md) | `usage-design.md` §6으로 병합됨 (이슈 #33 링크 보존) |
| [archive/peer-session-share-design.md](archive/peer-session-share-design.md) | 앱↔앱 세션 공유(kbm #7k) — 웹 호스팅(#7m)에 흡수돼 완료·폐기. 호스트 인프라(출력 tap·페어링·이벤트 미러)는 웹 원격이 계승했다가 웹 원격 제거(2026-10-14)와 함께 사라졌다 |
| [archive/work-inbox-design.md](archive/work-inbox-design.md) | 업무 수신함 구현 계획 — 구현 완료 후 `work-panels-design.md` §업무 수신함으로 병합. 결정 근거(스킬 대신 공통 안내문, 중복 방지 id, 삭제 시점) 보존 |
| [archive/web-hosting-design.md](archive/web-hosting-design.md) | 웹 호스팅(#7m/#7n) 완전판(Phase 1+2) — 2026-08-05 폐기, 웹 원격(#2dz, 이것도 2026-10-14 제거)에 계승. 루트의 낡은 Phase-1 스냅샷(`web-hosting-design.md`)은 삭제됨 |
| [archive/affinity-design.md](archive/affinity-design.md) | 에이전트 호감도(kbm #2rv) — 기획 확정 후 구현 전에 접음(2026-10-14, 점검·정리 §7.D C27). 코드 없음 |
| [archive/vscode-character-extension-plan.md](archive/vscode-character-extension-plan.md) | VSCode 로그 뷰어 확장(kbm #2ce) — V1 `vscode-ext/` 미사용으로 패키지째 제거(2026-10-14, 점검·정리 §7.D C40) |
| [archive/remote-server-design.md](archive/remote-server-design.md) | 독립 세션 서버 `serve`·데스크톱 원격 연결(kbm #2w4) — 미사용으로 제거(2026-10-14, 점검·정리 §7.D C36) |
| [archive/web-remote-design.md](archive/web-remote-design.md) | 웹 원격(kbm #2dz) — tailnet 브라우저 보기·제어. M1~M3 구현 후 미사용으로 제거(2026-10-14, 점검·정리 §7.D C35) |
| [archive/session-broker-v2-design.md](archive/session-broker-v2-design.md) | v2 상시 브로커 — 기본 off opt-in으로 머물다 제거(2026-10-14, 점검·정리 §7.D C37). v1 fd 핸드오프만 남음 |
| [archive/tmux-hosting-design.md](archive/tmux-hosting-design.md) | tmux 자동 호스팅(kbm #2pc) — 설정 프로필 0명으로 제거(2026-10-14, 점검·정리 §7.D C34) |
| [archive/ide-session-spike.md](archive/ide-session-spike.md) | IDE 세션 관찰 연결(kbm #2w6) — 실험 구현 후 미사용으로 제거(2026-10-14, 점검·정리 §7.D C39) |
| [archive/ide-persona-design.md](archive/ide-persona-design.md) | 원본 IDE 에이전트 말투(kbm #2w7) — IDE 관찰 연결과 함께 제거(2026-10-14) |
| [archive/external-session-attach-design.md](archive/external-session-attach-design.md) | 외부 터미널/tmux 세션 붙이기(kbm #2by) — `ctl attach`. 미사용으로 제거(2026-10-14, 점검·정리 §7.D C33) |
| [archive/bot-mode-design.md](archive/bot-mode-design.md) | 캐릭터 봇 모드(#57/#58/#61) — Gitea 이슈 폴링. 미사용으로 제거(2026-10-14, 점검·정리 §7.D C30) |
| [archive/terminal-automation-design.md](archive/terminal-automation-design.md) | 터미널 입력 자동화 v1/v2(kbm #2t9) — 미사용으로 제거(2026-10-14, 점검·정리 §7.D C29). 기동 receipt는 업무 보내기로 이전 |
| [archive/agent-talk-design.md](archive/agent-talk-design.md) | 동료 대화(Agent Talk) — M1+M2 구현 후 미사용으로 제거(2026-10-14, 점검·정리 §7.D C26). shim 권한 배선은 업무 수신함이 이어받음 |
| [archive/tts-confirm-line-design.md](archive/tts-confirm-line-design.md) | 알림 대사 TTS — 리라이트 + ElevenLabs 합성. 미사용으로 제거(2026-10-14, 점검·정리 §7.D C23). 키 저장소(`tts-keys.json`)는 요약기 API 키 저장소로 남음 |

## 관련

- [../README.md](../README.md) — 설치·첫 실행(정본, 2026-10-14)
- [guide.md 개발자 절](guide.md#개발자용-명령) — 저장소 작업 규칙·검증·빌드 캐시 정본. 로컬 `AGENTS.md`는 추적하지 않는다.

머리의 상태 날짜는 문서·코드 대조일이다. 본문의 과거 실측·테스트 날짜는 당시 기록이며 새 검증으로 읽지 않는다. `archive/`의 상태·경로는 당시 이력으로 보존한다.
