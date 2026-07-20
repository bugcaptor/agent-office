# 구독 사용량(rate limit) 표시 설계 — 이슈 #22

상태: 이력(archived, 2026-07-20) — 본문이 `docs/usage-design.md`로 병합됨(그쪽이 정본). 이슈 #22 링크 보존용.

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
- `limits[]`의 `is_active`는 **"지금 구속 중인 윈도"인지를 뜻할 뿐 유효성이 아니다**(실측: weekly_all/weekly_scoped가 `is_active:false`로 오지만 살아 있는 주간 한도임). 필터링에 쓰지 말 것 — 와이어 `UsageWindow.isActive`로 그대로 전달해 표시용 보조 정보(예: "지금 적용 중" 태그)로만 쓴다. five_hour/seven_day 폴백 경로와 Codex는 개념이 없어 항상 null.

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
- 스캔 전략: `sessions/YYYY/MM/DD` **모든** 날짜 디렉터리에서 rollout 파일을 모아 파일 mtime 내림차순(날짜 디렉터리 경계 없이 전역)으로 정렬한 뒤, 각 파일의 마지막 non-null `rate_limits` 라인(스냅샷)을 파싱해 스냅샷 자체의 timestamp(`fetched_at_ms`)가 가장 큰 후보를 취한다. 날짜 디렉터리 컷오프는 두지 않는다 — 장기 실행 세션의 rollout 파일은 세션 "시작" 날짜 디렉터리에 계속 append되므로, 최근 날짜 디렉터리 개수로 컷오프하면 새 날짜 디렉터리가 그 개수 이상 생기는 순간 mtime이 가장 최신인 파일(=가장 신선한 스냅샷)이 스캔에서 통째로 배제되는 버그가 생긴다. 조기 종료: 현재 최선 후보의 `fetched_at_ms`가 다음 파일의 mtime 이상이면 그 뒤(mtime이 더 낮은) 파일들은 이를 넘어설 수 없으므로 중단. 스캔 비용 상한은 날짜 컷오프 대신 파싱 파일 수 상한(`MAX_PARSED_FILES = 64`)으로 둔다 — 대상 파일 목록 수집·mtime 정렬은 전체 날짜 디렉터리 대상, 상한은 `parse_file` 호출 횟수에만 적용.
- 파일 내부 스캔(`parse_file`): 장기 세션 rollout은 수백 MB가 될 수 있어 `read_to_string`으로 전체를 읽지 않는다 — 파일 끝에서부터 `TAIL_CHUNK_BYTES`(64KB) 단위로 역방향 청크를 읽어 완성된 라인을 EOF 쪽부터 검사하고, 청크 경계에 걸린 라인은 다음(더 앞쪽) 청크와 이어붙여(carry) 처리한다. 상주 메모리는 청크 1개 + 경계에 걸친 미완결 라인 수준. 파일당 `MAX_TAIL_SCAN_BYTES`(8MB)까지만 훑고 못 찾으면 그 파일은 포기하고 다음 파일로 — rate_limits 스냅샷은 `token_count` 이벤트마다 기록되므로 유효한 파일이라면 스냅샷이 항상 꼬리 근처에 있다는 전제.

## 3. 아키텍처

기존 관례(session-analytics)를 그대로 따른다: 백엔드는 원시 스냅샷만 반환, 해석·표시는 프런트. 새 IPC 커맨드는 5접점 계약 준수.

### 백엔드 — `src-tauri/src/usage/`

- `mod.rs` — `load_usage_snapshot(claude_root, codex_root) -> UsageSnapshot` 조립.
- `claude.rs` — `<claude_root>/.claude.json` 파싱. 루트 경로를 인자로 받아 테스트에서 tempdir 주입.
- `codex.rs` — `<codex_root>/sessions` 스캔. 동일하게 루트 주입.
- IPC 커맨드 `load_usage_snapshot` (인자 없음) → `UsageSnapshot`. 실패한 소스는 해당 provider가 `null`일 뿐 커맨드는 성공한다. 루트 경로는 기본 홈 디렉터리 하위(`~/.codex`, `~/.claude.json`)이되, CLI가 실제로 존중하는 표준 환경변수 오버라이드를 지원한다: Codex는 `CODEX_HOME`(설정 시 `<CODEX_HOME>/sessions`), Claude는 `CLAUDE_CONFIG_DIR`(설정 시 `<CLAUDE_CONFIG_DIR>/.claude.json` — `claude.rs::load`의 파일명 결합 로직은 그대로, 루트만 바뀐다). 빈 문자열 env는 미설정으로 취급. 전역 `std::env::var` 접근과 분리한 순수 함수 `ipc::commands::resolve_usage_roots(home, codex_home_env, claude_config_env) -> (PathBuf, PathBuf)`로 테스트한다(전역 env를 건드리지 않고 조합 검증).

### 와이어 타입 (`src/shared/types.ts` ↔ Rust serde 미러)

```ts
type UsageWindowKind = "session" | "weekly" | "weekly_model" | "unknown";
interface UsageWindow {
  kind: UsageWindowKind;
  label: string | null;        // weekly_model일 때 모델 표시명 등
  usedPercent: number;
  resetsAtMs: number | null;   // epoch ms로 정규화 (Claude ISO·Codex 초 모두 백엔드에서 변환)
  windowMinutes: number | null;
  isActive: boolean | null;    // "지금 구속 중인 윈도" 표시(유효성 아님). Claude limits[]만, 나머지는 null
}
interface ProviderUsage {
  provider: "claude" | "codex";
  fetchedAtMs: number;         // 신선도 기준 시각
  planLabel: string | null;    // codex plan_type, claude oauthAccount.organizationRateLimitTier(루트 폴백) 등
  windows: UsageWindow[];
}
interface UsageSnapshot { claude: ProviderUsage | null; codex: ProviderUsage | null; }
```

- 단위 정규화는 전부 백엔드에서: resets_at → epoch ms, 신선도 → epoch ms.
- `windows`는 가변 배열 — UI가 "5시간+주간 둘 다 있음"을 하드코딩하지 않는다.

### 프런트 — `src/renderer/usage/`

- `UsageWidget.tsx` — BottomBar에 상시 표시되는 컴팩트 게이지. provider별로 **가장 절박한 윈도**(usedPercent 최대) 하나를 뱃지로. 색상: <70 기본, ≥70 경고, ≥90 위험(tokens.css 토큰 사용). 데이터 없으면 dim 처리한 `—`. 뱃지 마크업은 `<span class="usage-badge-label">CL</span> <span class="usage-badge-pct">61%</span>` — 라벨과 퍼센트를 별도 span으로 분리해 usage.css가 폭에 따라 라벨만 숨길 수 있게 한다(아래 §BottomBar 800px 참고).
- `UsageDialog.tsx` — 클릭 시 ModalState `{ kind: "usage" }` 모달. 윈도별 픽셀 바(사용률), 리셋까지 남은 시간 카운트다운("3시간 12분 후 리셋"), 마지막 갱신("14분 전 기준, Claude 실행 중에만 갱신됨" 안내). stale(>30분)이면 흐리게 + 표시.
- 폴링: `UsageWidget`이 마운트 시 + 60초 간격으로 `loadUsageSnapshot()` invoke, zustand store에 저장. 카운트다운 표시는 `SessionTimePanel`의 1초 tick 패턴 재사용(로컬 시계, 재조회 아님).

#### BottomBar 800px 기본 폭 (실측 2026-07-17)

기본 창 크기(`tauri.conf.json` 800×600)에서 BottomBar는 위젯 추가 이전부터 이미 여유가 거의 없다. 실측(실제 렌더러를 800px 뷰포트로 띄워 각 자식의 렌더 폭 측정): DungGeunMo 픽셀 폰트는 한글을 글자당 고정폭(≈16px)으로 그려 일반 산세리프보다 오히려 넓고, 기존 버튼(+New Agent/출근/전체 출근·퇴근/분석/설정/테마/알림) 8개 + 상태 텍스트("N running · M needs input", 자연폭 ≈160px)만으로도 800px를 거의 다 쓴다.

- `.bottom-bar > .pixel-btn`는 `flex-shrink: 0` + `white-space: nowrap`으로 고정 — flexbox 기본 shrink가 텍스트를 여러 줄로 접어 바를 깨뜨리는 걸 막는다(폭이 줄면 버튼이 줄바꿈되는 대신 항상 한 줄 그대로).
- `.bottom-bar-status`는 `flex:1; min-width:0` + `white-space:nowrap; overflow:hidden; text-overflow:ellipsis` — 폭이 부족하면 줄바꿈 대신 말줄임으로 흡수한다. 800px 기본 폭 + 아래 위젯 압축형 기준으로는 실측상 말줄임이 사실상 트리거되지 않는다(여유 ≈1px 이내).
- `.usage-widget`(및 내부 `.usage-badge-label`)의 **기본 표현은 CL/CX 라벨 없이 퍼센트만**(`72% 11%`) — 800px에서 라벨까지 넣으면 위 여유가 없어 상태 텍스트가 말줄임된다. 라벨은 `@media (min-width: 900px)`에서만 나타난다(그 지점부터는 라벨을 포함해도 다른 컨트롤을 밀어내지 않을 여유가 생김). 전체 정보(어느 provider인지)는 항상 `title` 속성(호버)과 클릭 시 여는 상세 모달로 확인 가능하므로 라벨 생략은 정보 손실이 아니라 표시 축약이다.
- `.bottom-bar` 자체에 `overflow-x: auto`를 둔다 — 창에 최소 폭 제약이 없어(tauri.conf.json에 `minWidth` 없음) 위 압축이 다 적용돼도 부족할 만큼 좁아지면(예: 550px 아래, 테마 라벨이 긴 "미드나이트" + 두 자리 카운트가 겹치는 등) 오른쪽 끝 버튼(음소거 등)이 화면 밖으로 밀려날 수 있다 — 줄바꿈 없음보다 "모든 컨트롤에 (스크롤로라도) 닿을 수 있음"을 우선해 가로 스크롤을 안전망으로 둔다.
- 검증 방법: 실제 vite dev 서버(`npm run dev`) + Chrome 자동화로 800/550/950px 뷰포트에 실제 렌더러를 띄워 각 자식 요소의 `getBoundingClientRect()`·`scrollWidth`를 측정. 정적 스냅샷/단위 테스트만으로는 실제 폰트 메트릭(DungGeunMo가 한글에서 오히려 넓다는 사실)을 포착하지 못해 이 방식으로 확인했다.

## 4. 테스트

- Rust: 픽스처 JSON을 tempdir에 써놓고 파싱 검증 — Claude(limits[] 우선/폴백/파손 파일→None), Codex(null 스킵, window_minutes 매핑, 최신 파일 우선, 초→ms 변환, 청크 경계에 걸친 긴 라인 이어붙이기, `MAX_TAIL_SCAN_BYTES` 상한 밖 스냅샷→None). `ipc::commands::resolve_usage_roots` 순수 함수로 `CODEX_HOME`/`CLAUDE_CONFIG_DIR` 오버라이드 조합(미설정/한쪽만/둘 다/빈 문자열) 검증.
- TS: 계약 왕복(`shared/__tests__/contract.test.ts` 패턴에 UsageSnapshot 픽스처 추가), 위젯/다이얼로그의 절박 윈도 선택·카운트다운 포맷 순수 함수 테스트.

## 5. 트레이드오프 기록

- **stale 허용**: CLI 미사용 구간에는 값이 멈춘다. 신선도 표시로 사용자에게 알리는 것으로 충분하다고 판단. 능동 조회(OAuth)는 미공개 API 의존이라 보류.
- **프런트 60초 폴링**: 파일 읽기가 저비용이라 파일 워처·백엔드 타이머 없이 단순 폴링 채택.
- Codex는 `plan_type`별 윈도 구성이 달라 UI는 배열 기반으로 렌더.
