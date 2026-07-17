# 구독 사용량(rate limit) 표시 설계 — 이슈 #22

Claude Code와 Codex CLI 구독 정액제의 시간별(5시간 세션)·주간 한도 사용률과 리셋까지 남은 시간을 앱에 표시한다.

## 1. 목표 / 비목표

**목표**
- Claude·Codex 각각의 활성 한도 윈도(5시간 세션, 주간, 모델별 주간)의 사용률(%)과 리셋 시각을 표시.
- 데이터의 신선도(마지막 갱신 시각)를 함께 표시 — 두 소스 모두 CLI가 실제로 돌 때만 갱신되는 캐시이므로 필수.
- BottomBar 상시 컴팩트 게이지 + 클릭 시 상세 모달.

**비목표 (v1)**
- Anthropic 미공개 OAuth usage 엔드포인트(`POST api.anthropic.com/api/oauth/usage`) 능동 호출. 미공개 API·429 위험·토큰 갱신 충돌 때문에 v1에서는 로컬 캐시 파일만 읽는다. stale 문제는 신선도 표시로 완화. (향후 확장 여지로만 기록)
- 토큰 수/비용 집계(ccusage류). 이 기능은 "한도 대비 %와 리셋 시각"만 다룬다.
- 백엔드 백그라운드 타이머. 갱신은 프런트 주기 폴링(온디맨드 invoke)으로 충분.

## 2. 데이터 소스 (2026-07 실측)

### Claude Code — `~/.claude.json` → `cachedUsageUtilization`

```json
"cachedUsageUtilization": {
  "fetchedAtMs": 1784281391475,
  "utilization": {
    "five_hour": { "utilization": 61, "resets_at": "2026-07-17T09:50:00+00:00" },
    "seven_day": { "utilization": 18, "resets_at": "2026-07-21T04:00:00+00:00" },
    "limits": [
      { "kind": "session",       "percent": 61, "severity": "normal", "resets_at": "...", "is_active": true },
      { "kind": "weekly_all",    "percent": 18, "resets_at": "..." },
      { "kind": "weekly_scoped", "percent": 24, "resets_at": "...", "scope": {"model": {"display_name": "Fable"}} }
    ]
  }
}
```

- `limits[]`가 있으면 우선 사용(더 구조화·모델별 주간 포함), 없으면 `five_hour`/`seven_day` 폴백.
- `resets_at`은 timezone 포함 ISO8601. `fetchedAtMs`가 신선도.
- 파일이 크고(100KB+) CLI가 세션 중 자주 rewrite하므로: `cachedUsageUtilization` 키만 추출, 파싱 실패 시 조용히 None(이전 값 유지·재시도는 프런트 폴링이 담당).
- transcript(`~/.claude/projects/**.jsonl`)·`stats-cache.json`에는 한도 정보 없음 — 소스로 쓰지 말 것.

### Codex CLI — `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`

`token_count` 이벤트에 서버가 준 `rate_limits` 스냅샷이 append된다:

```json
{ "timestamp": "2026-07-17T11:01:49.074Z", "type": "event_msg", "payload": { "type": "token_count",
  "rate_limits": {
    "primary":   { "used_percent": 11.0, "window_minutes": 10080, "resets_at": 1784786662 },
    "secondary": null,
    "plan_type": "prolite"
  } } }
```

- **`window_minutes`로 윈도 종류 판별** (300=5시간, 10080=주간). primary/secondary 위치로 가정 금지 — 플랜에 따라 구성이 다름(prolite는 주간 하나뿐).
- `resets_at`은 **유닉스 초** (Claude와 단위 다름). 이벤트 `timestamp`가 신선도.
- `rate_limits`가 null이거나 primary/secondary 모두 null인 이벤트가 흔함 → **가장 최근 non-null**을 찾는다.
- 스캔 전략: 최근 7일치 날짜 디렉터리만, 파일 mtime 내림차순으로 순회하며 각 파일에서 마지막 non-null `rate_limits` 라인을 찾고, 처음 발견되면 중단.

## 3. 아키텍처

기존 관례(session-analytics)를 그대로 따른다: 백엔드는 원시 스냅샷만 반환, 해석·표시는 프런트. 새 IPC 커맨드는 5접점 계약 준수.

### 백엔드 — `src-tauri/src/usage/`

- `mod.rs` — `load_usage_snapshot(claude_root, codex_root) -> UsageSnapshot` 조립.
- `claude.rs` — `~/.claude.json` 파싱. 루트 경로를 인자로 받아 테스트에서 tempdir 주입.
- `codex.rs` — rollout 스캔. 동일하게 루트 주입.
- IPC 커맨드 `load_usage_snapshot` (인자 없음) → `UsageSnapshot`. 실패한 소스는 해당 provider가 `null`일 뿐 커맨드는 성공한다.

### 와이어 타입 (`src/shared/types.ts` ↔ Rust serde 미러)

```ts
type UsageWindowKind = "session" | "weekly" | "weekly_model" | "unknown";
interface UsageWindow {
  kind: UsageWindowKind;
  label: string | null;        // weekly_model일 때 모델 표시명 등
  usedPercent: number;
  resetsAtMs: number | null;   // epoch ms로 정규화 (Claude ISO·Codex 초 모두 백엔드에서 변환)
  windowMinutes: number | null;
}
interface ProviderUsage {
  provider: "claude" | "codex";
  fetchedAtMs: number;         // 신선도 기준 시각
  planLabel: string | null;    // codex plan_type, claude organizationRateLimitTier 등
  windows: UsageWindow[];
}
interface UsageSnapshot { claude: ProviderUsage | null; codex: ProviderUsage | null; }
```

- 단위 정규화는 전부 백엔드에서: resets_at → epoch ms, 신선도 → epoch ms.
- `windows`는 가변 배열 — UI가 "5시간+주간 둘 다 있음"을 하드코딩하지 않는다.

### 프런트 — `src/renderer/usage/`

- `UsageWidget.tsx` — BottomBar에 상시 표시되는 컴팩트 게이지. provider별로 **가장 절박한 윈도**(usedPercent 최대) 하나를 뱃지로: `CL 61%` `CX 11%`. 색상: <70 기본, ≥70 경고, ≥90 위험(tokens.css 토큰 사용). 데이터 없으면 dim 처리한 `—`.
- `UsageDialog.tsx` — 클릭 시 ModalState `{ kind: "usage" }` 모달. 윈도별 픽셀 바(사용률), 리셋까지 남은 시간 카운트다운("3시간 12분 후 리셋"), 마지막 갱신("14분 전 기준, Claude 실행 중에만 갱신됨" 안내). stale(>30분)이면 흐리게 + 표시.
- 폴링: `UsageWidget`이 마운트 시 + 60초 간격으로 `loadUsageSnapshot()` invoke, zustand store에 저장. 카운트다운 표시는 `SessionTimePanel`의 1초 tick 패턴 재사용(로컬 시계, 재조회 아님).

## 4. 테스트

- Rust: 픽스처 JSON을 tempdir에 써놓고 파싱 검증 — Claude(limits[] 우선/폴백/파손 파일→None), Codex(null 스킵, window_minutes 매핑, 최신 파일 우선, 초→ms 변환).
- TS: 계약 왕복(`shared/__tests__/contract.test.ts` 패턴에 UsageSnapshot 픽스처 추가), 위젯/다이얼로그의 절박 윈도 선택·카운트다운 포맷 순수 함수 테스트.

## 5. 트레이드오프 기록

- **stale 허용**: CLI 미사용 구간에는 값이 멈춘다. 신선도 표시로 사용자에게 알리는 것으로 충분하다고 판단. 능동 조회(OAuth)는 미공개 API 의존이라 보류.
- **프런트 60초 폴링**: 파일 읽기가 저비용이라 파일 워처·백엔드 타이머 없이 단순 폴링 채택.
- Codex는 `plan_type`별 윈도 구성이 달라 UI는 배열 기반으로 렌더.
