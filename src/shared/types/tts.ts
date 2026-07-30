// src/shared/types/tts.ts
//
// Domain slice: 확인 요청 대사 TTS. AI 에이전트가 사용자 확인을 기다릴 때
// (알림 source="hook" → notificationType "question") 그 시스템 문구를 캐릭터
// 말투의 짧은 대사로 리라이트한 뒤 ElevenLabs로 합성해 캐릭터 목소리로
// 재생한다. stop/bell 알림은 발화하지 않는다.
//
// 보안 계약: **API 키는 이 경계를 넘지 않는다.** 합성은 전부 백엔드
// (`src-tauri/src/tts/`)에서 수행되고, 렌더러는 오디오 바이트(base64)만 받는다.
// 키 조회는 `TtsStatus`의 존재 여부 bool뿐이다.
//
// See src/shared/types.ts for the frozen-contract overview.

/** 대사 리라이트에 쓸 Anthropic 모델 — Rust `TtsRewriteModel` 미러.
 * 값이 곧 Messages API / `claude -p --model`의 모델 id다. */
export type TtsRewriteModel = "claude-haiku-4-5" | "claude-sonnet-5" | "claude-opus-5";

/**
 * 대사 리라이트 공급자 — Rust `TtsRewriteProvider` 미러.
 * - `auto`: 저장 API 키 → `ANTHROPIC_API_KEY` env → claude CLI → 리라이트 생략.
 * - `api`: Anthropic Messages API만(키 없으면 생략).
 * - `claude-cli`: `claude -p` 헤드리스 서브프로세스만. **구독 사용량을 소모한다.**
 * - `none`: 리라이트 없이 원문 문구를 그대로 읽는다.
 */
export type TtsRewriteProvider = "auto" | "api" | "claude-cli" | "none";

/** `tts_speak` 입력 — Rust `tts::SpeakRequest` 미러. 캐릭터 정보는 스토어에서 온다. */
export interface TtsSpeakRequest {
  agentId: string;
  /** 캐릭터 이름(말투 힌트). 빈 문자열 허용. */
  agentName: string;
  /** 캐릭터 아키타입 id. 부재/"auto"는 백엔드가 "human"으로 취급. */
  archetype?: string;
  /** 스프라이트 시드 — 보이스 결정적 배정 키. 비면 agentId로 폴백. */
  seed: string;
  /** 원문 알림 문구. */
  message: string;
}

/** `tts_speak` 결과 — Rust `tts::SpeakResult` 미러. */
export interface TtsSpeakResult {
  /** mp3 바이트의 base64(`data:` 접두사 없음). CSP를 건드리지 않도록 media URL이
   * 아니라 `AudioContext.decodeAudioData`로 디코드해서 재생한다. */
  audioBase64: string;
  /** 항상 "audio/mpeg". */
  mimeType: string;
  /** 실제 합성된 텍스트. 리라이트가 강등됐으면 원문과 같고, v2 폴백이면
   * 오디오 태그가 제거된 상태다. */
  line: string;
  /** 배정된 ElevenLabs voice_id(디버그용). */
  voiceId: string;
  /** 사용한 ElevenLabs model_id(`eleven_v3` 또는 `eleven_multilingual_v2`). */
  modelId: string;
  /** 디스크 캐시 히트였는지(외부 API 호출 없음). */
  cached: boolean;
  /** 대사가 LLM 리라이트를 거쳤는지. */
  rewritten: boolean;
  /** 실제로 리라이트를 수행한 경로. 강등됐으면 "none". */
  rewriteVia: TtsRewriteProvider;
}

/**
 * `tts_key_status` / `tts_set_keys` 응답 — Rust `tts::TtsStatus` 미러
 * (`keys` 필드는 `#[serde(flatten)]`이라 평평하게 온다).
 * **키 값은 절대 포함되지 않는다** — 존재 여부만.
 */
export interface TtsStatus {
  /** ElevenLabs 키를 (저장값이든 env든) 쓸 수 있는지. 없으면 발화 불가. */
  elevenlabsSet: boolean;
  /** Anthropic 키를 쓸 수 있는지. 없으면 리라이트를 CLI로 넘기거나 생략한다. */
  anthropicSet: boolean;
  /** 그 키가 저장값이 아니라 env 폴백인지(UI 안내용). */
  elevenlabsFromEnv: boolean;
  anthropicFromEnv: boolean;
  /** PATH에 `claude`가 있는지. */
  claudeCliAvailable: boolean;
  /** 현재 설정으로 실제 선택될 리라이트 경로. */
  effectiveRewriteVia: TtsRewriteProvider;
}

/** `tts_set_keys` 입력. `undefined`인 필드는 기존 값 유지, `""`는 삭제. */
export interface TtsSetKeysRequest {
  elevenlabs?: string;
  anthropic?: string;
}
