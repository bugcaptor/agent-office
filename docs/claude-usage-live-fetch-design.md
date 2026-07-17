# Claude 사용량 실시간 조회 설계 (이슈 #33)

상태: 설계 확정 (2026-07-18). 기반: docs/usage-limits-design.md (캐시 미러 v1).

## 1. 문제와 목표

v1(이슈 #22)은 `~/.claude.json`의 `cachedUsageUtilization`을 미러한다. 이 캐시는
Claude Code CLI가 자체 판단으로만 재fetch하므로, 리셋 경계가 지나도 낡은 값이
남는다. 실측(07-18 00:22 KST): 5시간 창이 23:39 KST 리셋됐는데 캐시는 리셋 1분
전 값(62%)으로 45분째 고정.

목표: Claude Code CLI가 내부적으로 쓰는 사용량 엔드포인트를 앱이 직접 호출해
리셋 경계 후 ≤1분 내 실제 값을 표시한다. 실패 시 현행 캐시 미러가 자연 폴백.

비목표(범위 제외): OAuth 토큰 리프레시(orca의 CLI 수리·PTY 스크래핑 경로),
Codex 쪽 실시간화(rollout rate_limits 유지), 렌더러 계약 변경.

## 2. 데이터 소스 (참고 구현: `_ref/orca/src/main/rate-limits/claude-fetcher.ts`)

### 2.1 엔드포인트

```
GET https://api.anthropic.com/api/oauth/usage
Authorization: Bearer <액세스 토큰>
anthropic-beta: oauth-2025-04-20
User-Agent: claude-code/2.1.0
```

- 타임아웃 10초. 응답 본문 루트가 `cachedUsageUtilization.utilization`과 같은
  모양(`five_hour`/`seven_day`/`limits[]`)이므로 **기존 `claude.rs`의
  `parse_limits`/`parse_fallback`을 그대로 재사용**한다 (pub(super)로 승격).
- ⚠️ 비공식 API. 계약 변경 시 파싱이 None → 폴백 경로로 자연 강등되어 표시가
  죽지는 않는다. UA를 CLI와 맞추는 것이 계약의 일부다.

### 2.2 토큰 출처 (읽기 전용, 우선순위 순)

1. macOS Keychain 스코프 항목: `CLAUDE_CONFIG_DIR` 설정 시
   `Claude Code-credentials-<sha256(dir) hex 앞 8자>`
2. macOS Keychain 레거시 항목: `Claude Code-credentials`
3. 파일: `<config_dir>/.credentials.json` (config_dir = `CLAUDE_CONFIG_DIR` 또는 `~/.claude`)

- Keychain 읽기는 `security find-generic-password -s <service> -a $USER -w`
  자식 프로세스로 (비-macOS는 파일만). 값 JSON 모양은 세 출처 공통:
  `{"claudeAiOauth":{"accessToken":"..."}}`.
- Keychain 자식 프로세스는 **5초 타임아웃**(tokio::process + kill_on_drop) —
  잠긴 Keychain·방치된 권한 다이얼로그로 `security`가 매달려도 폴링 경로가
  막히지 않고 파일→캐시 폴백으로 강등된다(PR #34 리뷰 P2 반영).
- `anthropic-version` 헤더는 보내지 않는다 — 참고 구현(orca)이 동일 헤더
  구성으로 프로덕션 동작 중이고, CLI 흉내 계약상 CLI가 안 보내는 헤더를
  추가하지 않는다. 실 계약은 `#[ignore]` smoke로 확인.
- 로컬 `expiresAt`으로 만료 판정하지 않는다 — orca 실측상 만료시각 후에도
  인증되는 사례가 있어 서버 401이 판정자다. 401이면 그냥 실패(→폴백).
- **토큰을 로그·에러 메시지에 절대 포함하지 않는다.**
- 첫 Keychain 접근 시 macOS 권한 프롬프트 1회 예상. 거부하면 security가
  에러 → 파일 폴백 → (파일도 없으면) 캐시 미러. 어느 경로든 앱은 정상.

## 3. 구조

### 3.1 새 모듈 `src-tauri/src/usage/claude_live.rs`

- `read_access_token(claude_config_dir) -> Option<String>` — §2.2 순서.
  JSON 파싱은 순수 함수 `parse_access_token(json) -> Option<String>` 분리.
- `fetch_live(token) -> Option<Vec<UsageWindow>>` — reqwest async, §2.1.
  응답 파싱은 순수 함수 `parse_live_response(&Value)` 분리(픽스처 테스트).
- `LiveUsageState` — AppState에 보관 (`parking_lot::Mutex<LiveUsageInner>`):
  - `last_success: Option<ProviderUsage>` (fetched_at_ms 포함)
  - `last_attempt_ms: Option<i64>`
- 스로틀 판단은 **순수 함수** `should_fetch(view, now_ms) -> bool`:
  - 성공 스냅샷이 없으면 → fetch
  - 마지막 시도 후 `MIN_ATTEMPT_GAP_MS`(5분) 미만이면 → 안 함 (성공/실패
    공통 하한 — 실패 백오프를 겸함)
  - 마지막 성공 후 `REFRESH_INTERVAL_MS`(15분) 경과 → fetch
  - **리셋 경계 조기 리프레시**: 마지막 성공 스냅샷의 윈도 중
    `resets_at_ms < now`인 것이 있으면 15분을 기다리지 않고 fetch
    (5분 하한은 여전히 적용). 렌더러 60초 폴링에 얹혀 리셋 후 빠르게 갱신
    — 이 이슈의 핵심 동기.
- 동시성: Mutex를 잡은 채 await 금지. 판단 시점에 `last_attempt_ms`를 먼저
  갱신해 두면(락 안에서) 60초 폴링 경합의 중복 fetch가 자연 차단된다.

### 3.2 조립 변경 (`usage/mod.rs` + `ipc/commands.rs`)

- 새 진입점 `load_usage_snapshot_with_live(live: &LiveUsageState, claude_root, codex_root, now_ms) -> UsageSnapshot`
  (async). 커맨드는 이것에 위임. 기존 `load_usage_snapshot`(동기, 파일만)은
  내부 구성요소로 유지.
- claude 필드 결정: live 시도(스로틀 통과 시) → 성공하면 메모리 상태 갱신.
  최종적으로 **파일 캐시와 메모리 live 중 `fetched_at_ms` 큰 쪽**을 반환
  (Claude Code가 방금 캐시를 갱신했다면 그쪽이 이길 수 있어야 함 — 렌더러
  `fresherProvider`와 같은 규칙을 백엔드에도 적용).
- `plan_label`은 live 응답에 없으므로 파일 캐시의 값을 live 결과에 접목.
- 커맨드 시그니처: `load_usage_snapshot(state: tauri::State<'_, AppState>)`.
  기존 커맨드 테스트 관례대로 본체 로직은 AppState 없이 호출 가능한 함수로
  두고 커맨드는 한 줄 위임.

### 3.3 렌더러

변경 없음. `UsageSnapshot` 계약 그대로이고, 신선도·stale 흐림·카운트다운은
기존 usageView 로직이 그대로 처리한다. contract.test.ts 영향 없음.

## 4. 테스트

- `parse_access_token`: 정상/키 부재/깨진 JSON.
- `parse_live_response`: 실 API 응답 모양 픽스처(limits[] 포함, five_hour
  폴백, 빈 응답) — 기존 파서 재사용 확인.
- `should_fetch`: 최초/15분 경과/5분 하한/리셋 경계 조기/실패 직후 케이스.
- 조립: live 성공이 파일 캐시를 이기는 경우, 파일이 더 신선한 경우,
  live 실패 시 파일 폴백, plan_label 접목.
- 실 API smoke: `#[ignore]` 테스트 1개(토큰 있으면 실호출) — 사용자 수동.
- HTTP 호출부는 얇게 유지(파싱·판단이 전부 순수 함수라 네트워크 목 불필요).

## 5. 실앱 눈검증 시나리오

1. 앱 시작 → (최초 1회) Keychain 권한 프롬프트 허용 → 사용량 위젯이 1분 내
   "방금 기준"으로 갱신되는지.
2. 5시간 창 리셋 경계를 지난 뒤(또는 캐시가 낡은 상태에서) 리셋된 %가
   1~6분 내 반영되는지.
3. 네트워크 차단 상태에서 앱이 캐시 미러로 정상 폴백하는지(에러 무표시,
   stale 흐림만).
