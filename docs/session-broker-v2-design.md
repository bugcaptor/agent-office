# 세션 브로커 v2 계획 — 상시 브로커(스폰부터 데몬이 PTY 소유)

작성: 2026-07-17 (Fable). 상태: **Phase 1 구현 중 (feature flag opt-in, unix 전용)**.
전제: v1(feat/session-handoff, PR #6) = 종료 시점 핸드오프. v2는 그 역전.
활성화: `AGENT_OFFICE_SESSION_BROKER=v2` + unix일 때만 `BrokerPtyFactory` 주입.
기본 off라 v1 경로(PortablePtyFactory + 종료 시 fd 핸드오프)는 그대로 보존된다.

## v2가 v1보다 나은 것 / 잃는 것

| | v1 핸드오프 | v2 상시 브로커 |
|---|---|---|
| 정상 종료 시 존속 | O | O |
| **앱 크래시 시 존속** | X | **O** |
| Windows 지원 | X (ConPTY 이전 불가) | **O** (브로커가 처음부터 소유) |
| 이어받은 세션 exit code | X (waitpid 불가, EOF만) | **O** (브로커가 부모) |
| 이중 리더/fd 전달 곡예 | 필요 (poll 인터럽트) | 불필요 (소유권 이동 없음) |
| 평상시 핫패스 | 프로세스 내 직접 I/O | **UDS 1홉 경유** (µs 단위, 체감 없음) |
| 단일 실패점 | 없음 (평상시 데몬 없음) | **브로커 크래시 = 전 세션 사망** |
| 버전 스큐 | 데몬 수명이 짧아 사실상 없음 | **구버전 브로커 ↔ 신버전 앱** 상시 가능 |

핵심 트레이드는 "크래시 생존 + Windows"를 얻고 "브로커 단일 실패점 + 프로토콜
호환성 관리"를 떠안는 것.

## 아키텍처

```
[앱] SessionManager (기존 그대로)
  └ BrokerPtyFactory : PtyFactory   ← 유일한 교체 지점 (트레잇 경계 활용)
       │ control conn (UDS/NamedPipe, u32LE+JSON — v1 프로토콜 확장)
       │ 세션당 data conn (raw 양방향 바이트 스트림, 프레이밍 없음)
[sessiond 브로커] ← 스폰부터 PTY/ConPTY 소유, 세션별 링버퍼 + waitpid
```

- **BrokerPtyFactory**: `spawn()`이 브로커에 Spawn RPC → 브로커가 openpty+fork →
  앱은 세션 전용 data conn을 열어 reader/writer로 사용. `SpawnedPty` 형태
  (reader/writer/control/waiter)가 보존되므로 **SessionManager는 사실상 무변경**
  — v1에서 install_session을 추출해 둔 것과 같은 원리로, 교체는 팩토리 한 겹.
- **데이터 전송**: 세션당 별도 raw 연결 (멀티플렉싱 안 함 — 프레이밍/역압 관리가
  사라지고 reader/writer가 소켓 그 자체가 됨. 세션 수십 개 수준에서 fd 수는 문제 아님).
- **control 채널**: v1 프로토콜(Hello/List/Kill/…)에 Spawn/Attach/Resize/Wait 추가.
  resize는 control 경유(브로커가 TIOCSWINSZ/ConPTY resize 수행).
- **waiter**: 브로커가 부모라 waitpid 가능 → Wait RPC 또는 data conn EOF 후
  ExitInfo 조회. v1의 "exit code 소실" 제약 해소.
- **브로커 수명**: 첫 세션 스폰 때 lazy 기동(`--sessiond` 재사용). 앱 종료 모달
  의미 변경 — "유지하고 종료" = 그냥 disconnect, "모두 종료" = KillAll RPC 후
  종료. 세션 0이 되면 브로커 자체 종료(v1과 동일 규칙).
- **재접속**: 부팅 시 List → 각 세션에 Attach(스냅샷+링버퍼 리플레이 → data conn
  재개). v1의 adopt 흐름과 UI(redraw nudge 포함)를 그대로 재사용.
- **화면 복원**: v1에서 만든 xterm 직렬화 스냅샷을 그대로 사용 — 단, 종료 시점이
  아니라 **주기 또는 disconnect 시점에 브로커로 업로드**(크래시 생존을 위해선
  마지막 스냅샷이 브로커에 있어야 함. 주기 30s + quit 시 1회가 기본안).

## 프로토콜 v2 확정 (Phase 1 와이어 계약)

`PROTO_VERSION`을 **2**로 올렸다. 규칙은 **additive-only**: 기존 메시지
(Hello/HelloOk/Handoff/HandoffOk/List/ListOk/Adopt/AdoptOk/Kill/KillOk/Error)는
의미 불변이고, 신규 필드는 전부 `#[serde(default)]`. 프레이밍은 v1과 동일
(`u32 LE 길이 + JSON`). 구현: `src-tauri/src/sessiond/{protocol,client,daemon}.rs`.

### 신규 메시지 (앱 → 데몬, 별도 표기 없으면 control 연결에서 프레임 RPC)

- `Spawn { agent_id, session_id, shell, args, env: [(k,v)], rows, cols, cwd, cleanup_paths }`
  → `SpawnOk { pid }`. 데몬이 portable-pty로 openpty+spawn하고 세션 테이블에
  등록한다. 링버퍼는 **스폰 시점부터** 수집한다. fd는 동반하지 않는다(v1 Handoff와
  정반대로 소유권이 처음부터 데몬에 있다). `env`는 앱(SessionManager)이 이미
  관찰자 훅/설정 파일 경로까지 계산해 넘기고 데몬은 그대로 주입만 한다.
- `DataAttach { agent_id }` → 프레임 `DataAttachOk { stream_offset }` **직후 해당
  연결이 raw 양방향 바이트 스트림으로 전환**된다(이후 프레이밍 없음). 데몬은
  DataAttachOk 직후 백로그를 같은 스트림에 먼저 쓰고 이어서 라이브 출력을 흘린다
  (이음새 없는 리플레이 — 백로그 스냅샷과 conn 설치를 하나의 락 아래에서 원자화해
  유실/중복 없음). `stream_offset`은 이 연결이 흘리는 백로그 첫 바이트의 절대
  스트림 오프셋(= 링의 `total - backlog.len()`)으로, 앱이 data reader의 누적 수신
  카운터를 여기서 시작하는 데 쓴다(§P1). **백로그 범위**: 스냅샷이 업로드된 적
  있으면 링버퍼 전체가 아니라 그 스냅샷의 오프셋(`snapshot_offset`) *이후* 바이트만
  흘린다 — 앱이 그 스냅샷을 화면으로 별도 복원하므로, "스냅샷 + 이후 출력"이 되어
  중복 없이 전체 스크롤백이 재구성된다. 스냅샷이 한 번도 없으면 링 전체를 흘린다.
  앱→데몬 방향 raw 바이트는 PTY master에 기록된다. **세션당 활성 data conn은 1개**
  — 새 DataAttach가 오면 기존 소켓을 `shutdown`해 교체한다. 앱 쪽 EOF/에러 =
  detach(자식은 죽이지 않음). **종료 직후 설치 레이스(§P2-b)**: 자식을 reap한
  waiter는 같은 io 락 아래에서 `closed=true`를 세우고 활성 conn을 정리한다. 그
  *뒤* 도착한 DataAttach는 (역시 같은 락 아래에서 `closed`를 보고) 새 conn을 설치
  하지 않고 백로그만 쓴 뒤 자기 fd를 `shutdown(Write)`해 앱에 백로그+EOF를 주고
  끝낸다(입력 펌프 생략 — 자식이 이미 죽어 master에 쓸 게 없다). 이 직렬화가
  없으면 그 conn을 아무도 닫지 않아 앱 reader가 영원히 블록된다.
- `Attach { agent_id }` → `AttachOk { rows, cols, pid, snapshot_b64,
  snapshot_compressed, exit: Option }`. 재접속용 메타데이터+최신 업로드 스냅샷
  회수(백로그는 data conn 담당이라 여기 buffer는 없다). 데몬은 스냅샷 바이트를
  불투명하게 보관·반환하고 `snapshot_compressed`로 압축 여부만 전달한다(§P2-c) —
  앱이 입양 시 해제한다. `exit`이 Some이면 이미 종료된 세션.
- `Resize { agent_id, rows, cols }` → `ResizeOk`. 데몬이 PTY master를 resize.
- `Wait { agent_id }` → 자식 종료까지 블로킹 후 `WaitOk { exit_code, signal }`.
  연결별 독립 처리이므로 waiter는 **전용 control 연결**을 하나 열어 쓴다. 데몬은
  자식을 reap(waitpid)해 ExitInfo를 기록하고, 종료 시 reader가 잔여 출력을 전부
  흘린 뒤 data conn을 닫는다. 데몬이 자식의 부모라 v1 EofWaiter의 "exit code
  소실" 제약이 사라진다.
- `KillAll` → `KillAllOk { killed }`. 브로커 소유 자식 전부 SIGKILL + 테이블 비움
  (원자적 일괄 종료용 프로토콜 surface). 단 앱의 "모두 종료"/dispose_all은
  실제로는 **세션별 `Kill { agent_id }`**로 처리한다 — 각 세션의 control이 알아서
  죽이므로(브로커 세션=Kill RPC, 데몬 접속 실패로 폴백 스폰된 in-process 세션=
  직접 kill) KillAll 특수 분기 없이도 폴백 세션 누수가 없다. `Kill`은 데몬이
  SIGKILL+테이블 제거+cleanup까지 하고, 그 자식을 reap한 waiter가 exit를 기록해
  앱의 Wait가 반환→상태가 Disposed로 전이한다.
- `UpdateSnapshot { agent_id, snapshot_b64, offset: Option<u64>, compressed }` →
  `UpdateSnapshotOk`. 주기 스냅샷 업로드(기본 30초, 렌더러) — 데몬은 세션당 최신
  것만 보관해 앱 크래시 후 화면 복원에 대비한다. `offset`(§P1)이 Some이면 데몬은
  수신 시점 `ring.total()` 대신 그 값을 `snapshot_offset`으로 기록한다(링 상한으로
  클램프, 하한은 리플레이가 처리). `compressed`(§P2-c)는 `snapshot_b64`가 deflate
  압축인지 — 데몬은 그대로 보관하고 AttachOk에 되돌려준다.
- `ListOk`의 `SessionInfo`에 `broker: bool`(serde default) 추가 — v1 핸드오프
  세션(false)과 v2 브로커 세션(true)을 additive로 구분한다.

### §P1 스냅샷 오프셋 정합 (유실 창 제거, 부분 수용 — 완전 동기화는 Phase 2)

렌더러가 화면을 직렬화한 시점과 데몬이 UpdateSnapshot을 수신한 시점 사이에, 데몬은
링에 넣었지만 앱은 아직 안 읽었거나 렌더러가 아직 파싱 안 한 바이트가 있을 수 있다.
데몬 수신 시점 `ring.total()`을 스냅샷 오프셋으로 쓰면 그 구간이 "스냅샷에도 없고
offset 이전이라 리플레이도 안 되는" 영구 유실 창이 된다. 이를 없애기 위해:

- data reader를 `CountingReader`로 감싸 **앱이 실제 수신한 절대 오프셋**을 센다
  (`DataAttachOk.stream_offset`으로 초기화). 스냅샷 업로드(`upload_snapshots`/
  `handoff_all_broker`)가 이 카운터 값을 `offset`으로 동봉한다.
- 렌더러는 직렬화 *전에* xterm write 큐를 flush한다(`TerminalRegistry.
  flushAndSerializeAll()` → `term.write("", cb)` 콜백 대기). 30초 업로더와
  `ConfirmQuitDialog`("유지하고 종료")가 이걸 쓴다 — 이미 도착한 바이트까지
  스냅샷에 반영해 offset과 정합시킨다.
- **잔여 한계**: 직렬화 완료~데몬의 UpdateSnapshot 처리 사이 수 ms 창에 흘러온
  바이트는 (offset이 그보다 앞서므로) 크래시 시 이론상 유실될 수 있다. quit 경로는
  flush 후 즉시 처리라 실질 무시 가능하고, 라이브 30초 주기의 오차는 다음 스냅샷/
  링이 흡수한다. reader와 데몬 사이 완전 오프셋 동기화(ack)는 Phase 2 후보.

### 버전 협상 / 폴백 (additive, 재시도 협상)

협상은 additive다 — 어느 쪽이 신·구든 세션을 잃지 않는다.

- **데몬**: `Hello{proto}`를 `proto >= 1`이면 수락하고 `HelloOk{ min(proto,
  PROTO_VERSION) }`로 답한다(proto 0만 거부). 구프로토 클라이언트는 그 버전의
  메시지만 보내므로 안전하고, 미래 클라이언트(proto > 상한)는 상한으로 클램프된다.
- **클라이언트**: 먼저 `Hello{PROTO_VERSION}`을 보내 `HelloOk{p}`(1..=PROTO_VERSION)를
  받으면 그 `p`를 `Client`에 보관한다. **구데몬(proto 1)은 Hello{2}를 못 알아듣고
  Error로 답하되 연결은 유지하므로, Error를 받으면 같은 연결에서 `Hello{1}`로 1회
  재시도**해 `p=1`로 협상한다 — 앱을 이번 버전으로 업데이트해도 구데몬이 쥔 v1
  핸드오프 세션의 adopt 경로가 그대로 살아 있다(이 재시도가 없으면 기본 off인
  기존 v1 사용자도 업데이트 직후 세션을 잃는 회귀가 난다).
- **v2 게이팅**: 협상 `p < 2`면 v2 RPC 래퍼(spawn/attach/resize/wait/
  update_snapshot/data-attach)는 네트워크로 나가기 전에 즉시 `Err`를 낸다 →
  `BrokerPtyFactory`가 그 spawn을 `PortablePtyFactory`로 폴백한다(로그 남김).
  v1 메서드(handoff/list/adopt/kill)는 `p=1`에서도 그대로 동작한다.
- 데몬은 앱이 자기 자신을 `--sessiond`로 재실행해 띄우므로, 브로커 모드에서 앱이
  직접 스폰한 데몬의 proto는 항상 앱과 같은 2다. p=1 협상은 "소켓에 이미 떠 있는
  구데몬"을 만난 업데이트 직후에만 발생한다.

### 연결 모델 (BrokerPtyFactory)

세션 하나당 세 연결: **control**(Spawn 후 resize/kill RPC에 재사용, Mutex 보관),
**data**(DataAttach 후 raw — reader/writer가 이 소켓의 try_clone), **wait**(Wait
전용, 블로킹). `SpawnedPty` 계약(reader/writer/control/waiter)이 그대로
보존되므로 SessionManager는 팩토리 교체 외 거의 무변경이고, `handoff`/
`reader_interrupt`는 None(브로커 세션은 fd 핸드오프가 필요 없다).

`try_broker_spawn`은 Spawn RPC 성공 후 data/wait 연결 조립이 실패하면(소켓 경합
등) best-effort `Kill{agent_id}`로 그 브로커 세션을 정리한 뒤 에러를 돌려
폴백(in-process 스폰)을 타게 한다(§P2-a, 데몬에 자식만 남는 고아 방지).

### 세션 단위 소유 플래그 (broker_owned) — 혼합 상황 처리

전역 `broker_mode`만으로 handoff/adopt 경로를 가르면, 브로커 모드에서도 팩토리
폴백으로 생긴 in-process 세션을 오분류한다. 그래서 `SpawnedPty`에 **`broker_owned:
bool`**을 두고 세션 단위로 소유를 추적한다 — `BrokerPtyFactory` 성공 경로와 브로커
재접속(`assemble_broker_adopted`)만 true, `PortablePtyFactory`·팩토리 폴백·v1 fd
입양은 false. 이 플래그가 Session까지 전파돼:

- **handoff_all(브로커 모드)**: 세션마다 `broker_owned`로 분기 — true면 스냅샷
  업로드+detach(자식은 데몬 소유), false(폴백)면 **기존 v1 fd 핸드오프**(§P1-a).
  하나의 `connect_or_spawn` 연결이 두 경로(v1 Handoff + v2 UpdateSnapshot)를 모두
  처리한다(데몬은 proto 2). 반환 카운트는 두 경로 합.
- **adopt_detached(브로커 모드)**: List를 훑어 `broker=true`는 Attach+DataAttach로,
  `broker=false`(v1 핸드오프/폴백 세션)는 **기존 v1 adopt(fd 회수)**로 입양한다
  (§P1-b). 협상 p=1인 구데몬 상대로는 broker 항목이 없어 자연히 v1만 처리된다.
  **exited 브로커 항목(§P2-a)**: detach 중 자식이 죽으면 데몬 테이블에 exited
  엔트리가 남는데(브로커 세션은 종료돼도 Attach로 exit를 보고하려 테이블에 남긴다),
  이걸 그대로 두면 table-empty 종료가 막히는 누수가 된다. adopt는 exited *브로커*
  항목에 best-effort `Kill`을 보내 치운다(v1 exited 항목은 v1 수명 규칙대로 스킵).

### 스냅샷 압축·프레임 상한 (§P2-c)

직렬화 스냅샷이 ~3MiB를 넘으면 base64 JSON 프레임이 `MAX_FRAME_BYTES`(4MiB)를
초과해 데몬이 연결을 끊고 이후 업로드까지 전멸한다. 그래서 앱은 스냅샷을 `flate2`
deflate로 압축해 보내고(`UpdateSnapshot.compressed=true`), 데몬은 불투명 보관 +
플래그 보존, `AttachOk.snapshot_compressed`로 되돌려주면 앱이 입양 시 해제한다.
터미널 텍스트는 압축률이 높아(≈10×) 대부분 이걸로 충분하다. 그래도(비압축성 등)
인코딩 프레임이 상한을 넘으면 client 래퍼가 **그 agent만 스킵 + eprintln**하고
전송하지 않아 연결을 오염시키지 않는다(다음 주기에 화면이 줄면 재개). 참고: v1
`Handoff`의 snapshot도 같은 상한 이슈가 이론상 있으나 기존 동작이라 이번 범위 밖.

## 버전 스큐 (앱 업데이트 중 구버전 브로커)

- 원칙: 프로토콜은 **additive-only** (serde default 필드 추가만, 메시지 제거/의미
  변경 금지). Hello에서 proto 교환.
- unix에는 **드레인 업그레이드** 경로가 있음: 신버전 앱이 구브로커에서 v1의
  Adopt(SCM_RIGHTS fd 반환)로 세션을 전부 회수 → 구브로커 종료 → 신버전 브로커
  스폰 → Handoff로 재예치. **v1의 fd 전달 기계가 통째로 v2의 마이그레이션
  도구가 된다** — v1 작업은 버려지는 게 아니라 업그레이드 경로로 남는다.
- Windows는 fd 이전이 없으므로 드레인 불가 → additive-only 원칙 + 브로커
  프로토콜에 호환 깨짐이 필요할 때만 "세션 종료 후 브로커 교체" 안내.

## 단계별 계획

**Phase 1 — unix 브로커 모드 (핵심)**
- sessiond에 Spawn/Attach/Resize/Wait/KillAll 추가, 세션당 data conn 수락.
- BrokerPtyFactory 구현, lib.rs에서 팩토리 주입 교체(설정 플래그로 v1/v2 전환
  가능하게 — 초기엔 기본 off로 넣고 안정화 후 기본 on).
- 종료 모달 의미 전환, 부팅 재접속(=기존 adopt UI 재사용), 스냅샷 주기 업로드.
- 리스크: 브로커가 셸 스폰 주체가 되면서 세션 env/작업폴더/래퍼 파일 생성이
  브로커 프로세스 컨텍스트에서 일어남 — observer 래퍼 경로/env 주입을 브로커로
  넘기는 배선이 이 단계의 실제 공수 대부분.

**Phase 2 — 강건성**
- 브로커 panic.log(기존 앱 관례 이식), 앱의 브로커 헬스체크+재기동(세션 없을 때만),
  드레인 업그레이드 자동화, 크래시 생존 시나리오 테스트.

**Phase 3 — Windows**
- ConPTY 스폰을 브로커로, 전송을 named pipe로. `handoff_supported` →
  `broker_supported`로 개념 교체, Windows 모달도 3버튼 활성화.
- 실기기 검증 필요(가장 큰 미지수). unix와 코드 공유율을 높이기 위해 전송
  계층만 트레잇으로 추상화.

## 공수 감각

Phase 1이 v1 전체와 비슷한 규모(브로커 쪽 스폰 배선 이관이 무거움), Phase 2는
소, Phase 3은 중+실기기 검증. v1 산출물 재사용률: 프로토콜/데몬 골격/링버퍼/
poll reader/스냅샷/프론트 UI ≈ 그대로, 앱 쪽 handoff_all만 드레인 전용으로 강등.

## 권고 (결정 필요)

1. **PR #6은 지금 머지**하고 v2를 그 위에 쌓는 것을 권함 — v2의 기반 부품이
   전부 PR #6 안에 있고, v2 완성 전까지는 v1이 "업데이트 시 터미널 존속"을
   즉시 제공한다. 버릴 코드는 사실상 handoff_all 호출부뿐이며 그마저 업그레이드
   드레인으로 남는다.
2. v2 착수 시점과 Phase 3(Windows) 포함 여부.
3. 스냅샷 업로드 주기(기본안 30초) — 크래시 생존 화면 복원의 신선도를 결정.
