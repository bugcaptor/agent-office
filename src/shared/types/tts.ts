// src/shared/types/tts.ts
//
// Domain slice: 알림 대사 TTS. AI 에이전트가 사용자 확인을 기다리거나
// (source="hook" → "question") 작업을 마쳤을 때(source="stop" → "done") 그
// 시스템 문구를 캐릭터 말투의 짧은 대사로 리라이트한 뒤 ElevenLabs로 합성해
// 캐릭터 목소리로 재생한다. bell(info) 알림은 발화하지 않는다.
//
// 보안 계약: **API 키는 이 경계를 넘지 않는다.** 합성은 전부 백엔드
// (`src-tauri/src/tts/`)에서 수행되고, 렌더러는 오디오 바이트(base64)만 받는다.
// 키 조회는 `TtsStatus`의 존재 여부 bool뿐이다.
//
// See src/shared/types.ts for the frozen-contract overview.

/**
 * 대사 리라이트 공급자 — Rust `TtsRewriteProvider` 미러.
 * - `auto`: 저장 API 키 → `ANTHROPIC_API_KEY` env → claude CLI → 리라이트 생략.
 * - `api`: Anthropic Messages API만(키 없으면 생략).
 * - `openrouter`: OpenRouter chat/completions만(키 없으면 생략). **명시 선택
 *   전용** — `auto` 체인은 이 경로를 고르지 않는다.
 * - `claude-cli`: `claude -p` 헤드리스 서브프로세스만. **구독 사용량을 소모한다.**
 * - `none`: 리라이트 없이 원문 문구를 그대로 읽는다.
 */
export type TtsRewriteProvider = "auto" | "api" | "openrouter" | "claude-cli" | "none";

/**
 * 무엇을 읽어주는 순간인가 — Rust `tts::rewrite::SpeakKind` 미러.
 * 리라이트 어조가 갈린다: `question`은 사용자 판단을 청하는 말,
 * `done`은 이미 끝난 일을 알리는 말(뿌듯/홀가분 계열 태그).
 * 부재 시 백엔드 기본값은 `question`이다.
 */
export type TtsSpeakKind = "question" | "done";

/** `tts_speak` 입력 — Rust `tts::SpeakRequest` 미러. 캐릭터 정보는 스토어에서 온다. */
export interface TtsSpeakRequest {
  agentId: string;
  /** 캐릭터 이름(말투 힌트). 빈 문자열 허용. */
  agentName: string;
  /** 캐릭터 아키타입(종족) id. **보이스 자동 캐스팅 전용** — 선호 라벨
   * (성별·연령)이 여기서 갈린다. 리라이트 프롬프트에는 실리지 않는다
   * (말투의 근거는 `personality`뿐). */
  archetype?: string;
  /** 캐릭터 성격(프로필 `personalityPrompt`). 리라이트가 참고하는 유일한 말투
   * 근거다 — 부재/빈 값이면 담백한 평상어로 발화한다. */
  personality?: string;
  /** 스프라이트 시드 — 보이스 결정적 배정 키. 비면 agentId로 폴백. */
  seed: string;
  /** 원문 알림 문구. */
  message: string;
  /** 상황. 부재 = "question"(구버전 호환). */
  kind?: TtsSpeakKind;
  /** 프로필에서 수동 지정한 voiceId. 비면 archetype 기반 자동 캐스팅.
   * 계정 목록에 없는 id면 백엔드가 조용히 자동 배정으로 강등한다. */
  voiceId?: string;
  /** 발화 시점 그 에이전트의 작업 맥락 한 줄(머리 위 라벨 파생 텍스트 등).
   * 리라이트 프롬프트에 참고용으로만 실린다 — 없어도 기존 동작과 같다. */
  context?: string;
}

/** `tts_list_voices` 항목 — Rust `tts::VoiceOption` 미러.
 * 프로필 다이얼로그의 보이스 드롭다운이 쓴다. **키 값은 실리지 않는다.** */
export interface TtsVoiceOption {
  voiceId: string;
  name: string;
  /** 사람이 읽는 라벨 요약(예: "female · young · american"). 없으면 빈 문자열. */
  labels: string;
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
  /** OpenRouter 키를 쓸 수 있는지. 공급자로 openrouter를 고른 경우에만 쓰인다. */
  openrouterSet: boolean;
  openrouterFromEnv: boolean;
  /** PATH에 `claude`가 있는지. */
  claudeCliAvailable: boolean;
  /** 현재 설정으로 실제 선택될 리라이트 경로. */
  effectiveRewriteVia: TtsRewriteProvider;
}

/** `tts_set_keys` 입력. `undefined`인 필드는 기존 값 유지, `""`는 삭제. */
export interface TtsSetKeysRequest {
  elevenlabs?: string;
  anthropic?: string;
  openrouter?: string;
}
