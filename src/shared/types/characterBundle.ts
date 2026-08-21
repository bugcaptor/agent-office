// src/shared/types/characterBundle.ts
//
// 캐릭터 내보내기/가져오기 번들(이슈 #77). 캐릭터 하나를 자기완결형 파일 한 개로
// 옮길 수 있게, 이식 가능한 프로필 필드 + 초상/스프라이트 PNG(base64 임베드)를
// 담는다. "로컬 환경에 관련된 것"(cwd·shell·startupCommand·bot 설정)은 담지
// 않는다 — 캐릭터 정체성과 외형만 이식한다.
//
// 파싱/검증은 순수 함수 `parseCharacterBundle`로 분리해 renderer/back-end 어디에도
// 의존하지 않게 한다(vitest로 직접 검증). 외부 입력이므로 신뢰 경계에서:
// kind/schemaVersion을 확인하고, 임베드 이미지가 저장소 상한을 넘으면 이미지만
// 건너뛰는 게 아니라 **가져오기 전체를 거부**한다(사용자 결정).

/** 번들 파일 판별자. profiles.json 등 다른 JSON과 헷갈리지 않게 고정 문자열. */
export const CHARACTER_BUNDLE_KIND = "agent-office.character" as const;
/** 현재 스키마 버전. 미래 버전 파일은 명확히 거부한다. */
export const CHARACTER_BUNDLE_SCHEMA_VERSION = 1 as const;

/** 초상 PNG 상한(2 MiB) — Rust `png_store::MAX_PORTRAIT_BYTES` 미러. */
export const MAX_PORTRAIT_BYTES = 2 * 1024 * 1024;
/** 스프라이트 시트 PNG 상한(1 MiB) — Rust `png_store::MAX_SPRITE_BYTES` 미러. */
export const MAX_SPRITE_BYTES = 1024 * 1024;
/** 미니미 PNG 상한(1 MiB) — Rust `png_store::MAX_MINIMI_BYTES` 미러. */
export const MAX_MINIMI_BYTES = 1024 * 1024;

/**
 * `AgentProfile`에서 이식 가능한(=캐릭터 정체성·외형) 필드만 추린 부분집합.
 * 제외: id/createdAt/deskIndex/assignedDeskIndex/clockedOut/portraitUpdatedAt/
 * spriteUpdatedAt/minimiUpdatedAt(가져오기 시 이미지 유무로 재설정), 그리고 로컬 환경 종속값
 * cwd/shell/startupCommand/bot(사용자 결정: 캐릭터만 내보낸다).
 */
export interface PortableProfile {
  name: string;
  role: string;
  note: string;
  seed: string;
  archetype?: string;
  appearance?: string;
  spriteRequest?: string;
  personalityPrompt?: string;
  keyboardSound?: string;
}

/** 자기완결형 캐릭터 번들. 이미지는 헤더 없는 base64 PNG로 임베드. */
export interface CharacterBundle {
  kind: typeof CHARACTER_BUNDLE_KIND;
  schemaVersion: typeof CHARACTER_BUNDLE_SCHEMA_VERSION;
  profile: PortableProfile;
  /** 초상 PNG(base64, data: prefix 없음). 부재 = 초상 없음. */
  portraitPngBase64?: string;
  /** 커스텀 스프라이트 시트 PNG(base64, data: prefix 없음). 부재 = 절차 생성. */
  spritePngBase64?: string;
  /**
   * 서브에이전트 미니미 픽셀아트 PNG(base64, data: prefix 없음, 단일 N×N).
   * 부재 = 미니미 커스텀 없음(부모 스프라이트 축소판 사용). **스키마 v1에
   * 추가된 선택 필드** — 이 키가 없는 기존 v1 번들도 그대로 읽히므로 버전을
   * 올리지 않는다(하위호환).
   */
  minimiPngBase64?: string;
}

export type ParseBundleResult =
  | { ok: true; bundle: CharacterBundle }
  | { ok: false; error: string };

/** base64 문자열의 디코드 후 대략 바이트 수(패딩 무시한 상한 근사). */
function approxDecodedBytes(b64: string): number {
  return Math.floor((b64.length * 3) / 4);
}

/** `unknown` → 문자열이면 그대로, 아니면 undefined. */
function asString(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

const fail = (error: string): ParseBundleResult => ({ ok: false, error });

/**
 * 번들 파일 텍스트를 파싱·검증한다. 성공 시 정규화된 `CharacterBundle`,
 * 실패 시 사용자에게 보일 한국어 오류 문자열을 돌려준다. 어떤 경우에도 예외를
 * 던지지 않는다(호출부가 결과만 분기).
 */
export function parseCharacterBundle(text: string): ParseBundleResult {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return fail("파일을 JSON으로 읽을 수 없습니다.");
  }
  if (!raw || typeof raw !== "object") {
    return fail("Agent Office 캐릭터 파일이 아닙니다.");
  }
  const o = raw as Record<string, unknown>;
  if (o.kind !== CHARACTER_BUNDLE_KIND) {
    return fail("Agent Office 캐릭터 파일이 아닙니다.");
  }
  if (typeof o.schemaVersion !== "number") {
    return fail("스키마 버전 정보가 없습니다.");
  }
  if (o.schemaVersion > CHARACTER_BUNDLE_SCHEMA_VERSION) {
    return fail("더 최신 버전에서 만든 파일입니다. 앱을 업데이트한 뒤 다시 시도하세요.");
  }
  if (o.schemaVersion !== CHARACTER_BUNDLE_SCHEMA_VERSION) {
    return fail("지원하지 않는 스키마 버전입니다.");
  }

  const p = o.profile;
  if (!p || typeof p !== "object") {
    return fail("프로필 데이터가 없습니다.");
  }
  const pr = p as Record<string, unknown>;
  if (typeof pr.name !== "string" || pr.name.trim() === "") {
    return fail("프로필에 이름이 없습니다.");
  }

  // 임베드 이미지 검증 — 상한 초과면 이미지만 건너뛰지 않고 전체 거부(사용자 결정).
  const portrait = o.portraitPngBase64;
  if (portrait !== undefined) {
    if (typeof portrait !== "string") {
      return fail("초상 이미지 데이터가 올바르지 않습니다.");
    }
    if (approxDecodedBytes(portrait) > MAX_PORTRAIT_BYTES) {
      return fail("초상 이미지가 너무 큽니다(2 MiB 초과). 가져오기를 취소합니다.");
    }
  }
  const sprite = o.spritePngBase64;
  if (sprite !== undefined) {
    if (typeof sprite !== "string") {
      return fail("스프라이트 이미지 데이터가 올바르지 않습니다.");
    }
    if (approxDecodedBytes(sprite) > MAX_SPRITE_BYTES) {
      return fail("스프라이트 이미지가 너무 큽니다(1 MiB 초과). 가져오기를 취소합니다.");
    }
  }
  const minimi = o.minimiPngBase64;
  if (minimi !== undefined) {
    if (typeof minimi !== "string") {
      return fail("미니미 이미지 데이터가 올바르지 않습니다.");
    }
    if (approxDecodedBytes(minimi) > MAX_MINIMI_BYTES) {
      return fail("미니미 이미지가 너무 큽니다(1 MiB 초과). 가져오기를 취소합니다.");
    }
  }

  const profile: PortableProfile = {
    name: pr.name,
    role: asString(pr.role) ?? "",
    note: asString(pr.note) ?? "",
    seed: asString(pr.seed) ?? "",
    archetype: asString(pr.archetype),
    appearance: asString(pr.appearance),
    spriteRequest: asString(pr.spriteRequest),
    personalityPrompt: asString(pr.personalityPrompt),
    keyboardSound: asString(pr.keyboardSound),
  };
  return {
    ok: true,
    bundle: {
      kind: CHARACTER_BUNDLE_KIND,
      schemaVersion: CHARACTER_BUNDLE_SCHEMA_VERSION,
      profile,
      portraitPngBase64: asString(portrait),
      spritePngBase64: asString(sprite),
      minimiPngBase64: asString(minimi),
    },
  };
}
