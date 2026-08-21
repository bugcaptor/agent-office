# tailnet 웹 원격 뷰/제어 설계 — 채팅 우선 하이브리드

작성: 2026-08-21 (Fable 검토·계획). 개정 1: 채팅 우선(사용자 결정 — "화면 그대로
미러링이 아니라 채팅형으로"). 상태: **구현 진행 중** (kbm #2dz).
브랜치: `feature/web-remote` (워크트리 `.claude/worktrees/web-remote`).
선행: `docs/archive/web-hosting-design.md`(#7m/#7n — 2026-08-05 폐기, 아카이브
복원본) · `docs/cli-control-design.md`(#55) · `docs/session-log-design.md`(전사
tail) · `docs/bot-mode-design.md`(문장 통째 주입 선례).

앱을 띄운 채 tailnet에 웹서버를 호스팅해, 브라우저(폰 포함)로 앱의 상당 부분을
보고 제어한다. **모델은 원격 조종(클로드 리모트 컨트롤류)이다** — PTY·세션의
원천은 항상 호스트 앱이고, 웹은 두 번째 관객+입력자다. 웹 브라우저 안에서
별도 셸이 돌지 않는다.

## 1. 타당성 결론

**가능하며, 리스크 낮음.** 근거 세 가지:

1. **완성 직전까지 갔던 전례가 로컬에 온전히 남아 있다.** #7m Phase 1(웹
   클라이언트·allowlist RPC·페어링 보안)은 구현 완료 상태로 폐기됐고, #7n
   Phase 2(tailscale serve HTTPS·가독성)는 설계 확정 상태였다. 폐기 사유는
   기술 실패가 아니라 범위 판단("과한 것 같다")이었다. 커밋 6개가
   `archive/peer-web-hosting`(=`7f924c6`) 태그로 보존돼 있다(2026-08-21 태그
   재생성으로 gc 방어).
2. **이식 충돌이 실측으로 작다.** 아카이브 기반(`77c990b`) 이후 main의 변화는
   커밋 16개·src-tauri 2,651줄 삽입이고, 최대 커밋 `58b177c`(피어 인프라
   5,556줄)를 현 main에 시험 체리픽한 결과 충돌은 `src-tauri/src/lib.rs`
   1개 파일(커맨드 등록 보일러플레이트)뿐이었다.
3. **현 코드베이스가 원격화 친화적으로 진화했다.** 프런트 Tauri 접점은
   `tauriApi.ts` 단일 파사드로 수렴(직접 import 7파일), 백엔드 이벤트는
   `AppEvents` trait로 추상화(`state.rs:25-31`) — WS 브로드캐스터를 3번째
   구현체로 끼우면 된다. 커맨드 대부분이 `State<AppState>`만 받아 axum 재노출이
   쉽고(control 서버가 이미 11개 라우트로 실증), PTY 출력도
   `OutputSink`(채널+backlog 추상, `session/output.rs:29-72`)라 교체 지점이
   명확하다.

## 2. 채팅 우선 하이브리드 (개정 1의 핵심)

과거 #7m의 주 화면은 터미널 미러(xterm.js)였다. 이번엔 **채팅 뷰가 주 화면,
터미널 미러는 폴백**이다.

### 2.1 왜 채팅인가

- 터미널 미러의 최대 리스크는 전부 "폰 브라우저에서 그리드를 그리고 키를
  받는" 마지막 한 뼘에 있었다: 한글 IME × xterm 숨은 textarea, 소프트 키보드
  뷰포트, 터치 가로 패닝(#7n §8-1·§13-1). 채팅 뷰는 표준 DOM 텍스트 + 표준
  `<input>`이라 이 문제군이 **구조적으로 소멸**한다.
- 재료가 이미 있다:
  - **읽기** — 세션 로그가 PTY 화면이 아니라 **CLI의 JSONL 전사 정본**을
    tail한다(`session_log/agent_transcript/{claude,codex}.rs`,
    `~/.claude/projects/<슬러그>/<sessionId>.jsonl` · codex rollout, 2초 틱).
    사용자 프롬프트/응답/툴 호출이 구조화돼 있어 채팅 버블로 직결된다.
    alt-screen TUI 문제가 원천적으로 없다. VSCode 확장 V1이 tail-follow
    패턴을 실증했다.
  - **쓰기** — 봇 모드가 완성 문장을 `write_input`으로 통째 주입하는 선례.
    웹 채팅 입력칸도 동일: 조합 완료된 문자열 + Enter를 한 번에 주입.
  - **확인 요청** — hook 알림(question)이 이미 온다. 채팅 뷰에 카드로 띄우고
    **퀵 키(1/2/3/y/n/Enter/Esc/↑/↓)** 로 키를 쏜다.
  - **진행 표시** — `activity-event`(prompt/tool)로 "작업 중…" 상태와 툴 사용을
    라인으로 흘린다(토큰 스트리밍 대체).

### 2.2 채팅 뷰의 제약 (수용)

- **턴 단위 갱신** — 전사는 메시지 단위 append(2초 틱)라 토큰 스트리밍이
  아니다. activity-event로 진행감을 보완한다.
- **전사가 없는 세션**(일반 셸, 미지원 CLI)은 채팅화 불가 → 터미널 폴백으로
  자동 안내.
- **전사 포맷 결합** — claude/codex JSONL 스키마에 의존한다. 이미
  세션 로그·VSCode 확장이 같은 결합을 지고 있으므로 새 부채는 아니다
  (파서 재사용, 한 곳에서만 관리).

### 2.3 화면 구성

```
[캐릭터 목록] → [캐릭터 채팅 뷰(주)] ⇄ [터미널 미러(폴백/보조)]
                 ├ 대화 버블(user/assistant/tool 접기)
                 ├ 알림 카드(question/stop/bell) + 퀵 키 바
                 ├ 진행 라인(activity: 🔧 도구, ⏳ 작업 중)
                 └ 채팅 입력칸(표준 input, 통째 주입) + 전송
```

터미널 미러 진입 조건: 전사 없는 세션 · TUI가 프롬프트 밖 상태로 빠졌을 때 ·
사용자가 수동 전환. 미러는 #7m 구현을 그대로 쓰되 주 경로가 아니므로 폰
최적화(폰트 스텝퍼 등)는 후순위.

## 3. 유지하는 과거 결정 (재검증 완료)

아카이브 문서의 결정 A~L 중 아래를 그대로 계승한다.

- **A. 렌더러 번들 재사용 기각 → 경량 웹 클라이언트(`src/web/`)**.
  기각 사유였던 상태 소유권 문제가 그대로다: `store/persist.ts`의 디바운스
  `saveState` 전량 덮어쓰기(last-writer-wins), 백그라운드 라이터 5종의 이중
  실행(턴 로그·라벨 요약 LLM 호출·일기·스냅샷·메모 청소), `tauriApi` 구체
  모듈을 40개 파일이 직접 import(주입 지점 없음). 웹 클라이언트는 로컬 영속이
  없고 서버 push가 유일한 진실이라 이 문제군이 통째로 소멸한다.
- **B. allowlist RPC — 테이블 밖 `cmd`는 무조건 거부.** 확장도 테이블에 한
  줄씩 추가하는 방식만 허용한다.
- **G. 페어링 인증** — PIN 승인 + 쿠키 30일 + 레이트리밋(pair 3회/분,
  실패 10회/10분 → 429).
- **K. `Tailscale-User-Login` 불신** — 직결 클라이언트가 위조 가능. 신원
  헤더는 인증에도 표시에도 쓰지 않는다.
- **H·I·J. tailscale serve 중간안** — 상태 정본은 tailscaled, 앱은 감지·표시 +
  버튼 대행. 전용 HTTPS 포트 47443(변경 가능), `Secure` 쿠키. `funnel` 영구
  금지, `serve reset` 미사용.
- **admin 티어 없음** — 원격 설정 변경은 셀프 락아웃·권한 상승 표면.
- **`session.start`는 저장된 프로필로만** — 웹이 cwd·shell·startupCommand를
  실어 보낼 수 없어야 원격 코드 실행 표면이 되지 않는다. 채팅 입력 주입은
  "이미 떠 있는 세션의 stdin"이므로 같은 경계 안이다(봇 모드와 동일 표면).
- **터미널 크기는 호스트 단독**(결정 D) — 웹발 리사이즈 없음. 채팅 뷰가 주
  화면이 되면서 이 결정의 비용은 더 줄었다.

## 4. 이번에 바꾸는 것

| | 과거(#7m) | 이번 | 근거 |
|---|---|---|---|
| 주 화면 | 터미널 미러(xterm) | **채팅 뷰** — 전사 tail + 표준 입력칸 + 알림 카드/퀵 키. 미러는 폴백 | §2 |
| 피어(앱↔앱) 뷰어 | `peer/viewer.rs`(753줄) 동결 유지 | **걷어낸다** — `PeerClientKind::Peer`, 캐릭터별 공유 토글, 앱↔앱 페어링 UI 포함 | "피어 접속"은 부활 범위 밖. 웹 전용으로 단순화하면 가시성 규칙의 함정도 소멸 |
| 바인드 | LAN/tailnet(전 인터페이스) | **tailnet 우선** — 기본은 tailscale 인터페이스(100.64/10) IP 바인드, 미발견 시 안내 후 폴백(설정으로 전 인터페이스 허용) | 사용자 요구가 tailnet 한정. 노출 표면 축소 |
| 모듈명 | `peer/` | `src-tauri/src/webremote/` 개명 이식 | "피어/웹 호스팅" → **웹 원격(web remote)** |

## 5. 마일스톤

### M1. 부활 — 백엔드 인프라 + 터미널(폴백용)

체리픽: `58b177c`(인프라) → `738526f`(Phase 1) → `6274419`(페어링 다이얼로그)
→ `2a3e724`·`af4650f`(dist-web 빌드 체인). 이후:
viewer 계열 제거(`viewer.rs`·`PeerClientKind::Peer`·공유 토글·앱↔앱 페어링 UI)
→ `peer/`→`webremote/` 개명(설정 필드도 `web_remote_enabled`로) → 16커밋치
현행화(외부 attach와 tap 상호작용·설정 필드·리팩터 드리프트) → tailnet 우선
바인드.

완료 기준: 데스크톱 브라우저에서 페어링 → 캐릭터 목록 → 터미널 미러로 입력
왕복. cargo/vitest/tsc 전 통과.

### M2. 채팅 뷰 (핵심 신규)

- **백엔드**: allowlist에 `transcript.follow`(WS 구독 — 세션 로그
  tail(`agent_transcript`) 재사용, 접속 시 최근 N개 백필+이후 push) ·
  `transcript.list`(과거 세션 전사 목록). 알림·activity를 WS push로
  (`notifications.stream` — `AppEvents` 3번째 구현체).
- **웹**: `ChatScreen`(버블·툴 접기·알림 카드·퀵 키 바·표준 입력칸 →
  `session.send`로 통째 주입) · 전사 없는 세션의 터미널 폴백 안내.
- **파서**: 전사 JSONL → 채팅 항목 정규화는 러스트에서(웹은 표시만) — 포맷
  결합을 백엔드 한 곳에 가둔다.

완료 기준: 폰 Safari에서 채팅으로 지시 → 진행 표시 → 응답 버블 → 확인 요청
카드에 퀵 키 응답까지 왕복.

### M3. tailnet HTTPS (#7n 설계분 선별)

serve 감지·등록/해제 대행(`ipc/commands/tailscale.rs`, `parse_serve_status`
픽스처 테스트, 포트 47443) · `Secure` 쿠키. 가독성 항목(폰트 스텝퍼·패닝·
`Resized` 발행자)은 폴백 화면 한정이므로 후순위로 강등 — 필요 시만.

### M4. 기능 확장

allowlist 한 줄씩. 작업폴더 읽기 6종(status/branch/diffFile/log/commitFiles/
diffCommit)+diff 뷰어(렌더러의 의존성 없는 unified diff 렌더 이식) · 세션 로그
열람 · 활동/통계 · 봇 status/start/stop · (선택) 메모 load/save.

**비범위(영구 또는 재방문 조건부)**: 설정 변경 · 프로필/스프라이트/캐릭터
생성·삭제 · 임의 cwd/명령 스폰 · 오피스 씬·마스코트 · TTS · 파일 쓰기
일반(markdown_write 등) · funnel · 네이티브 다이얼로그/외부 앱 계열
(호스트에서만 유의미한 ~12개 커맨드) · 웹발 PTY 리사이즈.

## 6. 리스크

1. **전사 포맷 드리프트** — claude/codex 버전업으로 JSONL 스키마 변화 시 채팅
   뷰가 빈다. 방어: 파서 단일화(러스트)+미지 항목은 무시하고 원문 접기로 표시,
   터미널 폴백 상존.
2. **턴 단위 지연 체감** — 2초 틱+메시지 단위라 "타자기" 느낌은 없다.
   activity-event 진행 라인으로 보완, 불만 시 tail 틱 단축 검토.
3. **확인 요청 카드의 커버리지** — hook 알림이 못 잡는 TUI 상태(예: 셸 자체
   프롬프트)는 카드가 안 뜬다 → 터미널 폴백 버튼을 항상 노출.
4. **이식 부패** — 16커밋치 API 드리프트. lib.rs 충돌은 작지만 컴파일 후 의미
   검증 필요(특히 외부 attach 세션과 tap).
5. **serve 경유 장수명 WS 끊김**(tailscale #18827) — 20초 ping + 백오프 재접속 +
   offset 복원으로 방어. 토큰은 쿼리스트링 금지(#18651).
6. **다중 클라이언트 동시 입력** — tmux식 "마지막 입력 승" 수용, 문서화만.
7. ~~모바일 IME × xterm~~ — 주 경로에서 제외돼 폴백 한정 리스크로 강등.

## 7. 검증 계획

- cargo: 페어링 레이트리밋 · allowlist 디스패치(미등재 cmd 거부) · tap 게이트 ·
  전사→채팅 정규화 파서(claude/codex 픽스처) · `parse_serve_status` ·
  `cookie_value` Secure.
- vitest: 웹 protocol 인코딩 · ChatScreen 항목 렌더 규칙(버블/카드/접기) ·
  퀵 키 시퀀스.
- 실기기 눈검증: 폰 페어링→채팅 왕복(한글 입력 포함)·확인 요청 퀵 키·터미널
  폴백 전환·토글 즉시 차단(매 요청 `web_remote_enabled` 확인)·serve HTTPS.
