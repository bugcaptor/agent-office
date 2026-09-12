# VS Code 표시 API 계약

상태: v1 구현. 이 문서는 Agent Office 본체와 VS Code 연결 확장 사이의 정본 계약이다.

## 연결

기존 로컬 control 서버를 사용한다. 서버는 `127.0.0.1`에만 열리고, 사용자가 CLI 제어를 켜고 승인해 발급한 토큰이 있어야 한다. 모든 호출은 `POST /v1/display/<route>`, `X-Agent-Office-Token` 헤더, JSON 본문을 사용한다. 응답은 항상 HTTP 200의 `{ ok: true, data }` 또는 `{ ok: false, error }`이며, 인증 실패만 HTTP 401이다.

| 경로 | 본문 | 성공 data |
| --- | --- | --- |
| `capabilities` | `{}` | `{ protocolVersion: 1, features: ["characters", "appearance", "portrait", "usage", "focus", "ide-session-focus"], generatorRevision: "office-gen-0617d45" }` |
| `characters` | `{}` | `{ characters: Character[] }` |
| `appearance` | `{ agentId, revision, kind?: "sprite" | "portrait" }` | `{ agentId, revision, pngBase64, mimeType: "image/png", frameCount }` |
| `usage` | `{}` | 기존 `UsageSnapshot` |
| `focus` | `{ agentId, intent?: "connectCodex" }` | `{ agentId, focused: true }` |

`Character`는 `{ agentId, name, role, cwd, seed, archetype, colors, portraitUpdatedAt, spriteUpdatedAt, revision }`이다. nullable 필드는 JSON `null`로 전송한다. `revision`은 `seed`, `archetype`, `colors`, `portraitUpdatedAt`, `spriteUpdatedAt`의 SHA-256 식별자이며, 확장은 `characters`를 다시 읽어 값이 바뀌면 외형 캐시를 폐기한다.

`appearance.kind`를 생략하거나 `"sprite"`로 주면 기존 4프레임 시트를 반환한다. `"portrait"`는 단일 PNG를 반환하고 `frameCount`는 1이다. 스프라이트가 없을 때만 `pngBase64`가 `null`이면 확장은 `generatorRevision`에 맞는 vendored 순수 생성기로 `seed`/`archetype`/`colors`에서 4프레임 시트를 생성한다. 커스텀 스프라이트는 1 MiB 이하, IHDR의 높이 16~256 및 폭=높이×4를 검증하며, 초상은 2 MiB 이하 및 IHDR의 폭·높이가 각각 1~4096임을 검증한다. data URL 접두사는 붙이지 않는다.

`appearance_revision_mismatch`, `unknown_agent`, `invalid_sprite`, `invalid_portrait`, `focus_unavailable`은 `ok:false`의 안정된 error 값이다. `focus`는 창을 복원하고 해당 프로필만 선택한다. `intent`를 생략하면 기존 프로필 편집기를 열며, `"connectCodex"`는 해당 캐릭터·작업 폴더·Codex 공급자를 미리 고른 IDE 연결 대화상자를 연다. 어느 요청도 세션·PTY·외부 IDE 프로세스를 만들거나 포커스를 되돌리지 않는다.

`usage`는 `load_usage_snapshot_body`를 그대로 공유하여 본체의 throttle, 캐시, 실패 상태를 보존한다. 확장은 누락·오래된 값에 잔여량을 추정하지 않는다.

## 연결 선택 자동화 (2026-09-12)

`connectCodex`로 여는 선택 창은 요청한 캐릭터를 매번 적용하며 이전 모달의 선택을 재사용하지 않는다. 현재 필터에 후보가 하나면 자동 선택하고, 여러 개면 사용자가 고른다. 캐릭터가 지정되지 않은 일반 진입에서는 선택한 대화의 폴더와 일치하는 유일 캐릭터를 선택한다.

캐릭터·폴더·공급자가 지정된 선택 창에 후보가 없으면 열린 동안 3초 간격으로 다시 조회한다. 닫기·설정 비활성화·필터 변경은 이전 조회 결과와 대기를 무효화한다. 실제 연결과 기존 세션 교체 확인은 유지한다. 후보는 과거 종료 기록일 수 있으므로 자동 선택을 현재 VS Code 창의 대화 식별 또는 자동 관찰 연결로 표시하지 않는다. 빈 New session과 기존 기록을 구별하거나 첫 대화 이전에 연결을 만드는 기능은 아니다.
