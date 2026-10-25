# 문서 뷰어(`office-md`) 설계

상태: 구현 중 (2026-10-25) — §구성 1 변환기(`doc_view.rs`)·§구성 2 CLI(`doc_view_cli.rs`, `--no-open`·종료 코드, 요청 파일 → 2초 대기 → 직접 열기 폴백)·§구성 3 앱 요청 감시(`doc_view::spawn_request_watcher`, 부팅 때 `gc`와 함께)·§구성 4 배선(shim·PATH·`AGENT_OFFICE_MD`·안내문·권한)·§구성 5 미리보기 버튼(`doc_view_open`, `gui:shot` 확인) 완료. 설치 앱 실기가 남았다. CDN 고정 버전: highlight.js 11.12.0, mermaid 11.17.2. 구현을 마치면 이 문서를 "현재 구조" 매뉴얼로 고쳐 정본으로 올린다(계획 시제·미정 표기 제거, 실제 파일·심볼로 갱신).

작업 중 만든 Markdown 문서를 HTML로 바꿔 OS 기본 브라우저로 연다. Agent Office를 설치하면 사람은 앱 Markdown 미리보기의 버튼이나 세션 터미널의 `office-md`로 바로 쓰고, 에이전트는 시스템 프롬프트 안내문으로 이 명령을 알아 "문서 보여 줘"라는 요청에 최신 문서를 띄운다.

관련 문서: [work-panels-design.md](work-panels-design.md)(업무 수신함 — `office-task` shim·공통 안내문·권한 사전 승인 배선의 원형), [subsystem-c-ui.md](subsystem-c-ui.md)(Markdown 편집기 오버레이).

## 결정 (2026-10-25 사용자 확인)

| 항목 | 결정 |
|---|---|
| 에이전트가 띄우는 시점 | **사용자가 요청할 때만.** 안내문은 도구가 있다는 사실과 쓰는 법만 알린다. 스스로 띄우지 않는다 |
| 사람용 진입점 | 앱 Markdown 미리보기 상단 바의 **브라우저로 열기** 버튼 + 앱이 띄운 세션 터미널 PATH의 `office-md` |
| HTML 위치 | 앱 데이터 아래 캐시 폴더 `doc-view/`. 작업 트리에 파일을 만들지 않는다 |
| 렌더 범위 | GFM(표·체크리스트·각주·취소선·코드 블록) + 코드 하이라이트(highlight.js) + Mermaid. 두 라이브러리는 CDN에서 버전 고정으로 불러오고, 오프라인이면 하이라이트 없는 코드·원문 코드 블록으로 보인다 |

## 왜 이 구조인가

- **`office-task`와 같은 배선.** 실행 파일 서브커맨드 + 공백 없는 OS temp shim + 안내문 덧붙이기 + shim 권한 사전 승인은 이미 Claude·Codex·Pi·Kilo 세션에서 검증된 경로다. 새 배선 개념을 만들지 않는다.
- **변환은 CLI, 열기는 앱.** 변환은 순수 계산이라 어디서 돌아도 같다. 브라우저 실행은 Codex 샌드박스(seatbelt) 안에서 막힐 수 있으므로, CLI는 요청 파일만 쓰고 샌드박스 밖의 앱이 연다. 앱이 없을 때만 CLI가 직접 연다.
- **Rust 변환기.** 렌더러의 `marked`+DOMPurify는 웹뷰 안에서만 돈다. CLI는 GUI 없이 돌아야 하므로 `pulldown-cmark`(GFM 확장)로 변환하고 `ammonia`로 원문 HTML을 거른다. 앱 버튼도 같은 Rust 경로를 IPC로 불러 결과를 하나로 맞춘다.

## 사용법(목표 모습)

```sh
office-md docs/plan.md          # 파일 하나를 변환해 기본 브라우저로 연다
office-md --latest              # 현재 폴더 아래에서 가장 최근에 고친 .md
office-md --latest docs         # 지정 폴더 아래에서 가장 최근에 고친 .md
office-md --no-open docs/a.md   # 변환만 하고 HTML 경로를 출력(스크립트·테스트용)
```

- 성공하면 생성한 HTML 절대 경로를 stdout에 한 줄 찍고 0으로 끝낸다.
- 파일이 없거나 `.md`/`.markdown`이 아니면, `--latest`가 아무것도 못 찾으면 stderr에 사유를 쓰고 1로 끝낸다. 사용법 오류는 2.

## 구성

### 1. 변환기 — `src-tauri/src/doc_view.rs`

- `render_document(source: &Path, markdown: &str) -> String`: 완성된 HTML 문서 하나.
  - `pulldown-cmark` 옵션: 표, 체크리스트, 각주, 취소선, 제목 속성. 제목에는 GitHub식 slug id를 붙인다(`MarkdownEditorOverlay.tsx`의 `headingSlug`와 같은 규칙) — 문서 안 `#앵커` 링크가 동작하게.
  - 원문 HTML은 `ammonia`로 거른다(script·이벤트 속성 제거). 우리가 넣는 스크립트는 거른 뒤에 덧붙인다.
  - 상대 경로 이미지·링크는 원본 파일 폴더 기준 `file://` 절대 URL로 바꾼다. `http(s):`·`mailto:`·`#앵커`는 그대로. `.md` 링크는 원본 `.md`를 가리킨다(브라우저에서 원문으로 열림 — 이어 보기 변환은 범위 밖).
  - ```` ```mermaid ```` 코드 블록은 `<pre class="mermaid">`로 내보낸다.
  - `<title>`은 첫 `# 제목`, 없으면 파일 이름. 머리에 원본 절대 경로와 변환 시각을 작게 보인다.
  - CSS는 문서 안에 넣는다(읽기 폭 제한, 표·코드 스타일). `prefers-color-scheme`으로 라이트/다크 둘 다.
  - CDN(버전 고정): highlight.js(cdnjs, 라이트/다크 테마 CSS 둘), mermaid(jsDelivr, ESM). 다크 모드면 mermaid `theme: "dark"`.
- `cache_path(data_dir, source) -> PathBuf`: `doc-view/<원본 절대 경로 sha1 앞 12자>-<파일 stem>.html`. 같은 문서를 다시 열면 덮어써서 브라우저 탭 새로고침으로 최신이 된다.
- `latest_markdown(dir) -> Option<PathBuf>`: `ignore::WalkBuilder`(`.gitignore`·숨김 존중, `markdown.rs`와 같은 방식)로 `.md`를 훑어 수정 시각이 가장 늦은 것.
- `gc(data_dir)`: 부팅 때 `doc-view/`의 7일 지난 HTML과 처리 못 한 요청을 지운다.

### 2. CLI — `agent-office md` 서브커맨드 (`src-tauri/src/doc_view_cli.rs`)

- `lib.rs`의 모드 분기에 `md`를 `task`·`ctl` 옆에 더한다. GUI·tokio 없이 돈다.
- 앱 데이터 폴더는 env `AGENT_OFFICE_APP_DATA`(세션에 이미 들어감), 없으면 OS 기본 앱 데이터 경로.
- 변환 후 열기:
  1. `doc-view/open/`에 요청 파일(`<시각>-<난수>.json`, `{ "html": "<절대 경로>" }`)을 임시 이름으로 쓰고 이름을 바꾼다(`write_atomic`).
  2. 앱이 살아 있으면(아래 감시가 2초 안에 요청을 지운다) 끝.
  3. 2초 안에 안 지워지면 요청을 지우고 직접 연다(macOS `open`, Windows `cmd /c start ""`, Linux `xdg-open`). 이것도 실패하면 HTML 경로와 함께 1로 끝내 에이전트가 사용자에게 경로를 알려 주게 한다.
  - 요청 파일을 **지우는 데 성공한 쪽이 연다**(앱·CLI 같은 규칙). 2초 경계에서 앱과 CLI가 겹쳐도 한쪽만 연다. 요청을 쓸 수 없으면(폴더 권한 등) 기다리지 않고 바로 직접 연다.
  - 심볼: `doc_view::write_open_request`(이름 `<유닉스 밀리초>-<uuid 앞 8자>.json`), `doc_view_cli::open_via_app`.

### 3. 앱 쪽 열기 — 요청 감시

- 앱 부팅 때 `doc-view/open/`을 1초 간격으로 보는 스레드를 띄운다(업무 수신함 감시와 별개 — 업무 패널이 꺼져 있어도 동작해야 한다).
- 요청의 `html`이 `doc-view/` 안의 실재 파일일 때만 `tauri-plugin-opener`로 기본 브라우저에 연다. 그 밖의 경로는 무시하고 지운다(임의 파일·URL 실행 방지).
  - 검증(`doc_view::resolve_open_request`): 절대 경로, 정규화(`canonicalize`) 결과의 부모가 정규화한 `doc-view/` 자체(하위 폴더·`..`·심볼릭 링크로 빠지는 경로 거부), 확장자 `.html`, 실재 파일. URL·상대 경로·깨진 JSON은 거부.
  - 한 번의 훑기(`doc_view::take_open_requests`)는 `*.json`만 집는다 — `write_atomic`의 임시 파일(`….json.tmp-<uuid>`)은 건너뛰고, 남으면 부팅 `gc`가 60초 뒤 지운다.

### 4. 배선 — shim·PATH·안내문·권한

- **shim**: `doc_view_cli::shim_path()` = `work_inbox_cli::office_shim_path("office-md")` — `office-task` 옆 `<OS temp>/agent-office/bin/office-md`(Windows `office-md.cmd`), 본문은 `exec "<exe>" md "$@"`. shim 헬퍼(`ensure_shim`·`refresh_shim`)는 서브커맨드를 인자로 받는다. 세션 준비마다 `ensure_shim(…, "md")`, 앱 수명 동안 `spawn_shim_keeper(vec![(office-task, "task"), (office-md, "md")], …)` 스레드("office-shims")가 5초마다 되살린다. `SessionManager::with_office_cli(exe)`가 두 shim을 함께 켠다(필드 `task_cli`·`md_cli`).
- **PATH**: 앱이 띄운 세션 env에 `PATH=<shim 폴더>:<앱 프로세스 PATH>`를 싣는다(`manager::prepend_path` — 이미 있으면 빼고 맨 앞에 한 번). shim 폴더가 같아 `office-task`도 PATH에 오른다. 사용자 셸 rc가 PATH를 덮어쓸 수 있으므로 절대 경로 env `AGENT_OFFICE_MD`도 함께 넣는다(agy처럼 안내문을 못 받는 CLI는 이 env만 받는다).
- **안내문**: `office_instructions::doc_view_instructions(shim)`. `appended_prompt(persona, office, doc_view)`의 셋째 조각으로 붙고, 업무 패널 설정과 무관하게 `md_cli`가 있으면 항상 붙는다. Pi·Kilo용 `AGENT_OFFICE_OFFICE_PROMPT`도 업무 안내문 + 문서 뷰어 안내문이다. 내용(영문, 모델용):
  - 사용자가 문서를 브라우저로/HTML로 보여 달라고 할 때만 띄운다. 스스로 띄우지 않는다.
  - 경로를 안 주면 이번 대화에서 만들거나 고친 문서, 그것도 없으면 `<shim> --latest`.
  - 띄운 뒤 한 줄로 알린다. 실패(0 아닌 종료)하면 출력된 HTML 경로(없으면 오류)를 알린다.
- **권한**: Claude는 `lib.rs`의 `office_shims`가 `office-md` shim을 항상 더해 `permissions_fragment`로 `Bash(<shim>:*)`를 사전 승인한다(`office-task`는 업무 패널이 켜졌을 때만). Codex는 `codex_writable_roots_config(user, &[roots])`에 `doc-view/`(`doc_view::cache_dir`)를 더한다 — 수신함은 업무 패널이 켜졌을 때만. 앱 부팅 때 `doc-view/open/`을 미리 만든다(없는 쓰기 허용 경로를 넘기지 않게).

### 5. 앱 Markdown 미리보기 버튼

- IPC 커맨드 `doc_view_open(root, relPath)`(`src-tauri/src/ipc/commands/doc_view.rs`): 다른 Markdown 커맨드처럼 `markdown::resolve_within_root`로 작업 폴더 경계를 검증하고 `.md`·`.markdown`만 받는다. 변환·캐시 쓰기는 CLI와 같은 `doc_view::render_to_cache`이고, 결과를 `tauri-plugin-opener`로 바로 연다(앱 안이므로 요청 폴더를 거치지 않는다). 오류 접두사: `path-outside-root`·`file-not-found`·`not-a-markdown-file`·`open-failed`.
- `MarkdownEditorOverlay.tsx` 상단 바(소스·미리보기 탭 뒤)에 **브라우저로 열기** 버튼(`markdown.openInBrowser`). 툴팁은 평소 `markdown.openInBrowserHint`("기본 브라우저에서 HTML로 엽니다"), 저장 안 된 변경이 있으면 `markdown.openInBrowserSaved`("저장된 내용으로 엽니다…")다. 편집 중 버퍼를 넘기지 않는다 — 결과가 파일과 어긋나지 않게. 실패는 다른 링크 열기처럼 콘솔 경고만 남긴다.
- 문자열은 6개 언어 `workdir.json`에 있다. 가짜 백엔드(`guiHarness/fakeApi.ts`)에 `markdownListFiles`·`markdownReadFile`·`markdownWriteFile`·`docViewOpen` 스텁과 `__guiHarness.markdownStore`·`docViewOpened`가 있고, `scripts/gui-steps/doc-view.mjs`가 편집기를 열어 버튼·툴팁·호출 경로를 찍는다(`npm run gui:shot -- --scenario office --script scripts/gui-steps/doc-view.mjs`).

## 테스트·확인

- Rust 단위: 변환(표·체크리스트·각주·mermaid 블록·제목 id), 원문 HTML 거르기, 상대 링크·이미지 재작성, `cache_path` 안정성, `latest_markdown`(gitignore 존중), CLI 종료 코드·출력, 요청 감시의 경로 검증, shim 본문·안내문·권한 조각·writable_roots.
- vitest: 버튼이 `doc_view_open`을 현재 파일 경로로 부르는지, 더티일 때 툴팁(`MarkdownEditorOverlay.test.tsx`). Rust: `ipc::commands::doc_view` 테스트가 캐시 경로와 경계·확장자 거부를 본다.
- 화면: `npm run gui:shot`으로 버튼 확인. 실기: 설치 앱에서 Claude·Codex 세션에 "방금 문서 브라우저로 보여 줘" → 기본 브라우저가 열리는지, 다크 모드·Mermaid·오프라인 표시.

## 범위 밖

- 브라우저에서의 실시간 갱신(파일 감시·자동 새로고침), 문서 간 `.md` 링크를 변환본으로 이어 주기, 시스템 셸 PATH 설치(`~/.local/bin` 링크).
