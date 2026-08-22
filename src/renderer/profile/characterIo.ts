// src/renderer/profile/characterIo.ts
//
// 캐릭터 내보내기/가져오기(이슈 #77)의 순수 변환 계층. 편집 다이얼로그의 `draft`
// 상태 ↔ 이식 가능한 `PortableProfile` ↔ 직렬화 번들 텍스트를 오간다. 파일 I/O와
// 이미지 저장·표시 같은 부수효과는 호출부(ProfileDialog)가 tauriApi/스토어로
// 수행하고, 여기서는 값 변환만 담당한다(vitest로 직접 검증).
//
// "로컬 환경에 관련된 것"(cwd·shell·startupCommand·bot 설정)은 내보내지 않고,
// 가져올 때도 건드리지 않는다(현재 draft 값 유지) — 캐릭터 정체성·외형만 이식한다.

import {
  CHARACTER_BUNDLE_KIND,
  CHARACTER_BUNDLE_SCHEMA_VERSION,
  type CharacterBundle,
  type PortableProfile,
} from "@shared/types";
import { mergeLegacyNote, normalizeColors, type DraftProfile } from "./generate";

/** 빈 문자열/공백은 undefined로(선택 필드 생략). */
function optionalTrim(v: string | undefined): string | undefined {
  const t = (v ?? "").trim();
  return t || undefined;
}

/** 현재 편집 draft에서 이식 가능한 프로필만 추린다(로컬 환경 필드 제외).
 *  archetype이 "auto"면 시드 추첨 결과가 아니라 "자동" 의도를 보존하기 위해 생략. */
export function portableFromDraft(d: DraftProfile): PortableProfile {
  return {
    name: d.name.trim(),
    role: d.role.trim(),
    seed: d.seed,
    archetype: d.archetype && d.archetype !== "auto" ? d.archetype : undefined,
    colors: normalizeColors(d.colors),
    portraitRequest: optionalTrim(d.portraitRequest),
    spriteRequest: optionalTrim(d.spriteRequest),
    minimiRequest: optionalTrim(d.minimiRequest),
    personalityPrompt: optionalTrim(d.personalityPrompt),
    keyboardSound: optionalTrim(d.keyboardSound),
  };
}

/** 가져온 프로필을 현재 draft에 병합한다. 이식 필드는 덮어쓰고, 로컬 환경
 *  필드(cwd/shell/startupCommand/bot*)는 현재 draft 값을 그대로 유지한다.
 *  빈 이름/시드는 폴백해 편집기 무결성을 지킨다. */
export function applyBundleToDraft(d: DraftProfile, p: PortableProfile): DraftProfile {
  return {
    ...d,
    name: p.name.trim() || d.name,
    role: p.role ?? "",
    seed: p.seed || d.seed,
    archetype: p.archetype ?? "auto",
    // 색 오버라이드는 통째로 교체한다 — 번들에 없으면 시드 기본색으로 돌아간다.
    colors: normalizeColors(p.colors) ?? {},
    // 옛 번들의 "외모 힌트"는 초상/스프라이트 양쪽에 쓰이던 값이라, 비어 있는
    // 칸에만 복사해 그림 결과가 달라지지 않게 한다(백엔드 migrate_loaded와 같은 규칙).
    portraitRequest: p.portraitRequest ?? p.legacyAppearance ?? "",
    spriteRequest: p.spriteRequest ?? p.legacyAppearance ?? "",
    minimiRequest: p.minimiRequest ?? "",
    // 메모는 성격 프롬프트로 통합됐다 — 예전 번들의 note는 합쳐 싣는다.
    personalityPrompt: mergeLegacyNote(p.personalityPrompt, p.legacyNote),
    keyboardSound: p.keyboardSound ?? "",
  };
}

/** 프로필 + 임베드 이미지(base64)를 자기완결형 번들 JSON 텍스트로 직렬화.
 *  `minimiPngBase64`는 선택 — 없으면 키 자체를 넣지 않아 기존 번들과 동일한 모양이 된다. */
export function serializeBundle(
  profile: PortableProfile,
  portraitPngBase64?: string,
  spritePngBase64?: string,
  minimiPngBase64?: string,
): string {
  const bundle: CharacterBundle = {
    kind: CHARACTER_BUNDLE_KIND,
    schemaVersion: CHARACTER_BUNDLE_SCHEMA_VERSION,
    profile,
    ...(portraitPngBase64 ? { portraitPngBase64 } : {}),
    ...(spritePngBase64 ? { spritePngBase64 } : {}),
    ...(minimiPngBase64 ? { minimiPngBase64 } : {}),
  };
  return JSON.stringify(bundle, null, 2);
}

/** 내보내기 저장 다이얼로그의 기본 파일명. 경로/파일명에 안전하지 않은 문자를
 *  치환하고 확장자를 붙인다. 이름이 비면 "character" 폴백. */
export function buildExportFileName(name: string): string {
  const safe =
    name
      .trim()
      .replace(/[/\\:*?"<>|]+/g, "_")
      .replace(/\s+/g, " ")
      .trim() || "character";
  return `${safe}.aoc.json`;
}
