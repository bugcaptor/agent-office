// src/renderer/sound/packs.ts
//
// 키보드 사운드 팩 레지스트리. samples/<팩id>/*.wav 디렉터리 하나가 팩
// 하나다 — wav 폴더를 추가하면 빌드만으로 자동 발견되고, PACK_META에
// 라벨/볼륨 보정을 등록하면 UI 표시가 다듬어진다(미등록이면 id가 라벨).
// 프로필의 keyboardSound(팩 id)가 무효/부재면 기본 팩으로 폴백한다.
//
// i18n: 이 레지스트리는 모듈 최상위 상수라 `t()`를 부를 수 없다 — 라벨 값이
// 아니라 `office` 네임스페이스의 **번역 키**를 담고, 번역은 소비처가 렌더
// 시점에 한다(`packLabel`). 자동 발견된 미등록 팩은 번역할 이름이 없으므로
// 키가 없고(undefined) id를 그대로 보여 준다.

export interface KeyboardSoundPackOption {
  id: string;
  /** `office` 네임스페이스의 번역 키. 미등록(자동 발견) 팩은 없다. */
  labelKey?: string;
}

export const DEFAULT_KEYBOARD_SOUND_ID = "cherry-kc1000";

/** 알려진 팩 메타데이터. gain은 팩 간 녹음 레벨 차이 보정 배율(기본 1). */
const PACK_META: Record<string, { labelKey: string; gain?: number }> = {
  "cherry-kc1000": { labelKey: "office:soundPack.cherryKc1000" },
  "topre-hhkb": { labelKey: "office:soundPack.topreHhkb" },
  "mech-clicky": { labelKey: "office:soundPack.mechClicky" },
  membrane: { labelKey: "office:soundPack.membrane" },
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

/**
 * 발견된 팩 id들 → UI 셀렉트 옵션. 기본 팩 먼저, 나머지는 라벨 키 순.
 *
 * 정렬 기준이 번역문이 아니라 **키**인 이유: 이 목록은 모듈 최상위에서 한 번
 * 만들어지므로 번역문으로 정렬하면 그때의 언어로 순서가 굳는다. 키 순서는
 * 언어와 무관하게 안정적이고, 지금 등록된 네 팩은 ko/en 라벨순과도 같다.
 */
export function listPackOptions(discoveredIds: Iterable<string>): KeyboardSoundPackOption[] {
  const ids = [...new Set(discoveredIds)];
  const toOption = (id: string): KeyboardSoundPackOption => {
    const labelKey = PACK_META[id]?.labelKey;
    return labelKey ? { id, labelKey } : { id };
  };
  const sortKey = (o: KeyboardSoundPackOption): string => o.labelKey ?? o.id;
  const rest = ids
    .filter((id) => id !== DEFAULT_KEYBOARD_SOUND_ID)
    .map(toOption)
    .sort((a, b) => sortKey(a).localeCompare(sortKey(b)));
  return ids.includes(DEFAULT_KEYBOARD_SOUND_ID)
    ? [toOption(DEFAULT_KEYBOARD_SOUND_ID), ...rest]
    : rest;
}

/** 셀렉트에 노출할 팩 이름. `t`는 호출자(컴포넌트)의 번역 함수 —
 *  미등록 팩은 번역할 이름이 없으므로 id를 그대로 보여 준다. */
export function packLabel(
  option: KeyboardSoundPackOption,
  t: (key: string) => string,
): string {
  return option.labelKey ? t(option.labelKey) : option.id;
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
