// src/shared/types/apiKeys.ts
//
// 외부 API 키(Anthropic·OpenRouter·OpenCode) 저장 상태 — Rust
// `key_store::ApiKeyStatus` 미러. **키 값은 이 경계를 넘지 않는다** — 존재
// 여부 bool만 내려온다.

export interface ApiKeyStatus {
  /** Anthropic 키를 (저장값이든 env든) 쓸 수 있는지. */
  anthropicSet: boolean;
  /** Anthropic 키가 저장값이 아니라 env 폴백인지(UI 안내용). */
  anthropicFromEnv: boolean;
  /** OpenRouter 키를 (저장값이든 env든) 쓸 수 있는지. */
  openrouterSet: boolean;
  /** OpenRouter 키가 저장값이 아니라 env 폴백인지(UI 안내용). */
  openrouterFromEnv: boolean;
  /** OpenCode 키(사용량 표시용)를 (저장값이든 env든) 쓸 수 있는지. */
  opencodeSet: boolean;
  /** OpenCode 키가 저장값이 아니라 env 폴백인지(UI 안내용). */
  opencodeFromEnv: boolean;
}
