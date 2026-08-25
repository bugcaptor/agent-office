# 구독 사용량(usage) 표시 설계 — 캐시 미러 + 실시간 조회(Claude·Codex·Antigravity)

상태: 정본 — 구현 완료 (v1 캐시 미러 = 이슈 #22, Claude live fetch = 이슈 #33/PR #34, 실패 사유 표시 = §7, Codex live fetch = §9 / kbm #2h8, 하루 넘게 실패한 provider 숨김 = §10 · Antigravity provider = §11 / kbm #2j4, 2026-08-25). 병합: 2026-07-20 (`usage-limits-design.md` + `claude-usage-live-fetch-design.md` 통합, 원본은 `docs/archive/`).

구현 파일: 백엔드 `src-tauri/src/usage/{mod,claude,codex,claude_live,claude_live_fallback,codex_live,antigravity_live}.rs`, 커맨드 `load_usage_snapshot`·`resolve_usage_roots`는 `ipc/commands/usage.rs`. 프런트 `src/renderer/usage/{UsageWidget,UsageDialog}.tsx`·`usageView.ts`. 와이어 타입은 `src/shared/types/usage.ts`(배럴 `shared/types.ts` 경유).

세션 활동 분석(작업시간 시계열)은 별개 기능 — `docs/session-analytics-design.md` 참조.

Claude Code·Codex CLI·Antigravity 구독 정액제의 시간별(5시간 세션)·주간 한도 사용률과 리셋까지 남은 시간을 앱에 표시한다. 하루 넘게 값을 못 가져온 provider는 표시하지 않는다(§10).

## 1. 목표 / 비목표

**목표**
- Claude·Codex 각각의 활성 한도 윈도(5시간 세션, 주간, 모델별 주간)의 사용률(%)과 리셋 시각을 표시.
- 데이터의 신선도(마지막 갱신 시각)를 함께 표시 — 캐시 소스는 CLI가 실제로 돌 때만 갱신되므로 필수.
- BottomBar 상시 컴팩트 게이지 + 클릭 시 상세 모달.

**비목표**
- 이 기능(BottomBar 게이지) 안에서의 토큰 수/비용 집계. 여기서는 "한도 대비 %와
  리셋 시각"만 다룬다.
- 백엔드 백그라운드 타이머. 갱신은 프런트 주기 폴링(온디맨드 invoke)으로 충분.
- Codex 쪽 실시간화(rollout rate_limits 캐시 유지).

> v1(#22)에서는 Anthropic 미공개 OAuth usage 엔드포인트 능동 호출도 비목표였으나,
> 리셋 경계 후 캐시가 낡는 문제가 실측돼 #33에서 Claude에 한해 도입했다(§6).

> 토큰 수/비용 집계 자체는 더 이상 비목표가 아니다 — **세션 활동 분석 패널**이
> 턴 단위 토큰과 API 환산 추정 비용을 보여준다(`docs/session-analytics-design.md`
> §7). 다만 그 집계는 이 게이지와 데이터 소스도 표시 위치도 다르다: 게이지는
> 구독 한도 스냅샷(`rate_limits`/`cachedUsageUtilization`)을, 분석 패널은 세션
> 이벤트 시계열의 stop 레코드에 실린 턴 사용량을 쓴다.

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

- **이 캐시는 CLI가 `/usage` 화면을 열 때만 갱신된다**(2026-07-29 CLI 2.1.220 바이너리 실측). 쓰는 함수는 하나뿐이고(`cachedUsageUtilization` 대입), 그 함수를 부르는 곳은 `loadPlanRateLimits`(= 앱과 같은 `GET /api/oauth/usage` 호출)뿐이며, 그것을 부르는 건 `/usage` 패널의 effect와 SDK `onGetUsage` 요청이다. 일반 대화 중 CLI가 응답 헤더(`anthropic-ratelimit-unified-*`)로 받는 사용률은 메모리에만 있고(`source:"headers"`) 파일에 쓰지 않는다.
  - 따라서 **"에이전트를 돌리면 캐시가 갱신된다"는 명제는 거짓이다** — 앱 안에서 돌리든 사용자가 터미널에서 돌리든 마찬가지다(실측 사례: `fetchedAtMs`가 11일간 고정). 이 사실이 §6 실시간 조회의 존재 이유이고, 실패 시 UI가 사용자에게 말해야 할 내용이기도 하다(§7).
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

- `mod.rs` — `load_usage_snapshot(claude_root, codex_root) -> UsageSnapshot` 조립(동기, 파일만). live 결합 진입점은 `load_usage_snapshot_with_live`(§6).
- `claude.rs` — `<claude_root>/.claude.json` 파싱. 루트 경로를 인자로 받아 테스트에서 tempdir 주입.
- `codex.rs` — `<codex_root>/sessions` 스캔. 동일하게 루트 주입.
- `claude_live.rs` — Claude OAuth usage 능동 조회(§6).
- `codex_live.rs` — codex app-server RPC 조회(§9).
- `antigravity_live.rs` — `agy -p /usage` print 모드 조회(§11). 파일 캐시가 없어 이 경로가 유일한 소스다.
- IPC 커맨드 `load_usage_snapshot` (인자 없음) → `UsageSnapshot`. 실패한 소스는 해당 provider가 `null`일 뿐 커맨드는 성공한다. 루트 경로는 기본 홈 디렉터리 하위(`~/.codex`, `~/.claude.json`)이되, CLI가 실제로 존중하는 표준 환경변수 오버라이드를 지원한다: Codex는 `CODEX_HOME`(설정 시 `<CODEX_HOME>/sessions`), Claude는 `CLAUDE_CONFIG_DIR`(설정 시 `<CLAUDE_CONFIG_DIR>/.claude.json` — `claude.rs::load`의 파일명 결합 로직은 그대로, 루트만 바뀐다). 빈 문자열 env는 미설정으로 취급. 전역 `std::env::var` 접근과 분리한 순수 함수 `resolve_usage_roots(home, codex_home_env, claude_config_env) -> (PathBuf, PathBuf)`(`ipc/commands/usage.rs`)로 테스트한다(전역 env를 건드리지 않고 조합 검증).

### 와이어 타입 (`src/shared/types/usage.ts` ↔ Rust serde 미러)

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

- Rust: 픽스처 JSON을 tempdir에 써놓고 파싱 검증 — Claude(limits[] 우선/폴백/파손 파일→None), Codex(null 스킵, window_minutes 매핑, 최신 파일 우선, 초→ms 변환, 청크 경계에 걸친 긴 라인 이어붙이기, `MAX_TAIL_SCAN_BYTES` 상한 밖 스냅샷→None). `resolve_usage_roots` 순수 함수로 `CODEX_HOME`/`CLAUDE_CONFIG_DIR` 오버라이드 조합(미설정/한쪽만/둘 다/빈 문자열) 검증.
- TS: 계약 왕복(`shared/__tests__/contract.test.ts` 패턴에 UsageSnapshot 픽스처 추가), 위젯/다이얼로그의 절박 윈도 선택·카운트다운 포맷 순수 함수 테스트.
- live fetch 테스트는 §6.4.

## 5. 트레이드오프 기록

- **stale 허용(캐시 소스)**: CLI 미사용 구간에는 값이 멈춘다. 신선도 표시로 사용자에게 알리는 것으로 충분하다고 판단 — 이후 Claude에 한해 능동 조회(§6)로 보완.
- **프런트 60초 폴링**: 파일 읽기가 저비용이라 파일 워처·백엔드 타이머 없이 단순 폴링 채택.
- Codex는 `plan_type`별 윈도 구성이 달라 UI는 배열 기반으로 렌더.

---

## 6. Claude 사용량 실시간 조회 (이슈 #33, 캐시 미러의 보완)

### 6.1 문제와 목표

캐시 미러(§2)는 Claude Code CLI가 자체 판단으로만 재fetch하므로, 리셋 경계가
지나도 낡은 값이 남는다. 실측(07-18 00:22 KST): 5시간 창이 23:39 KST 리셋됐는데
캐시는 리셋 1분 전 값(62%)으로 45분째 고정.

목표: Claude Code CLI가 내부적으로 쓰는 사용량 엔드포인트를 앱이 직접 호출해
리셋 경계 후 ≤1분 내 실제 값을 표시한다. 실패 시 캐시 미러가 자연 폴백.

비목표(범위 제외): OAuth 토큰 리프레시, Codex 쪽 실시간화, 렌더러 계약 변경.

### 6.2 데이터 소스 (참고 구현: `_ref/orca/src/main/rate-limits/claude-fetcher.ts`)

**엔드포인트**

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

**토큰 출처 (읽기 전용, 우선순위 순)**

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

### 6.3 구조

**모듈 `src-tauri/src/usage/claude_live.rs`**

- `read_access_token(claude_config_dir) -> Option<String>` — §6.2 순서.
  JSON 파싱은 순수 함수 `parse_access_token(json) -> Option<String>` 분리.
- `fetch_live(token) -> Option<Vec<UsageWindow>>` — reqwest async.
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
    — 이 기능의 핵심 동기.
- 동시성: Mutex를 잡은 채 await 금지. 판단 시점에 `last_attempt_ms`를 먼저
  갱신해 두면(락 안에서) 60초 폴링 경합의 중복 fetch가 자연 차단된다.

**조립 (`usage/mod.rs` + `ipc/commands/usage.rs`)**

- 진입점 `load_usage_snapshot_with_live(live: &LiveUsageState, claude_root, codex_root, now_ms) -> UsageSnapshot`
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

**렌더러**: 변경 없음. `UsageSnapshot` 계약 그대로이고, 신선도·stale 흐림·
카운트다운은 기존 usageView 로직이 그대로 처리한다.

### 6.4 테스트

- `parse_access_token`: 정상/키 부재/깨진 JSON.
- `parse_live_response`: 실 API 응답 모양 픽스처(limits[] 포함, five_hour
  폴백, 빈 응답) — 기존 파서 재사용 확인.
- `should_fetch`: 최초/15분 경과/5분 하한/리셋 경계 조기/실패 직후 케이스.
- 조립: live 성공이 파일 캐시를 이기는 경우, 파일이 더 신선한 경우,
  live 실패 시 파일 폴백, plan_label 접목.
- 실 API smoke: `#[ignore]` 테스트 1개(토큰 있으면 실호출) — 사용자 수동.
- HTTP 호출부는 얇게 유지(파싱·판단이 전부 순수 함수라 네트워크 목 불필요).

---

## 7. 실시간 조회 실패 사유 표시 (2026-07-29)

### 7.1 문제

§6의 실시간 조회는 실패를 전부 조용히 삼키고 파일 캐시로 강등한다. 그런데
§2에서 확인했듯 그 파일 캐시는 `/usage`를 열지 않는 한 영원히 안 움직인다.
두 사실이 겹치면 사용자에게는 **"에이전트를 계속 쓰는데 사용량 숫자가 며칠째
그대로"**로 보이고, 화면에는 그 이유를 알 단서가 없다. 실제로 겪은 경로:
GUI 번들 앱의 Keychain 접근이 막힘 → `.credentials.json` 파일 토큰으로 폴백 →
그 토큰이 두 달 전 만료 → 401 → 조용한 폴백 → 11일 된 캐시 표시.

목표: **표시값의 신선도에 대한 책임 소재를 화면에 드러낸다.** 실패해도 폴백
동작 자체는 §6 그대로 유지한다(표시가 죽지 않는 게 우선).

### 7.2 계약 확장

`UsageSnapshot`에 진단 필드 `claudeLive: ClaudeLiveStatus`를 **항상** 싣는다
(실패해도 null이 아니다 — "아직 모름"은 `never_attempted`).

```ts
type LiveFetchOutcome =
  | "never_attempted" | "ok" | "no_credentials"
  | "unauthorized" | "http_error" | "network_error" | "unexpected_response";
type TokenSource = "keychain_scoped" | "keychain_legacy" | "file";
interface ClaudeLiveStatus {
  outcome: LiveFetchOutcome;
  tokenSource: TokenSource | null;   // 토큰을 못 읽었으면 null
  detail: string | null;             // "HTTP 401", "시간 초과" 등 고정 어휘
  lastAttemptMs: number | null;      // 스로틀에 막혀 건너뛴 폴링은 시도가 아님
  lastSuccessMs: number | null;
}
```

- **`tokenSource`를 싣는 이유**: `file` + `unauthorized` 조합이 위 실패 경로의
  지문이다. 이 구분이 없으면 "401"만 보이고 사용자는 재로그인을 시도하지만,
  실제 원인은 Keychain 접근 차단이라 재로그인으로 낫지 않는다.
- `detail`에는 **토큰·자격증명 문자열을 절대 넣지 않는다**(§6.2 유지).
  reqwest 오류 문자열도 URL이 섞여 나오므로 그대로 싣지 않고
  `is_timeout()`/`is_connect()`로 분류해 고정 어휘만 쓴다.
- 로컬 `expiresAt`으로 만료를 판정하지 않는 규칙은 그대로다(§6.2) — 서버 401이
  판정자이고, `tokenSource`는 그 401을 해석하는 힌트일 뿐이다.

백엔드: `LiveUsageState`가 `last_success`와 **별개로** 마지막 시도의
outcome/detail/token_source를 들고 있는다(성공 이력이 있어도 지금은 실패
중일 수 있고, 그 상태가 정확히 "값이 안 움직이는 이유"다). `status()`가
`last_success_ms`를 성공 스냅샷에서 유도해 중복 필드를 두지 않는다.

### 7.3 표시

- 순수 함수 `describeLiveStatus(status) -> {level, text, short} | null`
  (`usageView.ts`)가 사유를 한국어 문장으로 만든다. 실패 문구에는 **항상**
  "표시값은 로컬 캐시이며 이 캐시는 `/usage`를 열 때만 갱신된다"를 붙인다 —
  사유만으로는 왜 숫자가 안 움직이는지가 안 닫힌다.
- `UsageDialog`: Claude 블록 아래 진단 줄 + "마지막 시도/성공 N분 전".
  **진단 줄은 `.usage-stale`(opacity 0.5) 바깥에 둔다** — 값이 낡아 흐려진
  블록 안에 설명을 넣으면 정작 읽어야 할 문장이 같이 흐려진다(opacity는
  자식이 되돌릴 수 없다). 이를 위해 `.usage-provider-block` 래퍼를 둔다.
- `UsageWidget`: **글자를 늘리지 않는다.** BottomBar는 800px 기본 폭에서
  여유가 거의 없어(§3 BottomBar 800px) 경고 글리프 하나에도 상태 텍스트가
  잘린다. 대신 provider 라벨 색(warn/error)과 title 툴팁 꼬리표로만 알리고,
  라벨이 숨겨지는 좁은 폭(<900px)에서는 퍼센트에 점선 밑줄을 준다.
- `mergeUsageSnapshot`은 진단을 **병합하지 않고 항상 새 응답을 쓴다** — 값과
  달리 "지금 상태"라서, 이전 실패를 살려두면 이미 복구된 상태가 계속 실패로
  보인다.
- 상세 모달의 신선도 줄에서 Claude의 "실행 중에만 갱신됨" 문구는 **제거**했다
  (§2에서 거짓으로 판명). Codex는 rollout 파일 기준이라 그대로 유효하다.

### 7.4 테스트

- Rust: `status()` 초기값=`never_attempted`, 성공/실패 기록 후 필드 전이,
  실패가 `last_success`를 지우지 않음, `http_failure`의 401/403↔기타 분류.
- TS: `describeLiveStatus` 각 outcome의 level·문구(특히 401+file ↔ 401+keychain
  분기), 모든 실패 문구가 `/usage` 안내를 포함하는지, `formatLiveAttempts`
  (성공 이력 없음/미시도), 진단이 병합되지 않고 교체되는지.
- 계약: `usage-snapshot.json` 픽스처에 401+file 조합을 굳혀 양쪽에서 검증.

## 8. TLS 루트와 외부 프로세스 폴백 (2026-07-29)

### 8.1 문제

다른 환경에서 실시간 조회가 통째로 실패한다는 보고가 있었다. 원인은 TLS 루트
신뢰 범위였다: `reqwest`를 `default-features = false, features = ["rustls-tls"]`로
쓰고 있었는데, 이 조합은 **컴파일 시점에 박힌 webpki(Mozilla) 루트 번들만**
신뢰하고 OS 인증서 스토어를 보지 않는다. 사내 MITM 프록시나 self-signed 루트를
설치한 환경에서는 그 루트가 OS 스토어에만 있으므로,

> **앱만 TLS 핸드셰이크에 실패하고, 같은 머신의 `claude` CLI·`curl`은 멀쩡히 통한다**

는 비대칭이 생긴다. §7의 진단으로는 `network_error`("연결 실패")로 보인다.

### 8.2 결정 — 두 층으로 막는다

1. **근본**: reqwest features를 `rustls-tls-webpki-roots` + `rustls-tls-native-roots`로
   바꾸고 클라이언트 빌더에서 둘 다 명시적으로 켠다. 번들 루트와 OS 스토어 루트를
   모두 신뢰하므로, 사내 CA가 OS에 설치된 흔한 케이스는 이걸로 끝난다.
2. **보험**: 그래도 못 뚫는 잔여 케이스(클라이언트 인증서를 요구하는 프록시 등)를
   위해, 이미 그 환경에서 성공하고 있는 **남의 프로세스에 조회를 위임**한다.
   `src-tauri/src/usage/claude_live_fallback.rs`.

폴백 순서와 성질:

| 순서 | 수단 | 성질 |
| --- | --- | --- |
| 1 | `curl --config -` | OS 인증서 스토어·프록시 설정을 그대로 씀. 1차와 **같은 엔드포인트·헤더·토큰**이라 응답이 동일해 파서를 공유한다. 모델을 호출하지 않아 구조적으로 토큰 소모가 없다. |
| 2 | `claude -p /usage --output-format json` | **현재(claude 2.1.x) 아무 사용량도 돌려주지 않는다** — `-p` 모드에서 `/usage`는 인식되지 않고 `/cost`와 동일한 세션 요약만 나온다(`num_turns: 0`). CLI가 나중에 지원하면 코드 변경 없이 살아나도록 넣어둔 자리다. |

### 8.3 안전장치

- **과금 봉인**: 2번은 CLI 정책이 바뀌어 모델을 태우는 순간 과금이 된다.
  응답의 `usage`/`total_cost_usd`/`num_turns` 중 하나라도 0이 아니면
  (`detect_token_spend`) 조회 성패와 무관하게 **그 즉시 세션 내내 이 갈래를
  봉인**한다. 되돌리는 경로는 없다(앱 재시작 시 한 번 더 확인하고 다시 봉인될 뿐).
- **1시간 스로틀**: 폴백은 1차 조회가 실패한 폴링에서만, 그것도 1시간에 한 번만
  시도한다(`should_try_fallback`). 1차 시도의 5분 하한과 **별개 축**이라 폴백을
  썼다고 1차 폴링이 막히지 않는다. 자식 프로세스를 60초마다 띄우는 일도 없다.
- **토큰 노출 방지**: curl에 토큰을 명령줄 인자로 주면 같은 머신의 다른 사용자에게
  `ps`로 보인다. `--config -`로 **stdin에** 흘려 넣는다. config 값은 역슬래시·
  큰따옴표를 이스케이프하고 제어문자를 버린다(개행이 섞이면 config 한 줄이
  쪼개져 엉뚱한 옵션이 된다).
- **유령 세션 방지**: 앱이 `claude`를 자식으로 띄우면 사용자의 훅이 발화해 오피스
  씬에 존재하지 않는 세션이 그려질 수 있다. 훅 스크립트는 `ORCA_*` 환경변수가
  없으면 즉시 종료하므로, 자식 env에서 그 키들을 명시적으로 지운다.
- 토큰을 한 줄도 못 읽은 경우에도 2번은 시도한다 — CLI는 **자신의** 자격증명을
  쓰므로 우리 Keychain 접근 차단과 무관하게 돈다. 그래서 `record_success`의
  `token_source`는 Option이다.

### 8.4 계약 확장 — `via`

```ts
type FetchTransport = "direct" | "curl" | "claude_cli";
interface ClaudeLiveStatus { /* …§7.2… */ via: FetchTransport | null; }
```

`via`는 **마지막으로 값을 얻어낸 수단**이다(마지막으로 *시도한* 수단이 아니다).
실패는 이 값을 지우지 않는다 — "지금은 401로 실패 중이지만 화면의 그 값은 아까
curl 우회로 받아온 것"이라는 설명이 성립해야 하기 때문이다. 같은 이유로
폴백 체인이 끝까지 실패하면 **1차 실패의 outcome을 유지**하고(사용자가 고쳐야 할
대상은 "앱이 왜 직접 못 가져오는가"다), 갈래별 결과는 `detail`에 접두를 달아
이어 붙인다: `"연결 실패 · curl: HTTP 401 · claude: 사용량 미제공"`.

표시: `direct`가 아니면 성공 문구에도 우회 사실을 알린다("앱이 사용량을 curl
우회로 조회 중 — 앱에서 직접 거는 HTTPS가 이 환경에서 막혀 있습니다"). 값은
최신이므로 level은 `ok`를 유지한다 — 이건 고장이 아니라 **환경 진단**이다.

### 8.5 테스트

- Rust 순수 함수: curl config 조립(엔드포인트·헤더·이스케이프·개행 제거),
  출력 분리(본문↔상태코드, 마커 부재), `detect_token_spend`(현재 0턴 응답=None,
  토큰/비용/턴 각각 감지), `extract_cli_usage`(현재 요약 응답=None, 루트/
  `result` 객체/`result` JSON 문자열 3자리), 1시간 스로틀과 1차 스로틀의 독립성,
  CLI 봉인 상태 전이.
- 수동 스모크(`#[ignore]`, `-- --ignored live_fallback`): 실제 Keychain 토큰으로
  curl 폴백이 윈도를 파싱하는지, `claude` 갈래가 **실패하더라도 과금이 0인지**.
- TS: `via`별 문구(curl/claude_cli 우회 알림, direct·null은 우회 문구 없음),
  실패 중에도 `formatLiveAttempts`에 우회 꼬리표가 남는지.

## 9. Codex 사용량 실시간 조회 (kbm #2h8, 2026-08-23)

### 9.1 문제

§2의 Codex 소스는 rollout jsonl에 남은 `rate_limits` 스냅샷이다 — **CLI가 실제로
돌 때만** 갱신된다. Claude의 `.claude.json` 캐시 미러와 똑같은 문제(며칠 쉬면
숫자가 그날에 멈춰 있고, 리셋 경계를 지나도 100%로 남아 있다)를 가진다.

### 9.2 결정 — CLI에 물어본다 (토큰을 만지지 않는다)

Claude는 자격증명을 우리가 읽어 비공식 HTTP 엔드포인트를 직접 쳐야 했지만
(§6), Codex는 **CLI가 라이브 조회 RPC를 이미 노출한다**:

```
codex app-server --listen stdio://        # 줄바꿈 구분 JSON-RPC
→ initialize (clientInfo 필수) → initialized → account/rateLimits/read
```

토큰 읽기·갱신·계정 선택을 전부 codex가 처리하므로 Keychain 접근도, UA 위장도,
curl 우회 체인도 없다. 실측(codex-cli 0.149.0) 왕복 0.69초.

응답(요약):

```jsonc
{
  "rateLimits": {            // 기본 버킷 — rollout 스냅샷과 같은 의미
    "limitId": "codex", "planType": "prolite",
    "primary": {"usedPercent": 29, "windowDurationMins": 10080, "resetsAt": 1787998886},
    "secondary": null, "credits": {"hasCredits": false, "balance": "0"}
  },
  "rateLimitsByLimitId": {   // 모델별 버킷 — rollout에는 아예 없던 정보
    "codex_bengalfox": {"limitName": "GPT-5.3-Codex-Spark",
      "primary": {"usedPercent": 4, "windowDurationMins": 300, ...},
      "secondary": {"usedPercent": 7, "windowDurationMins": 10080, ...}}
  },
  "rateLimitResetCredits": {"availableCount": 1, "credits": [...]}
}
```

**세 줄을 한꺼번에 쏟아붓고 stdin을 닫는 방식은 못 쓴다** — 서버가 EOF를 보는
즉시 응답 없이 종료한다(실측). 응답을 읽어 가며 다음 요청을 쓴다.

`app-server`는 experimental 서브커맨드라 계약이 바뀔 수 있다. 그래서 실패든
형식 변화든 전부 `Err`로 눌러 담고, 조립 단계가 rollout 값으로 조용히 강등한다
(`merge_provider` — Claude와 같은 "fetched_at_ms 큰 쪽" 규칙).

### 9.3 구조

- `usage/codex_live.rs` — 자식 프로세스 stdio JSON-RPC(20초 상한, `kill_on_drop`
  + 명시적 `start_kill`), 순수 파서(`parse_rate_limits`), 스로틀 상태
  (`CodexLiveState`). Windows는 summarizer와 같은 PowerShell 경유 `codex` 해석을
  쓰되 stdin은 그대로 물려준다(대화형 왕복이라 `ReadToEnd` 불가).
- 스로틀 판단은 **Claude와 같은 함수**(`claude_live::should_fetch`)를 재사용한다:
  5분 하한 · 15분 정기 리프레시 · 리셋 경계를 지난 윈도가 있으면 조기 조회.
  자식 프로세스를 띄우는 경로라 "폴링마다 돌지 않는다"는 요구가 Claude보다 강하다.
- 상태 그릇은 `LiveUsageState`에 필드로 얹었다(`live.codex`) — 이 구조체가 이미
  네이티브 커맨드와 웹 RPC가 공유하는 "실시간 조회 메모리 상태"라, 별도 Arc를
  앱 전역에 다시 배선하지 않았다.
- 두 provider의 조회는 `tokio::join!`으로 동시에 돈다(폴링 1회의 지연이 합이
  되지 않게). 실패 모드가 겹치지 않아 한쪽이 다른 쪽을 막지 않는다.

### 9.4 계약 확장

```ts
type UsageWindowKind = "session" | "weekly" | "session_model" | "weekly_model" | "unknown";

type CodexLiveOutcome =
  | "never_attempted" | "ok"
  | "cli_missing" | "cli_failed" | "timeout" | "rpc_error" | "unexpected_response";

interface CodexLiveStatus {
  outcome: CodexLiveOutcome;
  detail: string | null;
  lastAttemptMs: number | null;
  lastSuccessMs: number | null;
}
interface UsageSnapshot { /* …기존… */ codexLive: CodexLiveStatus }
```

- `CodexLiveOutcome`은 Claude의 `LiveFetchOutcome`과 **일부러 분리했다**. 이쪽
  실패는 HTTP 상태코드가 아니라 "CLI가 없다/죽었다/모르는 응답을 줬다"의
  어휘다. 자격증명을 앱이 만지지 않으므로 `tokenSource`·`via`도 없다.
- `session_model`을 새로 뒀다. 모델별 버킷의 5시간 창이 `session`으로 오면
  뱃지의 "5시간" 자리(`badgeWindows`)를 모델 한도가 가로채 계정 전체 한도인
  것처럼 보인다 — 종류로 구분하고 모델명은 `label`에 싣는다.
- 기본 버킷과 `limitId`가 같은 `rateLimitsByLimitId` 항목은 중복이라 건너뛴다.
  순회 순서는 serde_json Map(BTreeMap) 키 정렬이라 결정적이다.

### 9.5 테스트

- Rust 순수 함수: 실측 응답 → 기본 버킷 + 모델 버킷 창 매핑(초→ms, 300/10080
  종류, 중복 limitId 제외), `limitName` 부재 시 limitId 폴백, 모르는 창 길이는
  `unknown` + 원본 분 유지, `planType: "unknown"` 무시, JSON-RPC error 분류,
  detail 절단, 스로틀·상태 전이(실패가 마지막 성공을 지우지 않는다).
- 수동 스모크(`#[ignore]`, `-- --ignored codex_live_smoke`): 실제 CLI 왕복.
- TS: 사유별 문구와 rollout 강등 안내, `formatLiveAttempts`가 `via` 없는 진단도
  받는지, `session_model` 라벨과 뱃지 5시간 자리 비침입.

## 10. 하루 넘게 못 가져온 provider 숨김 (kbm #2j4, 2026-08-25)

### 10.1 문제

§7이 "왜 낡았는지"를 말해 주게 되면서 실패는 설명되지만 **사라지지는** 않았다.
표시값이 며칠째 그대로인 provider도 흐린 숫자(`.usage-stale`, opacity 0.5)나
dim `—`로 계속 남는다. 하루가 지난 값은 "낡은 참값"이 아니라 사실상 무의미하다
— 5시간 창은 네 번 넘게, 주간 창도 리셋 경계를 지났을 수 있다. 흐린 숫자는
그래도 읽히고, 읽히면 오해를 부른다. 자리만 차지하는 `—`도 마찬가지다.

### 10.2 결정 — 임계 24시간, 뱃지와 모달 양쪽에서 제거

`usageView.isProviderGone(usage, live, now)`:

- 값이 있으면 신선도만 본다. `now - fetchedAtMs > DEAD_THRESHOLD_MS`(24시간)이면 뺀다.
- 값이 없으면 **한 번이라도 시도했는지**로 가른다. `never_attempted`(또는 진단
  필드 자체가 없는 구버전 응답)면 부팅 직후일 수 있으니 남기고, 시도했는데도
  값이 하나도 없으면(미설치·미로그인) 뺀다.

두 번째 갈래에 24시간을 적용하지 않는 이유: 실패 시작 시각을 앱 재시작 너머로
보존하지 않아 "며칠째"를 잴 기준이 없고, 애초에 보여줄 숫자가 없으므로 기다릴
이유도 없다.

기존 30분 stale(흐리게)은 그대로 둔다 — 둘은 층이 다르다. 30분은 "조금 낡음",
24시간은 "표시할 가치 없음".

`visibleUsageProviders(snapshot, now)`가 `USAGE_PROVIDERS` 고정 순서를 유지한 채
남은 provider만 돌려주고, `UsageWidget`(뱃지)과 `UsageDialog`(모달 블록)가 같은
목록을 쓴다. 셋 다 빠지면 위젯은 **버튼 자체를 렌더하지 않고**(빈 버튼은
BottomBar 폭만 먹는다), 모달은 안내 한 줄(`usage.dialog.allHidden`)만 남긴다.

뱃지 쪽 판정 시각은 별도 tick 없이 렌더 시각(`Date.now()`)이다 — 60초 폴링이
어차피 새 스냅샷 객체를 스토어에 넣어 다시 그려지고, 임계값이 24시간이라 60초
해상도로 충분하다. 모달은 이미 1초 tick이 돈다.

### 10.3 테스트

- TS 순수 함수: 24시간 경계 양옆, 30분 stale 경계와 무관함, `never_attempted`는
  남기고 시도 후 무데이터는 뺌, `visibleUsageProviders` 순서 유지.
- TSX(모달): 시도 후 무데이터 provider가 안내 문구째 사라짐, 25시간 낡은 값이
  흐리게가 아니라 아예 안 그려짐, 전부 빠지면 안내 한 줄만.

## 11. Antigravity provider (kbm #2j4, 2026-08-25)

### 11.1 왜 gemini CLI가 아니라 Antigravity인가

개인 계정의 Gemini Code Assist 무료 티어가 Antigravity로 이관되면서 gemini CLI의
OAuth 클라이언트가 자격을 잃었다(2026-08-25 실측): `loadCodeAssist`는
`IneligibleTierError: UNSUPPORTED_CLIENT`("migrate to the Antigravity suite"),
`POST cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota`는 403
`SUBSCRIPTION_REQUIRED`를 돌려준다. 조회할 한도 자체가 Antigravity로 옮겨왔으므로
provider도 하나만 둔다.

### 11.2 데이터 소스 — `agy -p /usage --output-format json`

**파일 캐시 미러가 없다.** Antigravity는 사용량 스냅샷을 로컬에 남기지 않아
(`~/.gemini/antigravity-cli/` 전수 확인) Claude·Codex처럼 "실패하면 파일로 강등"
하는 층이 없다. 실패는 곧 표시 없음이고, 그래서 §10의 숨김 규칙과 자연히 맞물린다.

조회 수단은 codex_live와 같은 결 — 자격증명을 앱이 만지지 않고 CLI에 물어본다.
`agy`의 print 모드는 슬래시 명령을 그대로 실행하고 구조화 결과를 준다:

```json
{"status":"SUCCESS","num_turns":0,"usage":{"total_tokens":0},
 "command":{"name":"usage","data":{"groups":[
   {"name":"Gemini Models","buckets":[
     {"id":"gemini-weekly","window":"weekly",
      "remaining_fraction":0.1064912,"reset_time":"2026-08-29T06:50:27Z"}]},
   {"name":"Claude and GPT models","buckets":[
     {"id":"3p-weekly","window":"weekly",
      "remaining_fraction":1,"reset_time":"2026-09-01T12:39:21Z"}]}]}}}
```

값 규약 둘:

1. `remaining_fraction`은 **잔여**다. `used_percent`는 그 여집합(`(1-f)*100`).
2. 모델 턴을 돌지 않는다(`num_turns: 0`, tokens 0) — 사용량을 보려고 사용량을
   쓰지 않는다. 다만 에이전트 백엔드 콜드 스타트로 **1회 8~10초**가 걸린다.
   codex_live보다도 스로틀이 절실하다(§6.1과 같은 5분 하한 / 15분 정기 판단인
   `claude_live::should_fetch`를 그대로 쓴다).

창 매핑: 모든 버킷이 모델 그룹 소속이라 종류는 항상 모델별 갈래
(`weekly` → `weekly_model`, `session`/`five_hour` → `session_model`, 그 외
`unknown`)이고 라벨은 그룹명이다. 그래야 라벨 없이는 뜻이 서지 않는 값임이
UI에 드러나고, 뱃지의 "5시간" 자리(§9.4와 같은 이유)를 가로채지 않는다.
`plan_label`은 없다 — 응답이 티어 이름을 주지 않는다.

### 11.3 계약 확장

```ts
type AntigravityLiveOutcome =
  | "never_attempted" | "ok"
  | "cli_missing" | "cli_failed" | "timeout"
  | "command_failed"          // CLI가 실패 status를 돌려줌(미로그인 등)
  | "unexpected_response";

interface AntigravityLiveStatus {
  outcome: AntigravityLiveOutcome;
  detail: string | null;
  lastAttemptMs: number | null;
  lastSuccessMs: number | null;
}
interface UsageSnapshot {
  /* …기존… */
  antigravity: ProviderUsage | null;   // 파일 캐시가 없어 live 성공 때만 채워진다
  antigravityLive: AntigravityLiveStatus;
}
```

Codex의 `CodexLiveOutcome`과 어휘가 겹치지만 분리했다 — 이쪽은 JSON-RPC가 아니라
print 모드 1회 실행이라 "RPC 오류"가 없고, 대신 CLI가 붙여 주는 `status` 필드가
실패 갈래를 가른다(`command_failed`). 진단 문구의 꼬리말도 다르다: 강등할 캐시가
없으므로 "표시값은 로컬 캐시…"가 아니라
`usage.antigravityLive.noCacheNote`("직전 조회 값이 남거나 표시 자체가 사라진다").

`Provider`는 `"claude" | "codex" | "antigravity"`, 뱃지 접두는 `AG`,
모달 표시명은 `Antigravity`.

### 11.4 실행 경로

`Command::new("agy")`(PATH)가 1차. `NotFound`일 때만 `~/.local/bin/agy`(=
`agy install` 기본 위치)를 한 번 더 본다 — 번들 앱의 최소 PATH가 로그인 셸
PATH로 보강되지 않은 환경(`session/env_capture.rs`)에서 그 한 걸음이 차이를
만든다. 다른 실패(권한·타임아웃)는 두 번 띄우지 않는다. cwd는 `temp_dir()`
(print 모드가 cwd를 프로젝트로 잡으려 든다), 상한은 45초이고 CLI에 주는
`--print-timeout`은 30초로 더 짧게 둬 CLI가 스스로 정리할 기회를 준다.
Windows는 `.cmd` 셰임 대비로 codex_live와 같은 PowerShell 경유 해석을 쓴다.

### 11.5 테스트

- Rust 순수 함수: 실측 출력 → 두 주간 창 매핑(잔여→사용 변환, 그룹명 라벨,
  ISO→ms), 배너 줄 섞임 무시, 비-SUCCESS status → `command_failed`,
  groups 부재·빈 버킷·비-JSON → `unexpected_response`, 모르는 창 종류는
  `unknown` + `window_minutes: null`, 스로틀, 실패가 마지막 성공을 안 지움.
- 수동 스모크(`#[ignore]`, `-- --ignored antigravity_live`): 실제 `agy` 왕복.
- TS: 사유별 문구 키와 detail 전달.
