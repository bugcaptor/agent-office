# 웹 호스팅 설계 — 브라우저로 접속해 작업하기 (kbm #7m)

작성: 2026-08-05 (Fable 설계 + Opus 구현). 상태: **Phase 1 구현 완료 · 실기기
눈검증 대기**. 선행: `docs/archive/peer-session-share-design.md`(#7k — 완료·폐기,
호스트 인프라는 여기로 계승).

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
2. **tailscale serve 경유 시 Origin 검사** — serve가 Host를 어떻게 전달하느냐에
   따라 `origin_allowed`가 오탐할 수 있다. https 문서화 시 실측 필요.
3. **전 캐릭터 attach의 트래픽** — 알림 수신을 위해 접속 시 전 캐릭터에 붙는다
   (활성 탭 외 출력은 폐기). tailnet에선 무해하나 낭비가 실측되면 meta-only
   구독을 Phase 2에 추가.
4. **웹발 상태 변화의 네이티브 반영** — `session.start`는 `session-state`
   이벤트로 네이티브에 닿지만, 네이티브에 세션 런타임 엔트리가 없으면
   `setSessionState`가 no-op일 수 있다(appStore의 prev 부재 가드). 눈검증 항목.
5. **rust-embed release 빌드** — `dist-web` 미존재 시 컴파일 실패.
   `npm run build`가 `web:build`를 체인하고 `.gitkeep`을 커밋해 막았지만 클린
   빌드 경로 확인 필요.
