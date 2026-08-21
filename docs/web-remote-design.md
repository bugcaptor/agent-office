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
| 바인드 | LAN/tailnet(전 인터페이스 바인드 + 원격 주소 허용목록) | **tailnet 우선** — 기본은 tailscale 인터페이스(100.64/10) IP에 직접 바인드, 미발견 시 `127.0.0.1` 폴백 + 설정 UI 안내(설정으로 전 인터페이스 허용) | 사용자 요구가 tailnet 한정. 노출 표면 축소 |
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

#### M1 구현 기록 (2026-08-21 — 코드 완성·실행 눈검증 대기)

체리픽 5건은 계획대로 들어갔다. 충돌은 두 파일뿐 —
`src-tauri/src/lib.rs`(`use tauri::{Emitter, Manager, RunEvent}` + window-state
import 병합)와 `src-tauri/src/control/mod.rs`(`create`가 그 사이 성격 프롬프트를
디스크 프로필에서 읽도록 바뀌어 있었다 → 아카이브의 `spawn_session` 추출을
받아들이되 `personality_prompt`는 새 동작을 유지). **컴파일 드리프트는 없었다**
— 16커밋이 거의 순수 추가였고, `AppEvents`에 새로 생긴
`session_started`도 기본 no-op이라 `WebRemoteEvents`가 그대로 컴파일된다.

계획 대비 달라진 결정:

1. **바인드를 실제로 좁혔다.** 아카이브 구현은 `0.0.0.0`에 열고
   **원격 주소 허용목록**(`remote_policy`)으로 정책을 강제했다. 이제
   `choose_bind_ip(bind, local_addrs)`(순수 함수, 단위 테스트)가 고른 주소에
   **직접 바인드**한다 — `tailnet`(기본)이면 tailscale 인터페이스 IP(IPv4 우선),
   못 찾으면 `127.0.0.1` 폴백. 허용목록 미들웨어는 방어층으로 남긴다.
   - 인터페이스 열거는 unix에서 `getifaddrs(3)`(`nix` — portable-pty가 이미
     끌고 오는 의존이라 트리가 늘지 않는다). 그 외 플랫폼은 열거 API가 없어
     UDP 소스 주소 프로브로 폴백한다(외부 명령 없음).
   - 미탐지는 설정 UI가 명시한다("Tailscale이 감지되지 않았습니다 — 127.0.0.1
     에만 열었습니다"). `webRemoteStatus.tailnetFound`가 그 근거.
   - **부수효과**: tailnet 바인드 중에는 `http://localhost:47800`이 닫힌다.
     M3의 serve 업스트림은 `http://127.0.0.1:47800`이 아니라
     **`http://100.x.y.z:47800`** 으로 잡아야 한다(§10.2 명령 수정 필요).
2. **개명 범위가 모듈명보다 넓다.** 배포본이 없으므로 마이그레이션 없이
   와이어까지 정리했다: 라우트 `/peer/v1/*`→`/webremote/v1/*`, 쿠키
   `ao_peer_token`→`ao_web_remote_token`, 헤더
   `X-Agent-Office-Peer-Token`→`…-Web-Remote-Token`, 토큰 파일
   `peer-tokens.json`→`web-remote-tokens.json`, 와이어 필드
   `peerId`→`clientId`·`viewerName`→`clientName`·`peerToken`→`clientToken`,
   설정 `peer_bind`/`peer_port`→`web_remote_bind`/`web_remote_port`,
   `webremote/web.rs`→`rpc.rs`.
3. **`peer_share_enabled` 설정을 없앴다.** 리스너 기동 조건이
   `web_remote_enabled` 하나로 단일화된다(예전엔 둘 중 하나면 떴다).
   영속 저장소 `peer-shared.json`·`peer-hosts.json`과 `peer:` 네임스페이스
   경로(`persist.ts` 필터, `session.rs`의 원격 라우팅, `save_state` 게이트)도
   함께 사라졌다.
4. **tap 수명이 바뀌었다.** 캐릭터별 공유 토글이 없어져 부팅 시 `apply_shares`가
   없다 — tap은 브라우저가 `attach`할 때 설치된다. 즉 **첫 attach 이전 출력은
   링버퍼에 남지 않는다**(첫 화면은 렌더러 스냅샷이 담당하므로 실사용 영향은
   없다).
5. 외부(논리) attach 세션과 tap의 상호작용은 방어가 이미 서 있었다 —
   `add_output_tap`은 agentId 수명의 `OutputSink`에 달리고(PTY 무관),
   `size_of`/`write_input`/`dispose`/`session_id_for`가 전부 부재를 흡수한다.
   별도 수정 없음.

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

#### M2 구현 기록 (2026-08-21 — 코드 완성·폰 눈검증 대기)

**와이어 최종 형태**(계획 대비 이름이 바뀌었다 — 아래 이탈 1):

```
클라이언트 → 호스트 (기존 rpc 프레임 위)
  chat.follow { agentId }              viewer   구독 시작(멱등)
  chat.send   { agentId, text }        operator 문장 통째 주입
  chat.keys   { agentId, keys:[name] } operator 명명 키 시퀀스

호스트 → 클라이언트 (새 프레임 하나)
  { type:"chat", agentId, items:[TranscriptItem], backfill, unavailable }

TranscriptItem = { role:"user"|"assistant", kind:"text"|"tool_use"|"tool_result",
                   text, toolName?, isError, sidechain }
```

`backfill:true`는 **교체**다(이어 붙이기가 아니다). 재접속·늦은 합류에서 서버가
최근 대화를 다시 보내는데, 이어 붙이면 같은 대화가 두 벌 쌓인다 — 교체 규칙이
클라이언트 dedup 없이 그 문제를 없앤다. `unavailable:true`는 전사 파일을 못
찾았다는 뜻이고 웹은 터미널 폴백을 안내한다.

**파서 분리와 로그 동일성.** `TranscriptSource::render`가
`parse(raw) -> Vec<TranscriptItem>` + `format_items(items) -> Vec<String>`로
갈라졌다. `render`는 트레이트 기본 구현(`format_items(&self.parse(raw))`)이라
로그와 채팅이 어긋날 수 없다. **기존 세션 로그 픽스처 테스트를 한 줄도 고치지
않았고**(claude/codex의 `render(raw)` 헬퍼가 이제 `format_items∘parse_entry`를
부른다) 전부 통과한다 — 그것이 "문자열 수준 동일" 계약의 근거다.

- 클램프는 **한 번만** 한다: 항목의 `text`는 자르지 않은 원문이고, 로그는
  `block()`에서, 와이어는 `TranscriptItem::clamped()`에서 자른다. 항목 단계에서
  미리 자르면 로그 포매터가 두 번 잘라 "… (이하 생략)"이 두 줄 붙는다.
- codex `sub_agent_activity`는 **도구 이름이 없는** `tool_use` 항목으로 표현한다
  (`toolName: null` + `sidechain: true` → 로그 `⤷ 서브에이전트 …` 한 줄).
  설계 스키마의 닫힌 집합을 유지하면서 예전 로그 줄을 그대로 재현하는 유일한
  표현이었다.
- 스키마에 `sidechain`을 **추가**했다(설계 명세엔 없었다). 없으면 서브에이전트
  표식 `⤷`를 포매터가 복원할 수 없어 로그가 달라진다.

**백필 규칙.** `TranscriptTailer::backfill(max_bytes, max_items)` —
파일 끝에서 최대 **256KB**를 거슬러 읽고, 잘렸으면 첫 줄을 버리고(중간에서
시작한 조각), 마지막 줄이 개행으로 안 끝났으면 그것도 버린다
(`complete_lines`, 순수 함수·단위 테스트). 파싱한 뒤 **마지막 100개**만 남긴다.
tail 스레드는 (1) 전사 파일을 처음 찾았을 때 (2) 붙어 있는 파일 집합이 바뀌었을
때(리줌으로 새 세션 파일) (3) 새 팔로워가 합류했을 때 백필을 보낸다.

계획 대비 달라진 결정:

1. **커맨드 이름이 `chat.*`이다** — §5 원안의 `transcript.follow`/
   `notifications.stream`이 아니다. 구독·주입·키가 한 화면(채팅 뷰)의 세 동작이라
   접두사를 맞췄다. `transcript.list`(과거 세션 목록)는 **미구현**으로 미뤘다 —
   M2 완료 기준(왕복)에 필요하지 않고 allowlist 표면만 넓힌다.
2. **알림/활동은 별도 스트림 커맨드가 없다.** `WebRemoteEvents`가
   `notification-new`/`notification-cleared`/`activity-event`/`session-state`를
   **캐릭터 필터 없이** broadcast 하고(붙은 브라우저가 없으면 직렬화도 안 한다 —
   `hub.has_clients()`), WS 연결은 **터미널 프레임만**(`output`/`restore`/
   `resized`) attach 집합으로, **채팅 프레임만** follow 집합으로 거른다.
   - 부수효과(좋은 쪽): 웹 클라이언트가 더 이상 접속 즉시 **모든 캐릭터에
     attach 하지 않는다**. 예전엔 알림을 받으려고 그래야 했고, 그 대가로
     캐릭터마다 tap + 1MB 링버퍼가 생겼다. 이제 링버퍼는 터미널 화면을 실제로
     연 캐릭터에만 생긴다.
3. **채팅 tail의 수명은 WS 연결**에 매인다. `chat.follow`가 연결 id를 하나
   더하고, 연결이 끊기면 `ChatRegistry::release`가 그 연결 몫을 전부 놓는다.
   마지막 팔로워가 빠지면 tail 스레드가 다음 슬라이스(100ms)에 끝난다.
   - **알려진 한계**: `chat.unfollow`가 없다(allowlist를 3개로 유지). 목록으로
     돌아가도 그 캐릭터의 tail은 브라우저를 닫을 때까지 돈다 — 비용은 세션
     로그 수집 스레드와 같은 2초 stat 하나다.
4. **주입은 봇 모드와 같은 함수를 쓴다.** `bot::runner::single_line` +
   `INJECT_SUBMIT_DELAY_MS`(150ms)를 `pub`으로 올려 `chat.send`가 그대로 부른다:
   개행을 공백으로 바꿔 한 줄로 만들고, 텍스트를 쓴 뒤 잠깐 쉬었다가 CR을 따로
   보낸다(TUI가 Enter를 삼키는 것을 막는 실측 방어).
5. **퀵 키는 이름만 받는다.** 브라우저가 임의 바이트를 stdin에 쏘는 통로를
   만들지 않으려고 `key_bytes`(순수 함수) 테이블 밖 이름은 `badArgs`이고,
   시퀀스 중 하나라도 모르면 **아무것도 쓰지 않고** 전부 거부한다.

검증: cargo 1044(lib 1012 + 통합 22 + 10) · vitest 1776 · `tsc --noEmit` 0 ·
`npm run web:build` 성공. 새 테스트는 파서 구조화·클램프 1회·`complete_lines`·
backfill·키 매핑·채팅 allowlist/가시성·attach 없는 알림 도달·chat tail 왕복
(백필→증분→unavailable→늦은 팔로워)·ChatScreen 렌더 7건.

늦은 팔로워 처리는 자체 리뷰에서 두 번 고쳤다: (1) 소스가 하나도 없을 때
스레드가 즉시 죽어 두 번째로 채팅을 연 화면이 영영 비었고, (2) `resend` 신호를
틱 분기의 `else`에 두어 틱 경계에 걸린 팔로워가 굶었다. 둘 다 회귀 테스트가
있다.

### M3. tailnet HTTPS (#7n 설계분 선별)

serve 감지·등록/해제 대행(`ipc/commands/tailscale.rs`, `parse_serve_status`
픽스처 테스트, 포트 47443) · `Secure` 쿠키. **주의**: M1이 tailnet IP에 직접
바인드하므로 serve 업스트림은 `http://127.0.0.1:47800`이 아니라
`http://<tailnet IP>:47800`이다(아카이브 §10.2의 명령을 그대로 쓰면 안 된다). 가독성 항목(폰트 스텝퍼·패닝·
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
