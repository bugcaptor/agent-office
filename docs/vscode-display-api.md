# VS Code 표시 API 계약

상태: v1 구현. 이 문서는 Agent Office 본체와 VS Code 연결 확장 사이의 정본 계약이다.

## 연결

기존 로컬 control 서버를 사용한다. 서버는 `127.0.0.1`에만 열리고, 사용자가 CLI 제어를 켜고 승인해 발급한 토큰이 있어야 한다. 모든 호출은 `POST /v1/display/<route>`, `X-Agent-Office-Token` 헤더, JSON 본문을 사용한다. 응답은 항상 HTTP 200의 `{ ok: true, data }` 또는 `{ ok: false, error }`이며, 인증 실패만 HTTP 401이다.

| 경로 | 본문 | 성공 data |
| --- | --- | --- |
| `capabilities` | `{}` | `{ protocolVersion: 1, features: ["characters", "appearance", "usage", "focus"], generatorRevision: "office-gen-0617d45" }` |
| `characters` | `{}` | `{ characters: Character[] }` |
| `appearance` | `{ agentId, revision }` | `{ agentId, revision, pngBase64, mimeType: "image/png", frameCount: 4 }` |
| `usage` | `{}` | 기존 `UsageSnapshot` |
| `focus` | `{ agentId }` | `{ agentId, focused: true }` |

`Character`는 `{ agentId, name, role, cwd, seed, archetype, colors, spriteUpdatedAt, revision }`이다. nullable 필드는 JSON `null`로 전송한다. `revision`은 `seed`, `archetype`, `colors`, `spriteUpdatedAt`의 SHA-256 식별자이며, 확장은 `characters`를 다시 읽어 값이 바뀌면 외형 캐시를 폐기한다.

`appearance`의 `pngBase64`가 `null`이면 확장은 `generatorRevision`에 맞는 vendored 순수 생성기로 `seed`/`archetype`/`colors`에서 4프레임 시트를 생성한다. 값이 있으면 본체가 1 MiB 이하 PNG, IHDR의 높이 16~256, 폭=높이×4를 검증한 커스텀 시트다. data URL 접두사는 붙이지 않는다.

`appearance_revision_mismatch`, `unknown_agent`, `invalid_sprite`, `focus_unavailable`은 `ok:false`의 안정된 error 값이다. `focus`는 창을 복원하고 해당 프로필만 선택한다. 세션·PTY·외부 IDE 프로세스를 만들거나 포커스를 되돌리지 않는다.

`usage`는 `load_usage_snapshot_body`를 그대로 공유하여 본체의 throttle, 캐시, 실패 상태를 보존한다. 확장은 누락·오래된 값에 잔여량을 추정하지 않는다.
