# AGENTS.md — agent-office 작업 안내

agent-office는 여러 AI 코딩 에이전트(Claude Code, Codex, pi 등)의 터미널 세션을 픽셀 오피스 씬으로 시각화하는 Tauri v2 데스크톱 앱이다. 이 파일은 사람과 LLM 에이전트가 공통으로 따르는 프로젝트 규칙의 진입점이다.

## 문서 관리 원칙 (2026-07-17 확정)

혼자 개발하며 LLM과 자주 협업하는 프로젝트이므로, 문서는 세 층으로 역할을 나눈다.

| 층 | 역할 | 내용 |
|---|---|---|
| `docs/*.md` | **남아야 할 지식 (정본)** | 장기적으로 유지할 설계, 규칙, 운영 절차, API 명세. 코드와 함께 커밋·PR 단위로 검토하고, LLM이 코드와 함께 읽고 수정한다. |
| Gitea 이슈 | **흐르는 과정** | 구현할 일, 조사 과정, 임시 메모, 진행 상태, 의사결정의 맥락. 작업이 끝난 뒤에도 유효한 결론만 `docs/`에 반영한다. |
| Gitea 위키 | **입구 (포털)** | 프로젝트 안내, 자주 찾는 링크, 현재 작업 현황과 주요 이슈 연결. 문서 정본을 중복 보관하지 않는다. |

따라서:

- 설계·명세를 새로 쓰거나 고칠 때는 반드시 `docs/`의 해당 파일을 수정하고 커밋한다. 위키에 설계 문서를 만들지 않는다.
- 이슈에는 결론이 아니라 과정을 적는다. 이슈를 닫을 때 남길 가치가 있는 결정·제약이 있으면 `docs/`에 옮긴 뒤 닫는다.
- 위키 Home은 링크 모음으로만 유지한다. 내용이 길어지면 `docs/`로 내릴 신호다.

## 정본 문서 목록 (`docs/`)

- `subsystem-a-sessions.md` — 서브시스템 A: 세션·알림·영속화 (Rust/Tauri)
- `subsystem-b-office.md` — 서브시스템 B: 오피스 씬·절차적 픽셀 캐릭터
- `subsystem-c-ui.md` — 서브시스템 C: Renderer UI·상태관리·TerminalHost
- `session-handoff-design.md` — 앱 종료 후 터미널 존속 v1 (sessiond fd 핸드오프, PR #6 머지)
- `session-broker-v2-design.md` — 상시 브로커 v2 계획 (Windows 지원 후보, 미착수)
- `pi-support-design.md` — pi.dev CLI 작업상태 감지 (구현 완료, 상수·필드명은 observer/adapter 리팩터링 이전 기준)

주의: 오래된 문서 본문에 나오는 `docs/superpowers/...`, `docs/design/archive/...` 경로는 과거 구조의 흔적으로, 현재 저장소에는 없다.

## 저장소·워크플로

- 원격 정본은 Gitea(origin)이며 이슈·PR·위키를 여기서 관리한다. GitHub 원격은 단순 미러다.
- 기능 작업은 브랜치 + PR로 진행한다. 워크트리를 만들 때는 `.claude/worktrees/` 하위에 둔다.
- 커밋 메시지는 한국어 한 줄로 간명하게 쓴다. AI를 공동저자로 넣지 않는다.

## 빌드·테스트

- 프런트: Vite + TypeScript. 테스트는 vitest이며 `npx vitest run --dir src` 로 실행한다.
- 네이티브: `src-tauri/` (Rust, Tauri v2). `cargo test --manifest-path src-tauri/Cargo.toml`.
