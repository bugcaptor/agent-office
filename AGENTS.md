# AGENTS.md — agent-office 작업 안내

agent-office는 여러 AI 코딩 에이전트의 터미널 세션을 픽셀 오피스 씬으로 시각화하는 Tauri v2 데스크톱 앱이다.

## 원칙

- 장기 유지할 설계와 명세는 `docs/`를 정본으로 삼는다.
- 구현 과정과 임시 메모는 Gitea 이슈에 기록하고, 이슈 종료 전 유효한 결정은 `docs/`에 반영한다.
- Gitea 위키는 프로젝트 안내와 링크를 모은 포털로만 사용한다.
- 원격 정본은 Gitea `origin`이며 GitHub 원격은 미러다.
- 기능 작업은 별도 브랜치와 PR로 진행하고, 워크트리는 `.claude/worktrees/` 아래에 만든다.
- 커밋 메시지는 한국어 한 줄로 간명하게 쓰며 AI를 공동저자로 넣지 않는다.
- 프런트 테스트: `npx vitest run --dir src`
- 네이티브 테스트: `cargo test --manifest-path src-tauri/Cargo.toml`
