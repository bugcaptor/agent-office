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
import type { ColorOverrides } from './profile';

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
  seed: string;
  archetype?: string;
  /** 팔레트 슬롯별 색 오버라이드. 외형의 일부이므로 캐릭터와 함께 옮긴다. */
  colors?: ColorOverrides;
  /** 초상화 추가 프롬프트. */
  portraitRequest?: string;
  /** 스프라이트 추가 프롬프트. */
  spriteRequest?: string;
  /** 미니미(소환수) 추가 프롬프트. 부재 = 자동 위임 문구 사용. */
  minimiRequest?: string;
  personalityPrompt?: string;
  keyboardSound?: string;
  /** **레거시 전용**(옛 번들의 "메모"). 새로 내보내지 않는다 — 가져올 때
   * `personalityPrompt`에 합치는 입력으로만 읽는다. */
  legacyNote?: string;
  /** **레거시 전용**(옛 번들의 "외모 힌트"). 가져올 때 초상화/스프라이트 추가
   * 프롬프트가 비어 있으면 양쪽에 복사하는 입력으로만 읽는다. */
  legacyAppearance?: string;
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

/**
 * 파싱 실패 사유. **문구가 아니라 안정적인 코드**다 — `src/shared`는 renderer에
 * 의존할 수 없어 여기서 `t()`를 부를 수 없고, 부를 수 있더라도 이 결과는 그리는
 * 쪽(ProfileDialog)이 자기 언어로 옮겨야 한다. 코드↔문구 매핑은 renderer의
 * `common:errors.*` 카탈로그에 있다(SettingsDialog의 백엔드 오류 코드 매핑과 같은 관례).
 */
export type CharacterBundleError =
  | "bundle-not-json"
  | "bundle-not-character-file"
  | "bundle-schema-version-missing"
  | "bundle-schema-version-newer"
  | "bundle-schema-version-unsupported"
  | "bundle-profile-missing"
  | "bundle-profile-name-missing"
  | "bundle-portrait-invalid"
  | "bundle-portrait-too-large"
  | "bundle-sprite-invalid"
  | "bundle-sprite-too-large"
  | "bundle-minimi-invalid"
  | "bundle-minimi-too-large";

export type ParseBundleResult =
  | { ok: true; bundle: CharacterBundle }
  | { ok: false; error: CharacterBundleError };

/** base64 문자열의 디코드 후 대략 바이트 수(패딩 무시한 상한 근사). */
function approxDecodedBytes(b64: string): number {
  return Math.floor((b64.length * 3) / 4);
}

/** `unknown` → 문자열이면 그대로, 아니면 undefined. */
function asString(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

/** `unknown` → 색 오버라이드. 슬롯 3종의 문자열 값만 취하고, 하나도 없으면
 *  undefined(키 자체를 넣지 않는다). hex 형식 검증은 렌더러 팔레트 쪽 담당. */
function asColors(v: unknown): ColorOverrides | undefined {
  if (!v || typeof v !== "object") return undefined;
  const o = v as Record<string, unknown>;
  const out: ColorOverrides = {};
  for (const slot of ["skin", "hair", "shirt"] as const) {
    const c = asString(o[slot]);
    if (c !== undefined) out[slot] = c;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

const fail = (error: CharacterBundleError): ParseBundleResult => ({ ok: false, error });

/**
 * 번들 파일 텍스트를 파싱·검증한다. 성공 시 정규화된 `CharacterBundle`,
 * 실패 시 `CharacterBundleError` 코드를 돌려준다(문구는 호출부가 번역한다).
 * 어떤 경우에도 예외를 던지지 않는다(호출부가 결과만 분기).
 */
export function parseCharacterBundle(text: string): ParseBundleResult {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return fail("bundle-not-json");
  }
  if (!raw || typeof raw !== "object") {
    return fail("bundle-not-character-file");
  }
  const o = raw as Record<string, unknown>;
  if (o.kind !== CHARACTER_BUNDLE_KIND) {
    return fail("bundle-not-character-file");
  }
  if (typeof o.schemaVersion !== "number") {
    return fail("bundle-schema-version-missing");
  }
  if (o.schemaVersion > CHARACTER_BUNDLE_SCHEMA_VERSION) {
    return fail("bundle-schema-version-newer");
  }
  if (o.schemaVersion !== CHARACTER_BUNDLE_SCHEMA_VERSION) {
    return fail("bundle-schema-version-unsupported");
  }

  const p = o.profile;
  if (!p || typeof p !== "object") {
    return fail("bundle-profile-missing");
  }
  const pr = p as Record<string, unknown>;
  if (typeof pr.name !== "string" || pr.name.trim() === "") {
    return fail("bundle-profile-name-missing");
  }

  // 임베드 이미지 검증 — 상한 초과면 이미지만 건너뛰지 않고 전체 거부(사용자 결정).
  const portrait = o.portraitPngBase64;
  if (portrait !== undefined) {
    if (typeof portrait !== "string") {
      return fail("bundle-portrait-invalid");
    }
    if (approxDecodedBytes(portrait) > MAX_PORTRAIT_BYTES) {
      return fail("bundle-portrait-too-large");
    }
  }
  const sprite = o.spritePngBase64;
  if (sprite !== undefined) {
    if (typeof sprite !== "string") {
      return fail("bundle-sprite-invalid");
    }
    if (approxDecodedBytes(sprite) > MAX_SPRITE_BYTES) {
      return fail("bundle-sprite-too-large");
    }
  }
  const minimi = o.minimiPngBase64;
  if (minimi !== undefined) {
    if (typeof minimi !== "string") {
      return fail("bundle-minimi-invalid");
    }
    if (approxDecodedBytes(minimi) > MAX_MINIMI_BYTES) {
      return fail("bundle-minimi-too-large");
    }
  }

  const profile: PortableProfile = {
    name: pr.name,
    role: asString(pr.role) ?? "",
    seed: asString(pr.seed) ?? "",
    archetype: asString(pr.archetype),
    colors: asColors(pr.colors),
    portraitRequest: asString(pr.portraitRequest),
    spriteRequest: asString(pr.spriteRequest),
    minimiRequest: asString(pr.minimiRequest),
    personalityPrompt: asString(pr.personalityPrompt),
    keyboardSound: asString(pr.keyboardSound),
    // 옛 번들의 키는 레거시 슬롯으로 받아 둔다(가져오기에서 통합·복사한다).
    legacyNote: asString(pr.note),
    legacyAppearance: asString(pr.appearance),
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
