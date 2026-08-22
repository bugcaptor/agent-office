// src/renderer/profile/__tests__/characterIo.test.ts
//
// 캐릭터 내보내기/가져오기(이슈 #77) 순수 계층 검증: draft↔번들 라운드트립,
// 로컬 환경 필드 배제, 파서의 kind/schemaVersion/이미지 상한 거부, 파일명 안전화.
import { describe, expect, it } from "vitest";
import {
  portableFromDraft,
  applyBundleToDraft,
  serializeBundle,
  buildExportFileName,
} from "../characterIo";
import { generateDraft, type DraftProfile } from "../generate";
import {
  parseCharacterBundle,
  CHARACTER_BUNDLE_KIND,
  CHARACTER_BUNDLE_SCHEMA_VERSION,
  MAX_PORTRAIT_BYTES,
  MAX_SPRITE_BYTES,
  MAX_MINIMI_BYTES,
} from "@shared/types";

function draftWith(over: Partial<DraftProfile>): DraftProfile {
  return { ...generateDraft(), ...over };
}

describe("portableFromDraft", () => {
  it("이식 필드만 추리고 로컬 환경 필드는 담지 않는다", () => {
    const d = draftWith({
      name: "  Nova ",
      role: " 코더 ",
      seed: "seed123",
      archetype: "human",
      portraitRequest: " 안경 ",
      spriteRequest: " 망토 ",
      personalityPrompt: " 성격 ",
      keyboardSound: "typewriter",
      // 로컬 환경(제외 대상)
      cwd: "/home/x/dev",
      shell: "pwsh",
      startupCommand: "source ./init.sh",
      botSlug: "nova",
      botWhitelist: "a, b",
    });
    const p = portableFromDraft(d);
    expect(p).toEqual({
      name: "Nova",
      role: "코더",
      seed: "seed123",
      archetype: "human",
      portraitRequest: "안경",
      spriteRequest: "망토",
      personalityPrompt: "성격",
      keyboardSound: "typewriter",
    });
    // 로컬 환경 키가 어디에도 새어나오지 않음.
    expect(p).not.toHaveProperty("cwd");
    expect(p).not.toHaveProperty("shell");
    expect(p).not.toHaveProperty("startupCommand");
    expect(p).not.toHaveProperty("botSlug");
  });

  it("archetype=auto는 '자동' 의도 보존을 위해 생략한다", () => {
    const p = portableFromDraft(draftWith({ archetype: "auto" }));
    expect(p.archetype).toBeUndefined();
  });

  it("빈 선택 필드는 undefined로 생략한다", () => {
    const p = portableFromDraft(
      draftWith({ portraitRequest: "  ", spriteRequest: "", personalityPrompt: "  ", keyboardSound: "" }),
    );
    expect(p.portraitRequest).toBeUndefined();
    expect(p.spriteRequest).toBeUndefined();
    expect(p.personalityPrompt).toBeUndefined();
    expect(p.keyboardSound).toBeUndefined();
  });
});

describe("applyBundleToDraft", () => {
  it("이식 필드는 덮어쓰고 로컬 환경 필드는 유지한다", () => {
    const cur = draftWith({
      name: "Old",
      role: "old-role",
      cwd: "/keep/me",
      shell: "git-bash",
      startupCommand: "keep.sh",
      botSlug: "keepbot",
    });
    const next = applyBundleToDraft(cur, {
      name: "New",
      role: "new-role",
      seed: "s2",
      archetype: "robot",
      portraitRequest: "a",
      spriteRequest: "sr",
      personalityPrompt: "pp",
      keyboardSound: "ks",
    });
    expect(next.name).toBe("New");
    expect(next.role).toBe("new-role");
    expect(next.archetype).toBe("robot");
    expect(next.portraitRequest).toBe("a");
    // 로컬 환경은 그대로.
    expect(next.cwd).toBe("/keep/me");
    expect(next.shell).toBe("git-bash");
    expect(next.startupCommand).toBe("keep.sh");
    expect(next.botSlug).toBe("keepbot");
  });

  it("옛 번들의 note/appearance는 성격 프롬프트와 두 추가 프롬프트로 흡수된다", () => {
    const parsed = parseCharacterBundle(
      JSON.stringify({
        kind: CHARACTER_BUNDLE_KIND,
        schemaVersion: CHARACTER_BUNDLE_SCHEMA_VERSION,
        profile: {
          name: "Old",
          role: "r",
          seed: "s",
          note: "백엔드 담당",
          appearance: "짧은 검은 머리",
          personalityPrompt: "차분한 성격",
        },
      }),
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const next = applyBundleToDraft(draftWith({}), parsed.bundle.profile);
    expect(next.personalityPrompt).toBe("차분한 성격\n백엔드 담당");
    expect(next.portraitRequest).toBe("짧은 검은 머리");
    expect(next.spriteRequest).toBe("짧은 검은 머리");
  });

  it("빈 이름/시드는 기존 draft 값으로 폴백하고, 없는 archetype은 auto", () => {
    const cur = draftWith({ name: "Keep", seed: "keep-seed" });
    const next = applyBundleToDraft(cur, {
      name: "   ",
      role: "",
      seed: "",
    });
    expect(next.name).toBe("Keep");
    expect(next.seed).toBe("keep-seed");
    expect(next.archetype).toBe("auto");
  });
});

describe("serialize → parse 라운드트립", () => {
  it("이미지 포함 번들이 손실 없이 왕복한다", () => {
    const profile = portableFromDraft(draftWith({ name: "Nova", seed: "s", archetype: "human" }));
    const json = serializeBundle(profile, "UE5H-portrait", "UE5H-sprite");
    const res = parseCharacterBundle(json);
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.bundle.kind).toBe(CHARACTER_BUNDLE_KIND);
    expect(res.bundle.schemaVersion).toBe(CHARACTER_BUNDLE_SCHEMA_VERSION);
    expect(res.bundle.profile.name).toBe("Nova");
    expect(res.bundle.portraitPngBase64).toBe("UE5H-portrait");
    expect(res.bundle.spritePngBase64).toBe("UE5H-sprite");
  });

  it("이미지 없는 번들도 왕복하고 이미지 필드는 undefined", () => {
    const json = serializeBundle(portableFromDraft(draftWith({ name: "Bare" })));
    const res = parseCharacterBundle(json);
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.bundle.portraitPngBase64).toBeUndefined();
    expect(res.bundle.spritePngBase64).toBeUndefined();
    expect(res.bundle.minimiPngBase64).toBeUndefined();
  });

  it("미니미 PNG도 손실 없이 왕복한다", () => {
    const profile = portableFromDraft(draftWith({ name: "Nova" }));
    const json = serializeBundle(profile, undefined, "UE5H-sprite", "UE5H-minimi");
    const res = parseCharacterBundle(json);
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.bundle.spritePngBase64).toBe("UE5H-sprite");
    expect(res.bundle.minimiPngBase64).toBe("UE5H-minimi");
  });

  it("미니미가 없으면 키 자체를 넣지 않는다(기존 번들과 동일한 모양)", () => {
    const json = serializeBundle(portableFromDraft(draftWith({ name: "Nova" })), undefined, "S");
    expect(json).not.toContain("minimiPngBase64");
  });

  it("미니미 키가 없는 기존 v1 번들도 그대로 읽힌다(하위호환)", () => {
    const res = parseCharacterBundle(
      JSON.stringify({
        kind: CHARACTER_BUNDLE_KIND,
        schemaVersion: CHARACTER_BUNDLE_SCHEMA_VERSION,
        profile: { name: "Old" },
        spritePngBase64: "S",
      }),
    );
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.bundle.minimiPngBase64).toBeUndefined();
  });
});

describe("parseCharacterBundle 거부 경로", () => {
  it("JSON이 아니면 거부", () => {
    const res = parseCharacterBundle("not json {");
    expect(res.ok).toBe(false);
  });

  it("kind가 다르면 거부", () => {
    const res = parseCharacterBundle(JSON.stringify({ kind: "something-else", schemaVersion: 1, profile: { name: "x" } }));
    expect(res.ok).toBe(false);
  });

  it("미래 스키마 버전은 명확히 거부", () => {
    const res = parseCharacterBundle(
      JSON.stringify({ kind: CHARACTER_BUNDLE_KIND, schemaVersion: 999, profile: { name: "x" } }),
    );
    expect(res.ok).toBe(false);
    if (res.ok) return;
    expect(res.error).toContain("최신");
  });

  it("이름 없는 프로필은 거부", () => {
    const res = parseCharacterBundle(
      JSON.stringify({ kind: CHARACTER_BUNDLE_KIND, schemaVersion: 1, profile: {} }),
    );
    expect(res.ok).toBe(false);
  });

  it("초상 이미지가 상한 초과면 가져오기 전체를 거부", () => {
    // base64 길이 → 디코드 근사 바이트 = len*3/4. 상한 초과 길이를 만든다.
    const oversized = "A".repeat(Math.ceil((MAX_PORTRAIT_BYTES + 1024) * 4 / 3));
    const res = parseCharacterBundle(
      JSON.stringify({
        kind: CHARACTER_BUNDLE_KIND,
        schemaVersion: 1,
        profile: { name: "x" },
        portraitPngBase64: oversized,
      }),
    );
    expect(res.ok).toBe(false);
  });

  it("스프라이트 이미지가 상한 초과면 가져오기 전체를 거부", () => {
    const oversized = "A".repeat(Math.ceil((MAX_SPRITE_BYTES + 1024) * 4 / 3));
    const res = parseCharacterBundle(
      JSON.stringify({
        kind: CHARACTER_BUNDLE_KIND,
        schemaVersion: 1,
        profile: { name: "x" },
        spritePngBase64: oversized,
      }),
    );
    expect(res.ok).toBe(false);
  });

  it("미니미 이미지가 상한 초과면 가져오기 전체를 거부", () => {
    const oversized = "A".repeat(Math.ceil(((MAX_MINIMI_BYTES + 1024) * 4) / 3));
    const res = parseCharacterBundle(
      JSON.stringify({
        kind: CHARACTER_BUNDLE_KIND,
        schemaVersion: 1,
        profile: { name: "x" },
        minimiPngBase64: oversized,
      }),
    );
    expect(res.ok).toBe(false);
  });

  it("미니미 이미지 타입이 문자열이 아니면 거부", () => {
    const res = parseCharacterBundle(
      JSON.stringify({
        kind: CHARACTER_BUNDLE_KIND,
        schemaVersion: 1,
        profile: { name: "x" },
        minimiPngBase64: 123,
      }),
    );
    expect(res.ok).toBe(false);
  });
});

describe("buildExportFileName", () => {
  it("경로 위험 문자를 치환하고 .aoc.json을 붙인다", () => {
    expect(buildExportFileName("a/b:c*?")).toBe("a_b_c_.aoc.json");
  });
  it("빈 이름은 character로 폴백", () => {
    expect(buildExportFileName("   ")).toBe("character.aoc.json");
  });
});
