# 피어 세션 공유 설계 — 같은 네트워크의 agent-office끼리 (kbm #7k)

작성: 2026-08-04 (Fable 설계 + Opus 검증).
상태: **이력(archived)** — kbm 이슈 #7k는 2026-08-05에 완료·폐기 처리됐다.

**왜 남겨 두는가**: 이 문서가 결정한 호스트 인프라(출력 tap 팬아웃, 링버퍼와
절대 오프셋 회계, 스냅샷 왕복, 페어링·토큰 모델, tailnet 허용목록, `AppEvents`
미러)는 **폐기되지 않고 웹 호스팅(kbm #7m, `docs/web-hosting-design.md`)의
기반으로 그대로 계승됐다**. 그 코드가 왜 그렇게 생겼는지의 근거가 여기 있다.

**폐기된 것**: 앱↔앱 뷰어(§결정 3의 뷰어 절반, `peer/viewer.rs`)와 원격 캐릭터
오피스 연출. 브라우저 클라이언트가 상위호환이라 진화를 멈추고 동결했다(삭제하지
않은 이유: 오피스 씬에 남의 캐릭터를 합쳐 세우는 연출은 앱 뷰어로만 가능하다).

**뒤집힌 결정**: §결정 6의 "리사이즈는 호스트 단독"은 손님 의미론 전제라
웹 호스팅에는 쓸 수 없다 — 주인 의미론에서는 브라우저가 유일한 실사용
클라이언트인 시간대가 정상 시나리오다. 새 규칙은 `web-hosting-design.md` 참고.
근거 코드: `session/output.rs`, `session/manager.rs`, `state.rs`(AppEvents),
`control/{mod,protocol,client}.rs`, `sessiond/*`. 선행 문서:
`docs/session-broker-v2-design.md`(스냅샷·오프셋 회계의 원형),
`docs/cli-control-design.md`(2단계 옵트인 인증 모델의 원형).

## 1. 요구와 해석

호스트(A 머신)의 agent-office가 소유·실행 중인 에이전트 터미널 세션을, 같은
네트워크(우선 tailnet, 차선 LAN)의 뷰어(B 머신) agent-office에서 **보고 입력**한다.
프로세스·PTY·에이전트 CLI는 A에서만 돌고, B는 출력/입력만 중계한다 —
`tmux attach`의 의미론이지 ssh 세션이 아니다.

**ssh를 배제하는 이유(요구자 명시)**: ssh로 원격 셸을 열면 그 세션의 소유자는
"내 쪽 앱"이 되고, 에이전트 CLI의 훅(observer)·작업폴더·설정이 전부 원격 홈에
있으면서도 상태 관측은 이쪽에서 하게 되어 층이 어긋난다. 원하는 것은 **저쪽에서
이미 살아 있는 세션에 창을 하나 더 다는 것**이다.

## 2. 목표와 비목표

**목표**
- 뷰어의 오피스 씬에 원격 캐릭터가 등장하고, 터미널 출력·입력이 실시간 중계된다.
- 캐릭터 상태(작업중/대기/주의)·알림·서브에이전트 카운트가 동기화된다.
- 호스트/뷰어 모두 크로스플랫폼(전송이 HTTP/WS라 sessiond·UDS 의존 없음).
- 한 세션에 뷰어가 여럿 붙어도 동작한다(호스트 사용자 포함 N인 공동 관전).

**비목표(v1)**
- 인터넷 등 비신뢰망 노출(TLS·핀닝은 Phase 3).
- 뷰어에서 원격 세션 생성·kill·dispose·resize. **뷰어는 손님이다.**
- 원격 파일/git 브라우저, 세션 로그 열람, 사용량 집계.
- 호스트 앱이 꺼진 동안의 중계(§3 결정 1의 귀결).
- 뷰어가 받은 세션을 제3자에게 재중계(체이닝).

## 3. 핵심 결정

### 결정 1 — 중계 계층은 **앱↔앱**. 전용 peer 리스너 + WebSocket

뷰어가 필요로 하는 것은 PTY 바이트만이 아니다. observer가 파생시킨 캐릭터 상태,
알림, 프로필 메타, cwd/브랜치 라벨은 **앱에만 있고 sessiond에는 없다**. 앱 레벨
중계가 이 전부를 한 채널로 해결한다.

- **sessiond를 TCP로 확장하는 안 기각**: unix 전용·기본 off·**세션당 활성 data
  conn 1개**(새 attach가 기존 소켓을 shutdown)라 팬아웃하려면 데몬 링/conn 모델을
  수술해야 하고, 그러고도 앱 레벨 의미가 없어 반쪽 뷰가 된다. 데몬 프로토콜을
  네트워크에 노출하는 보안 표면 확대도 원치 않는다. 팬아웃을 앱에서 하면 브로커의
  "conn 1개" 제약을 아예 건드리지 않는다.
- **제3 프로세스 기각**: axum 서버가 이미 앱 안에 있다.
- **control 서버와의 관계**: 인증 패턴(2단계 옵트인·토큰 파일·상수시간 비교)은
  재사용하되 **리스너와 Router는 분리**한다. control은 `127.0.0.1` 전용이고
  `settings/set`·`create`처럼 네트워크에 내놓으면 안 되는 라우트를 갖는다. peer
  Router에는 그런 라우트가 **존재하지 않는다**(권한 축소를 코드 구조로 보장).
- **바인딩 정책**: 기본은 tailscale 인터페이스(100.64.0.0/10)에만 바인드. 설정으로
  `all`(0.0.0.0, 경고 동반) 또는 특정 IP. 포트 기본 47800(사용 중이면 +1 스캔,
  실제 포트는 설정 UI에 `호스트명 · IP:포트`로 표시 — 수동 입력이 디스커버리다).

### 결정 2 — 출력 팬아웃은 `OutputSink`에 tap 리스트를 다는 최소 변경

`FlushSink::emit`이 이미 유일한 방출 지점이다. `OutputSink`에
`taps: Mutex<Vec<Arc<dyn OutputTap>>>`를 더해 primary 채널(렌더러)과 tap 양쪽으로
복제한다. tap은 Vec이므로 **다중 뷰어가 자연히 성립**한다.

- backlog(채널 미부착 시 적재, cap 256)의 의미론은 **렌더러 primary 전용으로 유지**
  한다 — 뷰어의 화면 복원은 아래 링버퍼/스냅샷이 담당하며 목적이 다르다.
- output_pump가 아니라 sink에 다는 이유: pump는 세션 수명, sink는 agentId 수명이라
  세션 재생성을 가로질러 tap이 유지된다.
- **뷰어 첫 화면 복원**(브로커 v2 설계를 앱 레벨로 미러링):
  1. 공유 토글이 켜진 세션은 뷰어 유무와 무관하게 호스트가 **raw tail 링버퍼
     (512KiB) + 절대 오프셋**을 유지한다. `OutputChunk.bytes`가 이미 raw 스트림
     계수(§#49)라 회계를 그대로 쓴다.
  2. 뷰어 attach 시 호스트가 렌더러에 스냅샷을 요청 → 렌더러가
     `TerminalRegistry.flushAndSerializeAll()`(브로커 30초 업로더가 쓰는 그 경로)의
     단건 변형으로 직렬화 → `restore { snapshot(deflate+b64), baseOffset }` 송신 →
     이후 baseOffset 이후 delta + 라이브.
  3. 스냅샷을 3초 안에 못 받으면 **링버퍼 전체 replay로 폴백**한다.
  4. 뷰어 xterm에서 스냅샷은 §#49의 `Restore`와 동일하게 **계수하지 않는 화면
     이미지**로 취급한다(오프셋 회계 오염 방지 — 이미 값을 치른 함정).
- **재접속**: `attach { lastOffset }`이 링버퍼 범위 안이면 delta만, 벗어났으면
  스냅샷 경로. 브로커의 `stream_offset` 협상과 동형이다.

### 결정 3 — 뷰어 표현은 `RemotePtyFactory`가 **아니라** 얇은 병렬 레지스트리

`PtyFactory`는 "스폰" 추상화인데 원격 세션은 스폰이 아니라 **이미 있는 세션에
붙기**다. create 경로에 얹으면 우회해야 할 호스트 전용 부작용이 줄줄이 세션
플래그가 된다: 셸 해석(`shells.rs`), observer 계획과 **래퍼 스크립트 로컬 파일
생성**, env 주입, 세션 로그 tee, claude resume 연동, cleanup_paths, waiter/exit
의미론, 봇 모드 `last_activity`. `broker_owned` 하나로도 manager.rs 주석이 이만큼
불어난 마당에 8개 분기는 매니저를 원격 관심사로 오염시킨다.

대신 뷰어 백엔드에 `peer/viewer.rs`의 `RemoteSessionRegistry`를 둔다. 키는
`peer:<peerId>:<agentId>` 네임스페이스(문자열 키라 렌더러 스토어·오피스 배치가
그대로 소화한다).

- **출력**: 원격 에이전트마다 **기존 `OutputSink`를 그대로** 만들어 이 레지스트리
  맵에 보관. `attach_output` 커맨드가 프리픽스를 보고 라우팅하면 렌더러의
  sessionBridge·터미널 컴포넌트는 **무수정 재사용**된다.
- **입력**: `write_input` 진입점에서 프리픽스 라우팅 → WS `input` 송신.
- **상태/알림**: WS로 받은 이벤트를 뷰어의 Tauri emit으로 재방출 → 캐릭터 상태·알림
  파이프라인 재사용.
- **라우팅 위치**: 백엔드 커맨드 진입점(`ipc/commands/session.rs`)에서 한다. 렌더러
  브리지에서 가르면 우회 가능한 게이트가 된다.
- **영속화 금지 규칙**: `peer:` 프리픽스 에이전트는 프로필 스토어·설정 등 어떤
  로컬 영속 계층에도 쓰지 않는다(인메모리 전용). 저장 계층 진입점에서 프리픽스를
  거부해 강제한다.

### 결정 4 — 상태 동기화는 `AppEvents` 합성 팬아웃, 프로필은 호스트 소유

`state.rs`의 `AppEvents`(session_started / session_state / notification_new /
notification_cleared / activity_event)가 이미 **모든 앱 이벤트의 단일 관문**이다.
`CompositeEvents(TauriEvents, PeerEvents)`로 공유 세션의 이벤트만 WS에 미러링한다.
신규 훅은 사실상 없다.

- **미는 것**: session-state, activity(작업중/대기/주의·서브 카운트 — 같은 스트림이라
  공짜), notification-new/cleared, 프로필 메타(name/role/seed/cwd/브랜치), 터미널
  크기 변경.
- **포기(v1)**: 사용량 집계, 세션 로그, claude resume 상태, 커스텀 pixellab 스프라이트
  이미지(v1은 `seed` 기반 기본 스프라이트로 렌더, Phase 2에서 해시 캐시 전송).
- **프로필 소유권**: 호스트 단독. 뷰어는 읽기 캐시이고 편집 UI는 비활성.
- **이름 충돌**: 뷰어 씬에서 원격 캐릭터는 `이름@호스트명`으로 표기한다.

### 결정 5 — 보안: 2단계 옵트인 + 6자리 페어링 코드 → 장기 peer 토큰

control의 검증된 모델을 확장한다.

1. 설정 `peerShareEnabled` OFF 기본 → ON이면 리스너만 뜨고 **모든 요청 401**.
2. 뷰어가 `pair/start` → **호스트 앱 UI에 승인 다이얼로그 + 6자리 코드**(TTL 120초)
   → 뷰어가 코드 입력해 `pair/complete` → 128비트 peer 토큰 발급, 양쪽 0600 저장.
   코드 3회 오입력 시 페어링 폐기.
3. 이후 WS는 매 연결 시 토큰 검증(상수시간).
4. **공유 단위는 에이전트별 opt-in**(캐릭터 컨텍스트 메뉴 "이 캐릭터 공유"). 전체
   공유 스위치는 두지 않는다 — 실수로 전부 노출되는 사고를 구조로 막는다.
5. **뷰어 권한은 `readOnly | input` 2단계**(승인 시 선택, 이후 변경 가능). 서버에서
   검사한다(UI 비활성은 보조일 뿐). kill/dispose/생성/resize는 권한 체계에 아예 없다.

**전송 기밀성(v1)은 tailnet(WireGuard)에 위임**한다. 기본 바인딩이 tailscale
인터페이스이므로 기본 구성에서 평문이 LAN에 흐르지 않는다. `all` 바인딩은 명시
옵트인 + "평문 전송" 경고. TLS 자체서명+지문 핀닝은 Phase 3 — 지금 넣으면 인증서
수명·핀 갱신 UX가 페어링 UX를 잡아먹는다.

**디스커버리는 수동 `host:port` 입력 + 최근 피어 기억**. mDNS는 tailnet에서 동작하지
않고 사용자의 주 환경이 tailnet이므로 v1 기각, Phase 2에서 LAN 보조로 검토.

### 결정 6 — 입력은 자유 인터리브, 리사이즈는 호스트 단독

- 입력: 호스트·뷰어 입력을 PTY에 그대로 인터리브한다(PTY가 자연 직렬화). 잠금·턴
  테이킹 없음 — 요구가 정확히 tmux 공유 모델이다.
- 리사이즈: **호스트가 유일한 소유자**. 뷰어는 호스트가 push한 (cols, rows)로 xterm을
  고정하고 스케일/스크롤로 수용한다. tmux식 "최소 공통 크기"는 뷰어가 호스트
  사용자의 화면을 좁히게 되어 손님 의미론을 깨고, 크기 상태 소유자가 둘이 되면
  재접속 복원도 복잡해진다.

### 결정 7 — 수명·재접속

- WS keepalive 20초 ping / 45초 무응답 시 양쪽 폐기.
- 뷰어 끊김/재시작: 지수 백오프(1s→30s) 자동 재접속 + `attach { lastOffset }` delta 복원.
- 호스트가 뷰어 conn을 정리해도 **링버퍼는 공유 토글이 켜진 동안 유지**한다(재접속 delta용).
- 호스트 앱 종료: 뷰어 쪽 해당 캐릭터는 "끊김" 연출. 세션이 sessiond에 남아 있어도
  호스트 앱이 다시 떠서 입양하기 전에는 보이지 않는다(결정 1의 의도된 대가).
- 호스트에서 피어 삭제 = 토큰 폐기 + WS 즉시 종료.

## 4. 아키텍처

```
   호스트 (머신 A)                                    뷰어 (머신 B)
┌──────────────────────────────┐              ┌──────────────────────────────┐
│ 렌더러(React/xterm)           │              │ 렌더러(React/xterm)           │
│  · 로컬 캐릭터/터미널          │              │  · 로컬 + 원격(홀로그램) 캐릭터 │
│  · on-demand 스냅샷 직렬화 ─┐ │              │  · attach_output("peer:…") ▲  │
├────────────────────────────┼─┤              ├────────────────────────────┼─┤
│ SessionManager             │ │              │ RemoteSessionRegistry      │ │
│  · OutputSink ─emit─┬──────┼─┼──┐           │  · OutputSink(그대로 재사용)─┘ │
│  · write_input ◄────┼───┐  │ │  │           │  · peer: 프리픽스 라우팅      │
│ NotificationHub     │   │  ▼ │  │           │  · 재접속 백오프 / lastOffset │
│ AppEvents ─Composite┼───┼─► peer/host       │              ▲               │
│   (Tauri + Peer)    │   │  · tap + 링버퍼512K│              │               │
│ sessiond(선택·무관)  │   │  · 스냅샷 캐시     │  peer/viewer (WS 클라이언트)  │
└─────────────────────┼───┼──┬───────────────┘              │               │
                      │   │  │ axum listener                │               │
                      │   │  │ tailscale IP:47800           │               │
                      │   └──┴────────── WebSocket ─────────┘               │
                      └──── input ◄──────────────────────────────────────────┘
```

PTY·프로세스·observer 훅·세션 로그·봇 모드는 전부 호스트에 남는다. 건너가는 것은
(1) 출력 바이트 (2) 화면 스냅샷 (3) 앱 이벤트 (4) 입력, 넷뿐이다.

## 5. 와이어 프로토콜 초안

전송 `http://<host>:47800`, JSON camelCase, `protoVersion: 1`, additive-only 확장
(브로커 v2의 버전 협상 관례 준용). WS 업그레이드 헤더 `X-Agent-Office-Peer-Token`.

**HTTP (페어링)**

| 라우트 | 요청 | 응답 |
|---|---|---|
| `POST /peer/v1/pair/start` | `viewerName, appVersion, protoVersion` | `{ pairingId }` (호스트 UI에 코드 표시) |
| `POST /peer/v1/pair/complete` | `pairingId, code` | `{ peerToken, hostName, peerId, permission }` |
| `GET /peer/v1/ws` | (업그레이드 + 토큰) | WS |

**WS 호스트 → 뷰어**

| 메시지 | 필드 |
|---|---|
| `hello` | `hostName, appVersion, protoVersion, permission` |
| `agents` | `[{ agentId, name, role, seed, cwd, gitBranch, state, sessionId?, cols, rows }]` (초기 전량 + 공유 목록/프로필 변경 시 전량 재송) |
| `restore` | `agentId, snapshot?(deflate+b64), baseOffset` |
| `output` | `agentId, sessionId, seq, offset, data(utf8), bytes` |
| `activity` / `sessionState` | 기존 이벤트 직렬화 그대로 |
| `notification` / `notificationCleared` | 기존 이벤트 그대로 |
| `resized` | `agentId, cols, rows` |
| `sessionEnded` | `agentId, exitCode?, signal?` |
| `pong` | |

**WS 뷰어 → 호스트**

| 메시지 | 필드 | 비고 |
|---|---|---|
| `attach` | `agentId, lastOffset?` | 세션별 구독 |
| `detach` | `agentId` | |
| `input` | `agentId, data(utf8)` | `permission == input`일 때만, 아니면 에러 프레임 |
| `ping` | | |

## 6. 배선 맵

**신규**
- `src-tauri/src/peer/mod.rs` — 리스너 기동·바인딩 정책(tailscale 인터페이스 감지)·Router
- `src-tauri/src/peer/protocol.rs` — 메시지 타입(serde)
- `src-tauri/src/peer/pairing.rs` — 코드 생성·TTL·토큰 저장(`<app_data>/peer-tokens.json`, 0600)
- `src-tauri/src/peer/host.rs` — WS 핸들러, tap 구독, 링버퍼, 스냅샷 캐시, 권한 검사, `PeerEvents`
- `src-tauri/src/peer/viewer.rs` — WS 클라이언트, `RemoteSessionRegistry`

**수정(백엔드)**
- `session/output.rs` — `OutputSink`에 `add_tap`/`remove_tap`, emit 팬아웃
- `session/manager.rs` — `subscribe_output_tap(agent_id)` 공개, resize 시 peer push 콜백
- `state.rs` — `CompositeEvents`(기존 `TauriEvents` 불변)
- `ipc/commands/session.rs` — `attach_output`/`write_input`의 `peer:` 프리픽스 라우팅,
  `resize_session`/`dispose_session`은 원격이면 거부
- `lib.rs` — peer 부트스트랩, 신규 커맨드(`peer_status/approve/revoke/connect/list`,
  on-demand 스냅샷 응답), 종료 훅 정리
- `types.rs` — 설정 `peerShareEnabled`/`peerBind`/`peerPort`, 에이전트별 `shared`, 원격 DTO

**수정(프런트)**
- `store/appStore.ts` — 피어·원격 에이전트·연결 상태
- `ipc/sessionBridge.ts` — peer 이벤트 구독, 원격 세션 입력/리사이즈 게이트
- `terminal/` — 원격 배지·읽기전용 표시·리사이즈 잠금(호스트 크기 고정 + 스케일)
- `office/`·`sprite/` — 홀로그램 톤, 끊김 연출
- `settings/` — "세션 공유" 탭(호스트: 토글·바인딩·승인 코드·페어 목록 / 뷰어: 피어 추가·연결 상태)
- 캐릭터 컨텍스트 메뉴에 "이 캐릭터 공유" 항목(공용 `ContextMenu` 재사용)

## 7. 기능 매트릭스 (뷰어 쪽에서)

| 기능 | 배정 | 근거 |
|---|---|---|
| 터미널 출력·입력 | **호스트 중계** | 핵심 |
| 캐릭터 상태·서브에이전트 | **호스트 push** | AppEvents 미러 |
| 알림 표시 | **호스트 push** | |
| 알림 클리어 | 뷰어 로컬만 | 원격이 호스트 사용자의 알림을 지우면 놓침 |
| TTS | 뷰어 로컬(옵션, 기본 off) | 알림이 로컬에 도착하니 공짜, 소음 방지 |
| 파일/git 브라우저 | 비활성 (Phase 3 RPC 후보) | 호스트 fs 필요 |
| 세션 로그·분석 | 비활성 | 호스트에는 계속 기록됨 |
| 봇 모드 | 비활성 | 소유권 침해 |
| 요약 | 비활성 | API 키·일관성 |
| 사용량 | 비활성 | 호스트 로컬 파일 기반 |
| claude resume | 비활성 | 호스트 전용 |
| 새 세션 생성 | 불가 (Phase 3 후보 — control `create` 프록시라 저비용) | 손님 의미론 |
| kill/dispose/resize | 불가 | 결정 6 |
| 프로필 편집 | 비활성(읽기 캐시) | 호스트 소유 |

## 8. UX — 오피스 메타포

1. **파견 직원**: 원격 캐릭터가 게스트 데스크에 일반 캐릭터처럼 앉는다. → 로컬/원격
   구분이 약해 "어느 머신 세션인지" 혼동 위험. 기각.
2. **홀로그램 (추천)**: 청록 홀로그램 톤 + 은은한 스캔라인. 기존 배치·스프라이트
   파이프라인에 셰이드 패스만 얹으면 되어 가장 싸고, "실체는 저쪽에 있다"는 소유권
   의미론이 연출로 즉시 전달된다. 연결 상태를 연출로 흡수한다 —
   정상=은은한 파동, 지연(RTT>1s 지속)=주기적 깜빡임, 끊김=프레임 정지+노이즈+
   「연결 끊김」 말풍선, 재접속 중=반투명 점멸.
3. **지사 포털**: 씬 가장자리에 "○○지사" 문, 클릭 시 원격 미니 오피스. → 별도 씬
   렌더라 공수가 크다. Phase 3 확장 연출 후보.

## 9. 단계 계획

**Phase 1 — 최소 실사용** (감각: 브로커 v2의 6~7할. 백엔드 신규 ~2천 줄 + 프런트 수백 줄)
1. `peer/protocol.rs` + 페어링(HTTP)·토큰 저장·설정 필드
2. `OutputSink` tap 팬아웃 + 공유 세션 링버퍼/오프셋
3. `peer/host.rs` WS: attach/restore(스냅샷 + 링 폴백)/output/input/`CompositeEvents` 미러
4. `peer/viewer.rs` + `peer:` 프리픽스 라우팅 + 렌더러 원격 상태·홀로그램·리사이즈 잠금
5. 설정 탭·승인 다이얼로그·끊김 연출·재접속 백오프

의도적으로 버리는 것: TLS, mDNS, 커스텀 스프라이트 전송, 파일/git 프록시, 원격 세션
생성·kill, 사용량, 알림 원격 클리어, 다중 뷰어 튜닝(동작은 하되 상한/디바운스 미조정).

**Phase 2 — 완성도**: 커스텀 스프라이트 전송(해시 캐시), RTT 측정·지연 연출, LAN mDNS
보조, 뷰어 TTS 정식 옵션, 링버퍼/스냅샷 주기 튜닝, Windows 양방향 실기기 검증.

**Phase 3 — 신뢰 확장**: rustls 자체서명 + 페어링 시 지문 핀닝(비 tailnet LAN 정식
지원), 원격 생성/kill 권한 레벨, 파일/git 브라우저 RPC 프록시, 지사 포털 연출.

## 11. 구현 현황과 #7m(웹 호스팅) 통합

### 이미 들어와 있는 것 (그대로 재사용 대상)

| 산출물 | 위치 | #7m에서의 지위 |
|---|---|---|
| 바인딩 정책(tailnet 100.64.0.0/10 판정, 원격 주소 허용목록) | `peer/mod.rs` | 그대로 |
| 페어링(6자리 코드·사람 승인·3회 오입력 폐기) + 토큰 스토어(0600·상수시간) | `peer/pairing.rs` | 그대로 (코드 표시 주체만 반대로) |
| WS 호스트 핸들러(attach/restore/output/input/keepalive/Lagged 복원) | `peer/mod.rs` | 그대로 — 웹 RPC는 이 리스너에 라우트를 얹는다 |
| 출력 tap 팬아웃 + 링버퍼 + 오프셋 회계 + 스냅샷 왕복 | `session/output.rs`, `peer/host.rs` | 그대로(공유 자산) |
| `AppEvents` 미러(`CompositeEvents` + `PeerEvents`) | `state.rs`, `peer/host.rs` | 그대로 |
| 캐릭터별 공유 토글 + 뷰어용 메타(`build_agents`) | `peer/mod.rs` | 그대로 |
| 앱↔앱 뷰어(`RemoteSessionRegistry`, 재접속·오프셋 델타) | `peer/viewer.rs` | **우선순위 하향** — 브라우저 클라이언트가 상위호환 |
| 렌더러: 원격 캐릭터 스토어 주입·스냅샷 응답·설정 UI | `store/appStore.ts`, `ipc/peerBridge.ts`, `settings/PeerShareSection.tsx` | 설정 UI·스냅샷 응답은 유지, 원격 캐릭터 오피스 연출은 보류 |

### #7m 때문에 **선반영한 변경** (2026-08-05)

- **인증 3경로화**: `X-Agent-Office-Peer-Token` 헤더 외에 **쿠키
  (`ao_peer_token`, HttpOnly·SameSite=Strict)** 와 `Sec-WebSocket-Protocol`
  (`agent-office.token.<token>`)로도 인증한다. 브라우저의 WebSocket API는
  커스텀 헤더를 붙일 수 없어 헤더 전용 인증이던 원안으로는 **브라우저가 아예
  붙지 못했다**(#7m §D). `pair/complete`가 쿠키를 함께 발급한다.
- **Origin 검사**: `Origin`이 없으면 앱↔앱으로 보고 통과, 있으면 `Host`와 같은
  오리진만 허용한다 — 남의 페이지가 사용자의 쿠키를 업고 WS를 여는 것을 막는다.

### 아직 #7m의 몫 (여기서 하지 않은 것)

1. 커맨드 **allowlist + 권한 티어**(viewer/operator/admin) 테이블 — 웹은 라우트를
   뺄 수 없으므로 구조 대신 명시 테이블로 권한을 보장해야 한다.
2. `AgentOfficeApi`의 **두 번째 구현(webApi)** + WS 상관 RPC(`{id, cmd, args}`).
3. 커맨드 본문의 **service 계층 이관**(Tauri·control·peer·web 4중 진입점 정리).
4. 모바일 레이아웃 + 소프트 키보드 보조바, capability 게이팅.
5. **터미널 크기 소유권**: 이 문서의 §결정 6("호스트 단독")은 손님 의미론 전제라
   웹 호스팅에는 그대로 쓸 수 없다 — #7m §G의 소유권 위임으로 대체될 예정.

## 10. 미해결 질문

1. on-demand 스냅샷 요청 시 호스트 웹뷰가 백그라운드 절전이면 응답이 늦는다 —
   링버퍼 replay 폴백만으로 충분한지 실증 필요(대체 화면 TUI에서 특히).
2. 다중 뷰어 동시 attach 시 스냅샷 요청 디바운스 필요 여부.
3. 호스트에서 캐릭터 삭제/agentId 재사용 시 뷰어 캐시 무효화 규칙(현안: `agents`
   전량 push 덮어쓰기).
4. 포트 47800 고정의 충돌 실태 — 릴리스 전 재검토.
5. 링버퍼 512KiB가 적정한지(에이전트 TUI는 출력이 굵다). 스냅샷이 주 경로라면 더
   작아도 되고, 폴백이 주 경로가 되면 더 커야 한다 — 1번과 함께 결정.
