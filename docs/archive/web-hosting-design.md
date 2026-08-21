# 웹 호스팅 설계 — 브라우저로 접속해 작업하기 (kbm #7m, #7n)

작성: 2026-08-05 (Fable 설계 + Opus 구현). 상태: **Phase 1 구현 완료 · 실기기
눈검증 대기 / Phase 2(#7n) 설계 확정 · 구현 대기**. 선행:
`docs/archive/peer-session-share-design.md`(#7k — 완료·폐기, 호스트 인프라는
여기로 계승). Phase 2는 §9~§11.

앱을 띄운 채 선택적으로 호스팅을 켜면, 같은 tailnet/LAN의 **브라우저**로 접속해
상태를 보고 터미널에 개입한다. **주인 의미론** — 내 기계를 내가 원격 조종하는
것이다(#7k의 손님 의미론과 반대).

## 1. 결정 요약

| | 결정 | 근거 |
|---|---|---|
| A. 웹 UI | **렌더러 번들 재사용 기각**, `src/web/` 경량 클라이언트 신설 | 아래 §2 |
| B. allowlist | RPC **6개만** (viewer 3 / operator 3), admin 티어는 빈 테이블 | 원격 코드 실행 표면 최소화 |
| C. service 리팩터 | 전면 리팩터 없이 `spawn_session` **하나만** 추출 | 3중 복제(tauri/control/web)를 실제로 유발한 것이 그 하나 |
| D. 터미널 크기 | Phase 1은 **호스트 단독**, `resize`는 테이블에 없음. 웹은 폰트 축소로 수용 | 크기 중재 프로토콜은 P1 범위 밖 |
| E. 정적 서빙 | **rust-embed**(`dist-web/`) + 별도 `vite.web.config.ts` | debug 빌드는 런타임에 디스크를 읽어 `--watch`만으로 개발 가능 |
| F. 커트라인 | 페어링 → 목록/상태/알림 → 터미널 → **소프트 키 보조바** | 보조바가 없으면 "보기"는 되고 "작업"이 안 된다 |
| G. 인증 | 쿠키 **30일**, pair/start IP당 3회/분, 인증 실패 10회/10분 → 429 | 브라우저가 생기며 페어링 표면이 커졌다 |

## 2. 왜 렌더러 번들을 재사용하지 않는가 (A의 근거)

크기는 문제가 아니었다 — 초기 다운로드 ~1.7MB(main 915K + xterm 청크 + 폰트
536K)로 tailnet에서 감당된다. 재사용을 죽인 것은 **상태 소유권**이다:

- `src/renderer/store/persist.ts` — 렌더러가 `PersistedState`(전체 프로필)의
  소유자다. `agents` 변경마다 디바운스 `saveState`로 **전량 덮어쓴다**. 네이티브와
  웹이 동시에 뜨면 last-writer-wins로 서로의 변경을 소거한다.
- `appStore.ts`의 `addAgent`/`clockOut`/`assignDesk`/`removeAgent`/
  `updateAppSettings`/`startBot` — 전부 서버 응답 전에 로컬을 먼저 바꾼다.
- **백그라운드 라이터 이중 실행**: `logSettledTurn`(턴마다 append → 통계 2배),
  `installTaskLabelSummarizer`(**로컬 CLI 호출 = 크레딧 2배**),
  `installDiaryAutoWriter`(일기 2중 생성), 스냅샷 업로더, 메모 청소기.
- `AgentOfficeApi`(80 메서드)는 존재하지만 **주입 지점이 없다** — `tauriApi`가
  렌더러 30여 파일에 구체 모듈로 직접 import된다.

즉 재사용안의 실제 비용은 "80개 WS RPC + 부트스트랩 전 설치기 분기 + 이중 스토어
조정 프로토콜"이고, 화면 4개짜리 경량 클라이언트보다 크고 위험하다. 반대로 웹의
실사용(상태 확인·터미널 개입·세션 재기동)은 **이미 구현된 peer WS 프로토콜이
90%를 덮는다**. 경량 클라이언트는 그 프로토콜의 두 번째 소비자일 뿐이라 —
**로컬 영속이 없고 서버 push가 유일한 진실이라** — 스토어가 두 벌 되는 문제 자체가
소멸한다.

## 3. 아키텍처

```
   브라우저(폰)                         앱(호스트)
┌────────────────────┐            ┌──────────────────────────────────┐
│ src/web (React)    │            │ peer 리스너 (기존, 47800)         │
│  PairingScreen     │  HTTP      │  POST /peer/v1/pair/start|complete│
│  AgentList         ├───────────►│  GET  /peer/v1/ws                 │
│  TerminalScreen    │  WS(쿠키)  │  GET  /web, /web/*  (rust-embed)  │
│  KeyBar            ├───────────►│                                   │
└────────────────────┘            │  peer/web.rs — allowlist 디스패처 │
      ▲                            │    agents.list / notifications.* │
      │ output·restore·이벤트       │    usage.snapshot / session.*     │
      └────────────────────────────┤  PeerHub — tap·링버퍼·스냅샷      │
                                   │  SessionManager / NotificationHub │
                                   └──────────────────────────────────┘
```

**새 서버도 새 포트도 없다.** 기존 peer 리스너에 `/web` 라우트와 `Rpc` 프레임을
얹었을 뿐이다.

## 4. 권한 모델

- **가시성**: `PeerClientKind`가 규칙을 가른다. `Web`은 내 캐릭터 전부(주인),
  `Peer`는 캐릭터별 공유 토글을 켠 것만(손님, #7k 의미론 보존).
  - 함정: 웹이 attach하면 tap이 깔리는데, 그걸 "공유 중"으로 판정하면 앱↔앱
    뷰어에게까지 누출된다. 그래서 앱↔앱 판정은 **영속 공유 목록**
    (`peer-shared.json`)을 본다 — tap은 "받아 적는 중"이지 "공유하기로 했다"가
    아니다. 회귀 테스트로 고정했다.
- **티어**: `ReadOnly`=viewer(읽기), `Input`=operator(조작). **admin 티어는 빈
  테이블** — 원격 설정 변경은 `webHostingEnabled`를 스스로 끄는 셀프 락아웃이자
  권한 상승 표면이다(control 서버가 `cliEnabled`를 막는 것과 같은 논리).
- **`session.start`는 저장된 프로필로만** 스폰한다. 웹이 cwd·shell·
  startupCommand를 실어 보낼 수 없는 것이 이 커맨드가 원격 코드 실행 표면이 되지
  않는 근거다.
- **테이블 밖 `cmd`는 무조건 거부**(`unknownCmd`). 실재하는 Tauri 커맨드라도
  등재되지 않았으면 없는 것이다.
- 정적 자산·RPC 모두 **매 요청** `web_hosting_enabled`를 확인한다(토글 즉시 반영).

## 5. 와이어

기존 `/peer/v1/ws` 한 소켓에 RPC 프레임을 얹는다.

```jsonc
{ "type": "rpc", "id": 7, "cmd": "session.start", "args": { "agentId": "a1" } }
{ "type": "rpcResult", "id": 7, "ok": true, "data": { /* CreateSessionResult */ } }
{ "type": "rpcResult", "id": 7, "ok": false,
  "error": { "code": "forbidden", "message": "조작 권한이 필요합니다" } }
```

에러 코드(폐쇄 집합): `unknownCmd` · `forbidden` · `badArgs` · `notFound` ·
`internal`. 토큰 무효는 RPC까지 오지 못한다(WS 업그레이드 401).

| cmd | 티어 | 비고 |
|---|---|---|
| `agents.list` | viewer | 가시성 규칙에 따라 필터 |
| `notifications.list` | viewer | |
| `usage.snapshot` | viewer | 네이티브와 **같은 스로틀 상태** 공유 |
| `session.start` | operator | 저장된 프로필로만 |
| `session.dispose` | operator | |
| `notifications.clear` | operator | |

인증은 3경로(헤더 / 쿠키 `ao_peer_token` / `Sec-WebSocket-Protocol`) + Origin 검사.

## 6. 구현 맵

**Rust**: `peer/web.rs`(신설 — allowlist·디스패처·정적 서빙) · `peer/protocol.rs`
(`Rpc`/`RpcResult`/`PeerClientKind`/`RpcError`) · `peer/pairing.rs`(`kind`,
`PairRateLimiter`, pending 5개 상한, 토큰 30일) · `peer/mod.rs`(`agent_allowed`/
`build_agents_for` 게이트, 레이트리밋 배선, `/web` 라우트) ·
`ipc/commands/session.rs`(`spawn_session` 추출) · `control/mod.rs`(그 함수 사용) ·
`state.rs`(`live_usage`를 Arc로) · 설정 `web_hosting_enabled`.

**웹 클라이언트**: `src/web/{index.html,main.tsx,App.tsx,ws.ts,protocol.ts,
PairingScreen,AgentList,TerminalScreen,KeyBar,styles.css}` + `vite.web.config.ts`
(`dist-web/`, base `/web/`) + npm `web:build`/`web:dev`.

**네이티브 UI**: 설정 "세션 공유" 섹션에 웹 호스팅 토글 + 접속 주소, 승인
다이얼로그에 클라이언트 종류 배지.

빌드 산출물: 445KB(gzip 122KB) — 데스크톱 번들의 절반 이하.

## 7. 하지 않은 것 (Phase 1)

렌더러 번들 재사용 · admin 티어(설정·봇) · 새 캐릭터 생성과 임의 cwd/shell 주입 ·
터미널 리사이즈 · https 자체 구현(tailscale serve 문서화로 갈음) · 오피스 씬 ·
프로필/스프라이트 편집 · 파일/git 브라우저 · 세션 로그 · 메모 · TTS ·
`peer/viewer.rs`(동결 유지).

## 8. 미해결 / 위험

1. **모바일 IME·소프트 키보드가 최대 리스크.** xterm의 숨은 textarea는 모바일
   한글 조합과 상성이 나쁘다 — KeyBar에 별도 입력칸을 두어 조합 완료 후 통째로
   보내는 전략을 썼지만, iOS Safari 실기기 눈검증 전까지 확정이 아니다.
2. ~~tailscale serve 경유 시 Origin 검사~~ — **해소(확정)**. serve는 유닉스 소켓
   업스트림이 아닌 한 `r.Out.Host = r.In.Host`로 원본 Host를 보존한다
   (`ipn/ipnlocal/serve.go`). Origin 호스트부와 Host가 일치하므로 `origin_allowed`는
   그대로 통과한다. §9-B.
3. **전 캐릭터 attach의 트래픽** — 알림 수신을 위해 접속 시 전 캐릭터에 붙는다
   (활성 탭 외 출력은 폐기). tailnet에선 무해하나 낭비가 실측되면 meta-only
   구독을 Phase 2에 추가.
4. **웹발 상태 변화의 네이티브 반영** — `session.start`는 `session-state`
   이벤트로 네이티브에 닿지만, 네이티브에 세션 런타임 엔트리가 없으면
   `setSessionState`가 no-op일 수 있다(appStore의 prev 부재 가드). 눈검증 항목.
5. **rust-embed release 빌드** — `dist-web` 미존재 시 컴파일 실패.
   `npm run build`가 `web:build`를 체인하고 `.gitkeep`을 커밋해 막았지만 클린
   빌드 경로 확인 필요.

---

# Phase 2 — HTTPS와 가독성 (kbm #7n)

작성: 2026-08-05 (Fable 설계). 상태: **설계 확정 · 구현 대기**.

실사용 후 나온 불만은 정확히 둘이다. ① tailscale serve로 HTTPS를 쓰고 싶다.
② 웹 터미널이 너무 작고, **페이지를 확대해도 커지지 않는다**.

## 9. 진단 — 확대가 상쇄되는 구조

`TerminalScreen.tsx:25`의 `fitFontSize(containerWidth, cols)`는 컨테이너 폭
(**CSS px**)을 호스트 cols로 나눠 폰트를 정하고, `resize` 리스너가 이를
재계산한다. 데스크톱에서 ⌘+로 확대하면 CSS px 뷰포트가 같은 비율로 줄고 → 폰트
CSS px도 같은 비율로 줄어 **물리적 글자 크기가 정확히 원위치한다**. 확대가
수학적으로 상쇄된다. 상한 16px 때문에 창을 넓혀도 그 이상 커지지 않는다.

폰은 여기에 클리핑이 겹친다. 호스트가 200열이면 per-char 1.95px → 하한 7px에
걸리고, 그때 터미널 실폭(≈840px)이 뷰포트를 넘는데 `.term-mount`가
`overflow: hidden`(`styles.css:205`)이라 **오른쪽 절반은 스크롤로도 도달할 수
없다**. 핀치 확대는 `user-scalable=no`로 막혀 있다(iOS는 무시하지만 헤더·KeyBar까지
함께 확대돼 조작이 망가진다).

**덤으로 발견한 결함**: `HostMsg::Resized`는 프로토콜 정의(`protocol.rs:266`)와
수신부(웹·앱 뷰어)만 있고 **발행자가 전무하다**. `SessionManager::resize()`는
PTY만 바꾸고 이벤트를 내지 않는다 — 웹이 붙어 있는 동안 네이티브에서 창을
리사이즈하면 웹 xterm은 낡은 cols로 계속 그려 줄바꿈이 깨진다. Phase 2에서 함께
고친다.

## 10. 결정

| | 결정 | 근거 |
|---|---|---|
| H. serve 관리 | **중간안** — 상태 정본은 tailscaled, 앱은 감지·표시 + 버튼 한 번으로 등록/해제 대행 | 문서화만은 반쪽(포트 충돌·인증서 상태를 매번 사용자가 알아내야 함). 토글 연동 자동관리는 `--bg`가 기계 전역·재부팅 영속이라 앱 설정과 수명이 어긋나고, 조용한 전역 변경은 페어링·control의 명시적 옵트인 원칙과 충돌 |
| I. serve 포트 | **전용 HTTPS 포트 47443**(사용자 변경 가능), 443 서브패스 기각 | 443 루트는 이미 남의 서비스가 점유할 수 있다(개발 기계 실측: `/`→8484). 서브패스는 vite base·쿠키 Path·`ws.ts` 절대경로를 전부 프리픽스 대응시켜야 하는데 전용 포트는 그 비용이 0이고 인증서는 포트 무관 |
| J. 서버측 변경 | **`Secure` 쿠키 + https 주소 안내 2건만**. Origin 검사·바인딩 정책 불변 | Host 보존이 실측 확정(§8-2) |
| K. 신원 헤더 | **`Tailscale-User-Login`을 인증에도 표시에도 쓰지 않는다** | 47800에 직결하는 누구나(tailnet의 다른 기기, 로컬 프로세스) 위조할 수 있다. "loopback발일 때만 신뢰"도 로컬 프로세스에 뚫린다 — control 서버가 로컬 프로세스를 신뢰하지 않는 것과 같은 위협 모델. 위조된 신원 표시는 승인 유도 사회공학 표면 |
| L. 가독성 | **순수 클라이언트 해결(폰트 스텝퍼 + 가로 스크롤)**. 결정 D(호스트 단독 크기) **유지** | 아래 §10.1 |

### 10.1 왜 웹발 리사이즈(결정 D 뒤집기)를 지금 하지 않는가

웹이 `terminal.resize`를 보내는 안은 사용자가 겪은 문제("확대가 안 됨")의 해법이
아니라 **다른 기능**이고, 비용이 크다.

- 네이티브 렌더러는 `TerminalRegistry`의 `activate()`/`refit()`이 탭 활성화와
  컨테이너 리사이즈마다 `fit()+resize`를 재강제한다. 웹이 45열로 줄이면 네이티브가
  탭을 만지는 순간 200열로 되돌아간다 — **줄다리기가 구조적으로 발생한다**.
  결정 D가 막으려던 "네이티브 작업 중 화면이 갑자기 줄어드는 사고"는 실재한다.
- 소유권 중재(활성 탭 감지 IPC, detach 시 복원, 캐릭터별 옵트인 UI)가 통째로 필요.
- 그런데 §9의 상쇄 버그를 고치면 불만이 사라지는지조차 아직 검증되지 않았다.

**재방문 조건**: §11을 배포한 뒤에도 "폰에서 200열을 패닝하며 일하기 어렵다"는
불만이 남으면, 그때 캐릭터별 옵트인 리사이즈를 #7n 후속 이슈로 연다.

### 10.2 serve 운용 세부

- 명령: `tailscale serve --bg --https=47443 http://127.0.0.1:47800`.
  해제: `tailscale serve --https=47443 off`. 상태: `tailscale serve status --json`.
  **`serve reset`은 쓰지 않는다**(남의 매핑까지 파괴). **`funnel`은 영구 금지**
  (공개 인터넷 노출, 같은 포트에서 마지막 명령이 이긴다).
- 포트 프록시는 **sudo 불필요**(non-root 실증). CLI 탐색 순서:
  `/usr/local/bin/tailscale` → `/Applications/Tailscale.app/Contents/MacOS/Tailscale`
  → PATH.
- 충돌: enable 전에 `status --json`을 파싱해 대상 포트가 남의 매핑에 점유돼 있으면
  실패 메시지 + 포트 입력칸으로 유도. **자동 포트 순회는 하지 않는다.**
- 실패 경로: CLI 미발견/tailscaled 미실행/`CertDomains` 비어 있음(관리 콘솔에서
  HTTPS 활성화 필요) 각각을 구분해 안내하고, 최후에는 **복사 가능한 명령**으로 폴백.
- **앱 종료 시 정리하지 않는다.** 남은 매핑은 죽은 포트로의 프록시일 뿐이고
  (47800이 닫혀 있고, 열려도 `web_hosting_enabled` 게이트가 있다), "앱을 다시 켜면
  폰에서 바로 붙는다"가 오히려 의도에 맞다. 대신 UI에 "앱을 꺼도 HTTPS 매핑은
  유지됩니다"를 명시해 `--bg` 영속과 기대의 어긋남을 없앤다.
- 첫 HTTPS 접속은 인증서 발급으로 수 초 걸릴 수 있음을 안내한다.
- 레이트리밋이 serve 경유 시 전부 `127.0.0.1` 버킷으로 합쳐지는 것은 **수용한다**
  (단일 사용자 tailnet, 페어링별 시도 상한이 별도로 있다).

### 10.3 Secure 쿠키

`pair_complete`에서 `X-Forwarded-Proto: https`일 때만 쿠키에 `; Secure`를 붙인다
(직결 클라이언트가 헤더를 위조해도 자기 쿠키에 속성이 하나 붙을 뿐 무해).
부작용 하나를 수용한다: https로 페어링한 브라우저가 나중에 `http://100.x:47800`로
직접 붙으면 Secure 쿠키가 동반되지 않아 재페어링이 필요하다.

### 10.4 가독성 상세 (L)

- 터미널 헤더에 `A−` / `A+` / `맞춤`. 범위 **7~28px**, 1px 스텝.
  값은 `localStorage`(`ao.termFont`) **전역 1개**(캐릭터별 기억은 과함).
  `맞춤`은 오버라이드를 지우고 자동 fit으로 복귀.
- 자동 fit은 **초기값으로 강등**한다. 저장값이 없을 때만 쓰고, 하한을 7→**9px**로
  올린다(7px는 "보이지만 못 읽는" 크기였다). 상한 16px은 초기값에만 남는다.
- `.term-mount`를 `overflow: auto`로 — 폰트가 fit보다 크면 패닝으로 본다.
  §9의 폰 클리핑도 이걸로 함께 고쳐진다.
- **`user-scalable=no`는 유지**한다. 페이지 확대는 고정 헤더·KeyBar까지 확대해
  조작을 망가뜨린다 — 확대의 책임은 폰트 스텝퍼가 진다.
- 회전·`resized` 수신: 오버라이드가 있으면 폰트를 유지하고 스크롤 범위만 갱신,
  없으면 현행대로 fit 재계산.
- 소프트 키보드: rows는 호스트 소유라 불변. `100dvh` + `visualViewport` 현행 유지.
- 읽기 전용 뷰어에게도 스텝퍼는 **노출**한다(읽기야말로 크기가 중요). KeyBar만 숨김.
- **`Resized` 발행자 신설**: `AppEvents`에 기본 no-op `terminal_resized()`를 추가하고
  `SessionManager::resize()` 성공 시 호출, `PeerEvents`가 `is_shared` 게이트 하에
  `HostMsg::Resized`를 broadcast. 웹 수신부는 이미 있다.

## 11. 구현 계획 (순서: L → J → H·I)

**1단계 — 가독성(L)**
- `src/web/termFont.ts` 신설: `fitFontSize`(하한 9·상한 16),
  `resolveFontSize(stored, width, cols)`, `load/saveFontOverride()` — 순수 함수.
- `src/web/TerminalScreen.tsx`: 스텝퍼 UI, 오버라이드 상태, `resize`/`resized`
  핸들러가 오버라이드를 존중.
- `src/web/styles.css`: `.term-mount { overflow: auto }` + 스텝퍼 스타일.
- `src-tauri/src/state.rs`: `AppEvents::terminal_resized`(기본 no-op) +
  `CompositeEvents` 전달. `peer/host.rs`: broadcast. `session/manager.rs`: 발화.

**2단계 — Secure 쿠키(J)**
- `peer/mod.rs`: `pair_complete`에 `HeaderMap` 추출자, `cookie_value(token, secure)`
  헬퍼 추출, `forwarded_https(&headers)` 판정.

**3단계 — serve(H·I)**
- `src-tauri/src/ipc/commands/tailscale.rs` 신설 — `tailscale_serve_status` /
  `_enable(https_port)` / `_disable(https_port)`. 상태 응답:
  `{ cli_found, tailscaled_running, cert_domain, https_port, conflict }`.
  JSON 파싱은 `parse_serve_status(json, peer_port)` 순수 함수로 분리(픽스처 테스트),
  CLI 실행은 `tokio::process` + 10초 타임아웃.
- `lib.rs` 커맨드 등록 · `src/renderer/ipc/peerApi.ts` 바인딩 ·
  `PeerShareSection.tsx`의 `webHostingEnabled` 블록에 HTTPS 소절
  (상태·포트 입력·켜기/끄기·https URL·폴백 명령·영속 안내).

**테스트로 고정할 것**
- cargo: `cookie_value`(Secure 유무) · `parse_serve_status`(우리 매핑/충돌/빈 상태,
  실기계 JSON 픽스처) · `manager.resize` → `terminal_resized` 발화와 `is_shared` 게이트.
- vitest: `termFont`(하한 9·상한 16, 오버라이드 우선, `맞춤` 복귀, localStorage 왕복).

**실기기 눈검증**
1. iPhone Safari — 스텝퍼 확대 후 **가로 패닝**(xterm 터치와의 충돌이 최대 관찰
   항목), 회전, 소프트 키보드, 읽기 전용.
2. 데스크톱 — 스텝퍼 동작, 200열 세션 가독.
3. 웹 attach 중 네이티브 창 리사이즈 → 웹 화면 즉시 추종(`Resized` 신설 검증).
4. serve 켜기 → `https://<name>:47443/web/` 접속·페어링·쿠키 `Secure`(devtools)·
   수 분간 WS 안정성.
5. serve 끄기, 앱 종료 후 잔존 매핑의 접속 실패 화면, **기존 443 매핑 무손상**.

## 12. Phase 2에서 하지 않는 것

웹발 PTY 리사이즈와 크기 소유권 중재(재방문 조건 §10.1) · 443 서브패스 서빙 ·
신원 헤더 활용 · funnel · 캐릭터별/기기별 폰트 기억과 Ctrl+휠 줌 ·
레이트리밋의 XFF 기반 분리.

## 13. Phase 2 위험

1. **xterm 터치 + 가로 스크롤 상성** — xterm은 touchmove를 세로 스크롤백에 쓴다.
   컨테이너 패닝이 뻑뻑할 수 있다. 폴백은 항상 성립(폰트를 줄이면 패닝 불필요).
2. **serve 경유 장수명 WS 끊김** — tailscale #18827(10~40초 주기 끊김 사례)가 미해결.
   기존 20초 ping + 백오프 재접속 + offset 복원이 방어하지만 체감되면 백오프 하한
   축소를 후속 검토. 토큰은 쿼리스트링으로 보내지 않는다(#18651 — serve가 WS
   업그레이드의 쿼리스트링을 벗기는 사례).
3. **47443의 타 기계 충돌** — 포트 입력칸이 출구.
4. **https/http 혼용 시 재페어링**(§10.3) — 수용.
5. **창 드래그 중 `Resized` 다발** — refit이 ResizeObserver마다 발화한다. 웹 처리
   비용은 싸서 수용하되, 트래픽이 문제 되면 manager 쪽 디바운스를 후속으로.
