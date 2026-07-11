// src/renderer/sound/packs.ts
//
// 키보드 사운드 팩 레지스트리. samples/<팩id>/*.wav 디렉터리 하나가 팩
// 하나다 — wav 폴더를 추가하면 빌드만으로 자동 발견되고, PACK_META에
// 라벨/볼륨 보정을 등록하면 UI 표시가 다듬어진다(미등록이면 id가 라벨).
// 프로필의 keyboardSound(팩 id)가 무효/부재면 기본 팩으로 폴백한다.

export interface KeyboardSoundPackOption {
  id: string;
  label: string;
}

export const DEFAULT_KEYBOARD_SOUND_ID = "cherry-kc1000";

/** 알려진 팩 메타데이터. gain은 팩 간 녹음 레벨 차이 보정 배율(기본 1). */
const PACK_META: Record<string, { label: string; gain?: number }> = {
  "cherry-kc1000": { label: "기계식 — Cherry KC1000" },
  "topre-hhkb": { label: "토프레 — HHKB (통통)" },
  "mech-clicky": { label: "기계식 — 클릭키 (카랑카랑)" },
  membrane: { label: "멤브레인 — 사무용" },
};

// Vite가 번들 URL로 바꿔준다(dev/build 공통). samples/<팩id>/*.wav 폴더가
// 팩 하나 — 폴더를 추가하면 여기서 자동 발견된다.
export const PACK_SAMPLE_URLS: ReadonlyMap<string, string[]> = groupSampleUrlsByPack(
  import.meta.glob("./samples/*/*.wav", {
    eager: true,
    query: "?url",
    import: "default",
  }) as Record<string, string>
);

/** 번들에서 발견된 팩들의 UI 옵션 (기본 팩 먼저). */
export const KEYBOARD_SOUND_PACK_OPTIONS: readonly KeyboardSoundPackOption[] = listPackOptions(
  PACK_SAMPLE_URLS.keys()
);

/**
 * import.meta.glob("./samples/*\/*.wav") 결과(키=상대경로, 값=번들 URL)를
 * 팩 id(디렉터리명)별 URL 목록으로 그룹핑. 루트 바로 아래 wav는 무시.
 */
export function groupSampleUrlsByPack(
  globEntries: Record<string, string>
): Map<string, string[]> {
  const grouped = new Map<string, string[]>();
  for (const path of Object.keys(globEntries).sort()) {
    const m = /\/([^/]+)\/[^/]+\.wav$/.exec(path);
    if (!m || m[1] === "samples") continue;
    const urls = grouped.get(m[1]) ?? [];
    urls.push(globEntries[path]);
    grouped.set(m[1], urls);
  }
  return grouped;
}

/** 발견된 팩 id들 → UI 셀렉트 옵션. 기본 팩 먼저, 나머지 라벨순. */
export function listPackOptions(discoveredIds: Iterable<string>): KeyboardSoundPackOption[] {
  const ids = [...new Set(discoveredIds)];
  const toOption = (id: string): KeyboardSoundPackOption => ({
    id,
    label: PACK_META[id]?.label ?? id,
  });
  const rest = ids
    .filter((id) => id !== DEFAULT_KEYBOARD_SOUND_ID)
    .map(toOption)
    .sort((a, b) => a.label.localeCompare(b.label, "ko"));
  return ids.includes(DEFAULT_KEYBOARD_SOUND_ID)
    ? [toOption(DEFAULT_KEYBOARD_SOUND_ID), ...rest]
    : rest;
}

/** 프로필의 keyboardSound 값 → 유효한 팩 id. 미지정/무효 = 기본 팩. */
export function resolvePackId(
  requested: string | undefined,
  available: ReadonlySet<string>
): string {
  return requested && available.has(requested) ? requested : DEFAULT_KEYBOARD_SOUND_ID;
}

/**
 * 재생 시점의 샘플 선택: 요청 팩(무효/부재면 기본 팩) → 비어 있으면(로드
 * 전/실패) 기본 팩 → 그것도 비면 null(호출측이 합성음으로 폴백).
 */
export function pickPackSamples<T>(
  byPack: ReadonlyMap<string, readonly T[]>,
  requested: string | undefined
): readonly T[] | null {
  const loaded = new Set([...byPack.keys()].filter((id) => (byPack.get(id)?.length ?? 0) > 0));
  const id = resolvePackId(requested, loaded);
  return byPack.get(id)?.length ? byPack.get(id)! : null;
}

/** 팩별 볼륨 보정 배율. 메타 미등록 팩은 1. */
export function packGain(id: string): number {
  return PACK_META[id]?.gain ?? 1;
}
