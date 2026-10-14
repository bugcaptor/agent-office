# 헤드리스 GUI 하네스

상태: 정본 (2026-10-14) — 구현 완료.

Tauri 빌드나 창 없이 렌더러를 헤드리스 Chrome에 띄우고 가짜 백엔드로 화면을 확인한다. 에이전트가 UI 변경을 스크린샷으로 확인하거나, 시나리오 스크립트로 클릭·입력을 재현할 때 쓴다. 측정값: 시나리오 하나에 2~4초, 브라우저 실행을 포함해 세 시나리오에 약 8초.

## 사용

```bash
npm run gui:shot                                            # office 시나리오 → /tmp/agent-office-gui/office.png
npm run gui:shot -- --scenario fresh,office,working --out /tmp/shots
npm run gui:shot -- --scenario working --script scripts/gui-steps/terminal.mjs
npm run gui:shot -- --serve                                 # 하네스 서버만 띄우고 URL 출력(DevTools MCP 등으로 직접 조작)
```

옵션: `--size 1280x800`, `--lang ko`(설정 `language`와 브라우저 로캘). 출력은 스크린샷 경로, 가짜 백엔드가 답하지 않은 호출(`unfaked`), 콘솔 경고·에러, 400 이상 HTTP 응답이다. 잡히지 않은 예외(`pageerror`)가 있으면 종료 코드 1이다.

`--script` 파일은 시나리오마다 `export default async ({ page, shot, harness, scenario }) => {}`로 실행된다.

- `page`: Playwright `Page`.
- `shot(name)`: `<out>/<name>.png`를 저장한다.
- `harness.call(fn, ...args)`: 페이지의 `window.__guiHarness` 함수를 부른다. `pushOutput(agentId, text)`, `emitSessionState(agentId, state)`, `notify(agentId, message)`, `emitTauriEvent(event, payload)`가 있다.
- `harness.store((state, ...args) => …, ...args)`: 앱 스토어 액션을 직접 부른다. 예: `s.openTerminal("p1")`.

## 시나리오

`src/renderer/guiHarness/scenarios.ts`. 응답 모양은 Rust serde와 대조되는 `src/shared/contract-fixtures/`에서 가져온다.

| id | 내용 |
|---|---|
| `fresh` | 캐릭터 없음, 첫 실행 온보딩 |
| `office` | 캐릭터 3명(p1 Ada, p2 Bo, p3 Cy), 세션 없음 |
| `working` | p1·p2 세션 실행 중(입양 경로로 시드), p1 터미널 출력, p2 입력 대기 알림 |
| `workpanels` | 업무 패널 켬, 캐릭터별 다른 작업 폴더(Cy는 없음), 상태·선행 작업·미배정이 섞인 업무 7개와 판단자가 멈춘 자동 진행 루트 1개 |

## 구조

| 조각 | 역할 |
|---|---|
| `scripts/gui-shot.mjs` | 프로젝트 Vite 설정으로 개발 서버를 띄운다(포트 1430부터, `tauri dev`의 1420과 분리). 하네스 플러그인을 얹고 `playwright-core`로 시스템 Chrome(없으면 Edge)을 헤드리스로 구동한다. 다운로드는 없다 |
| 하네스 Vite 플러그인 (같은 파일) | ① `ipc/tauriApi.ts` import를 `guiHarness/fakeApi.ts`로 바꾼다. ② 모든 HTML의 맨 앞에 `guiHarness/tauriStub.ts`를 넣는다 |
| `guiHarness/tauriStub.ts` | 앱 모듈보다 먼저 `window.__TAURI_INTERNALS__`를 심는다. `@tauri-apps/api/window`·`event`를 직접 쓰는 모듈(포커스 추적·quitGuard·마스코트 브리지)의 `plugin:*` 호출에 응답하고 이벤트 버스를 흉내 낸다 |
| `guiHarness/fakeApi.ts` | `AgentOfficeApi` 가짜. 구현한 메서드는 계약 시그니처로 타입 검사를 받는다. 구현하지 않은 메서드는 reject하고 `unfaked`에 남긴다. 세션 출력은 백엔드처럼 백로그를 들고 있다가 새 구독에 재생한다. 사람 입력은 메아리친다 |

하네스 파일은 앱 엔트리에서 import되지 않으므로 `vite build` 산출물에 들어가지 않는다. 타입 검사(`tsc`)에는 포함된다.

새 화면이 `unfaked` 호출에 의존하면 `fakeApi.ts`의 `impl`에 메서드를 추가한다. 고정 데이터가 필요하면 contract-fixtures를 먼저 찾는다.

## 범위와 한계

- **Rust 백엔드는 검증하지 않는다.** 백엔드는 `cargo test`, TS·Rust 직렬화 정합은 contract-fixtures 대조 테스트가 맡는다.
- **엔진 차이.** Chromium이지 WKWebView(macOS)·WebView2(Windows)가 아니다. 폰트 렌더링, CSS 세부, IME 차이는 실앱에서 확인한다.
- **WebGL.** GPU 없이 swiftshader(`--use-angle=swiftshader`)로 Pixi 씬을 그린다. 렌더링은 정상이고 `GPU stall due to ReadPixels` 경고는 걸러 낸다.
- **실제 PTY 없음.** 터미널 내용은 시나리오와 `pushOutput`이 넣은 텍스트뿐이다.
- **멀티 창.** `mascot.html`은 별도 페이지로 열 수 있다. main↔mascot 사이의 실제 창 간 이벤트는 오가지 않는다.
- **시간 의존 UI**(자정 롤오버, 월간 시상)는 스크립트에서 Playwright `page.clock`으로 고정한다.

## 검토한 대안 (2026-10)

| 방식 | 판단 |
|---|---|
| 공식 `tauri-driver`(WebDriver) | macOS 미지원 — Apple이 WKWebView용 WebDriver를 제공하지 않는다. Linux(webkit2gtk-driver)·Windows(msedgedriver)만 된다 |
| 앱 내장 WebDriver: `@wdio/tauri-service` + `tauri-plugin-wdio-webdriver`, `danielraffel/tauri-webdriver` | macOS 실앱 E2E가 가능하다. 다만 디버그 빌드에 플러그인을 넣어야 하고, Rust 빌드가 느리고, 실제 창이 뜬다. 앱 데이터가 identifier(`com.bugcaptor.agent-office`)에 묶여 사용자 데이터를 건드리고 single-instance와도 충돌하므로, 테스트용 identifier를 `--config`로 덮어써야 한다. 출시 전 스모크 용도로만 고려한다 |
| CrabNebula macOS WebDriver | 유료 API 키가 필요하다 |
| Linux 컨테이너 + Xvfb + tauri-driver | 실백엔드까지 헤드리스로 돌릴 수 있지만 Docker 안의 Rust 빌드가 무겁고, macOS 전용 기능(AppleScript·알림·서명)이 빠진다 |
| `@tauri-apps/api/mocks`의 `mockIPC` | 명령 단위 모킹이라 응답 타입 검사가 없다. `emitTo`는 지원하지 않는다. 하네스는 `AgentOfficeApi` 단위로 바꿔 끼워 계약 타입을 그대로 쓴다 |

Claude in Chrome·computer use는 실제 창과 사용자 브라우저를 쓰기 때문에 느리고 헤드리스가 아니다.
