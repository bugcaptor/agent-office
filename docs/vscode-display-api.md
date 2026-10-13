# VS Code 표시 API 계약

상태: 정본 (2026-10-14). 이 문서는 본체에 남은 표시 API의 정본 계약이다. `vscode-ext/` 로그 뷰어 패키지는 제거됨(2026-10-14, [archive/vscode-character-extension-plan.md](archive/vscode-character-extension-plan.md)); 이 API는 설치된 별도 companion 확장과의 호환을 위해 유지한다. 구현은 `src-tauri/src/control/display.rs`의 `capabilities`·`characters`·`appearance`·`usage`·`focus`다.

## 연결

기존 로컬 control 서버를 사용한다. 서버는 `127.0.0.1`에만 열리고, 사용자가 CLI 제어를 켜고 승인해 발급한 토큰이 있어야 한다. 모든 호출은 `POST /v1/display/<route>`, `X-Agent-Office-Token` 헤더, JSON 본문을 사용한다. 응답은 항상 HTTP 200의 `{ ok: true, data }` 또는 `{ ok: false, error }`이며, 인증 실패만 HTTP 401이다.

| 경로 | 본문 | 성공 data |
| --- | --- | --- |
| `capabilities` | `{}` | `{ protocolVersion: 1, features: ["characters", "appearance", "portrait", "usage", "focus"], generatorRevision: "office-gen-0617d45" }` |
| `characters` | `{}` | `{ characters: Character[] }` |
| `appearance` | `{ agentId, revision, kind?: "sprite" | "portrait" }` | `{ agentId, revision, pngBase64, mimeType: "image/png", frameCount }` |
| `usage` | `{}` | 기존 `UsageSnapshot` |
| `focus` | `{ agentId }` | `{ agentId, focused: true }` |

`Character`는 `{ agentId, name, role, cwd, seed, archetype, colors, portraitUpdatedAt, spriteUpdatedAt, revision }`이다. nullable 필드는 JSON `null`로 전송한다. `revision`은 `seed`, `archetype`, `colors`, `portraitUpdatedAt`, `spriteUpdatedAt`의 SHA-256 식별자이며, 확장은 `characters`를 다시 읽어 값이 바뀌면 외형 캐시를 폐기한다.

`appearance.kind`를 생략하거나 `"sprite"`로 주면 기존 4프레임 시트를 반환한다. `"portrait"`는 단일 PNG를 반환하고 `frameCount`는 1이다. 스프라이트가 없을 때만 `pngBase64`가 `null`이면 확장은 `generatorRevision`에 맞는 vendored 순수 생성기로 `seed`/`archetype`/`colors`에서 4프레임 시트를 생성한다. 커스텀 스프라이트는 1 MiB 이하, IHDR의 높이 16~256 및 폭=높이×4를 검증하며, 초상은 2 MiB 이하 및 IHDR의 폭·높이가 각각 1~4096임을 검증한다. data URL 접두사는 붙이지 않는다.

`appearance_revision_mismatch`, `unknown_agent`, `invalid_sprite`, `invalid_portrait`, `focus_unavailable`은 `ok:false`의 안정된 error 값이다. `focus`는 창을 복원하고 해당 프로필 편집기를 연다. 세션·PTY·외부 IDE 프로세스를 만들거나 포커스를 되돌리지 않는다. 예전의 `intent: "connectCodex"`(IDE 연결 대화상자)는 IDE 세션 관찰 제거(2026-10-14)로 없어졌고, 그 필드를 보내도 무시하고 프로필 편집기를 연다. `ide-session-focus` 기능 표지도 capabilities에서 빠졌다.

`usage`는 `load_usage_snapshot_body`를 그대로 공유하여 본체의 throttle, 캐시, 실패 상태를 보존한다. 확장은 누락·오래된 값에 잔여량을 추정하지 않는다.
