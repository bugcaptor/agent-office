# 세션 브로커 v2 계획 — 상시 브로커(스폰부터 데몬이 PTY 소유)

작성: 2026-07-17 (Fable). 상태: **계획 단계 (구현 미착수, 승인 대기)**.
전제: v1(feat/session-handoff, PR #6) = 종료 시점 핸드오프. v2는 그 역전.

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
