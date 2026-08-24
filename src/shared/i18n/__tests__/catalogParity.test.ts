// 카탈로그 파리티 — 언어를 추가해도(폴더 하나 더 만들어도) 자동으로 검사
// 대상이 된다. "언어 추가 = JSON 추가뿐"이라는 약속을 지키게 하는 장치라,
// 새 언어에서 키를 빠뜨리거나 `{{보간}}` 이름을 흘리면 여기서 걸린다.
//
// 정본은 ko다(SOURCE_LANGUAGE) — 다른 언어는 ko의 키 집합을 정확히 재현해야 한다.
import { describe, expect, it } from "vitest";

import { SOURCE_LANGUAGE, SUPPORTED_LANGUAGES, resources } from "../catalog";

/** 중첩 객체를 `a.b.c` 평탄 키 → 문자열 맵으로 편다. */
function flatten(obj: unknown, prefix = ""): Record<string, string> {
  const out: Record<string, string> = {};
  if (obj === null || typeof obj !== "object") return out;
  for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v !== null && typeof v === "object" && !Array.isArray(v)) {
      Object.assign(out, flatten(v, key));
    } else {
      out[key] = String(v);
    }
  }
  return out;
}

/** 값 안의 `{{name}}` 보간 이름들(순서 무관 비교용으로 정렬). */
function placeholders(value: string): string[] {
  return [...value.matchAll(/\{\{\s*([^}\s,]+)[^}]*\}\}/g)].map((m) => m[1]).sort();
}

function flatCatalog(lang: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [ns, content] of Object.entries(resources[lang] ?? {})) {
    for (const [k, v] of Object.entries(flatten(content))) out[`${ns}:${k}`] = v;
  }
  return out;
}

/** `_meta`는 번역문이 아니라 카탈로그 메타데이터라 값 비교 대상에서 뺀다. */
function isMeta(key: string): boolean {
  return key.includes(":_meta.");
}

describe("번역 카탈로그", () => {
  it("정본 언어(ko) 카탈로그가 존재한다", () => {
    expect(SUPPORTED_LANGUAGES).toContain(SOURCE_LANGUAGE);
    expect(Object.keys(resources[SOURCE_LANGUAGE] ?? {}).length).toBeGreaterThan(0);
  });

  it("모든 언어가 같은 네임스페이스 집합을 갖는다", () => {
    const expected = Object.keys(resources[SOURCE_LANGUAGE]).sort();
    for (const lang of SUPPORTED_LANGUAGES) {
      expect({ lang, ns: Object.keys(resources[lang]).sort() }).toEqual({ lang, ns: expected });
    }
  });

  it("모든 언어가 정본과 같은 키 집합을 갖는다", () => {
    const source = flatCatalog(SOURCE_LANGUAGE);
    const expected = Object.keys(source).sort();
    for (const lang of SUPPORTED_LANGUAGES) {
      const keys = Object.keys(flatCatalog(lang)).sort();
      const missing = expected.filter((k) => !keys.includes(k));
      const extra = keys.filter((k) => !expected.includes(k));
      expect({ lang, missing, extra }).toEqual({ lang, missing: [], extra: [] });
    }
  });

  it("보간 플레이스홀더 집합이 언어마다 같다", () => {
    const source = flatCatalog(SOURCE_LANGUAGE);
    for (const lang of SUPPORTED_LANGUAGES) {
      if (lang === SOURCE_LANGUAGE) continue;
      const target = flatCatalog(lang);
      for (const [key, value] of Object.entries(source)) {
        if (isMeta(key)) continue;
        expect({ lang, key, ph: placeholders(target[key] ?? "") }).toEqual({
          lang,
          key,
          ph: placeholders(value),
        });
      }
    }
  });

  it("빈 문자열인 번역이 없다", () => {
    for (const lang of SUPPORTED_LANGUAGES) {
      const empty = Object.entries(flatCatalog(lang))
        .filter(([, v]) => v.trim() === "")
        .map(([k]) => k);
      expect({ lang, empty }).toEqual({ lang, empty: [] });
    }
  });

  it("각 언어가 자기 이름(_meta.label)을 갖는다", () => {
    for (const lang of SUPPORTED_LANGUAGES) {
      const meta = resources[lang].common?._meta as { label?: string } | undefined;
      expect({ lang, hasLabel: Boolean(meta?.label) }).toEqual({ lang, hasLabel: true });
    }
  });

  it("정본이 아닌 언어에 한글 번역이 남아 있지 않다", () => {
    for (const lang of SUPPORTED_LANGUAGES) {
      if (lang === SOURCE_LANGUAGE) continue;
      const untranslated = Object.entries(flatCatalog(lang))
        .filter(([k, v]) => !isMeta(k) && /[가-힣]/.test(v))
        .map(([k]) => k);
      expect({ lang, untranslated }).toEqual({ lang, untranslated: [] });
    }
  });
});
