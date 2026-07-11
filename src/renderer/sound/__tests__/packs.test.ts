// 키보드 사운드 팩 레지스트리 — 순수 로직 검증.
// samples/<팩id>/*.wav 디렉터리 구조를 팩 단위로 그룹핑하고,
// 프로필의 keyboardSound 값을 유효한 팩 id로 해석한다.
import { describe, expect, it } from "vitest";
import {
  DEFAULT_KEYBOARD_SOUND_ID,
  groupSampleUrlsByPack,
  listPackOptions,
  packGain,
  pickPackSamples,
  resolvePackId,
} from "../packs";

describe("groupSampleUrlsByPack", () => {
  it("글롭 경로의 디렉터리명을 팩 id로 삼아 URL을 그룹핑한다", () => {
    const grouped = groupSampleUrlsByPack({
      "./samples/cherry-kc1000/keypress-001.wav": "/assets/keypress-001-abc.wav",
      "./samples/cherry-kc1000/keypress-002.wav": "/assets/keypress-002-def.wav",
      "./samples/topre/key-01.wav": "/assets/key-01-xyz.wav",
    });
    expect([...grouped.keys()].sort()).toEqual(["cherry-kc1000", "topre"]);
    expect(grouped.get("cherry-kc1000")).toEqual([
      "/assets/keypress-001-abc.wav",
      "/assets/keypress-002-def.wav",
    ]);
    expect(grouped.get("topre")).toEqual(["/assets/key-01-xyz.wav"]);
  });

  it("팩 디렉터리 밖(루트)의 wav는 무시한다", () => {
    const grouped = groupSampleUrlsByPack({
      "./samples/legacy.wav": "/assets/legacy.wav",
    });
    expect(grouped.size).toBe(0);
  });
});

describe("listPackOptions", () => {
  it("기본 팩을 맨 앞에 두고 나머지는 라벨순으로 정렬한다", () => {
    const opts = listPackOptions(["zzz-unknown", DEFAULT_KEYBOARD_SOUND_ID]);
    expect(opts[0].id).toBe(DEFAULT_KEYBOARD_SOUND_ID);
    expect(opts.map((o) => o.id)).toContain("zzz-unknown");
  });

  it("메타데이터에 없는 팩도 id를 라벨로 삼아 노출한다 (자동 발견)", () => {
    const opts = listPackOptions(["my-custom-pack"]);
    const custom = opts.find((o) => o.id === "my-custom-pack");
    expect(custom).toBeDefined();
    expect(custom!.label).toBe("my-custom-pack");
  });
});

describe("resolvePackId", () => {
  const available = new Set(["cherry-kc1000", "topre"]);

  it("유효한 팩 id는 그대로 돌려준다", () => {
    expect(resolvePackId("topre", available)).toBe("topre");
  });

  it("미지정(undefined)이면 기본 팩", () => {
    expect(resolvePackId(undefined, available)).toBe(DEFAULT_KEYBOARD_SOUND_ID);
  });

  it("무효한 id(팩 삭제됨 등)면 기본 팩으로 폴백", () => {
    expect(resolvePackId("deleted-pack", available)).toBe(DEFAULT_KEYBOARD_SOUND_ID);
  });
});

describe("pickPackSamples", () => {
  const byPack = new Map<string, string[]>([
    [DEFAULT_KEYBOARD_SOUND_ID, ["d1", "d2"]],
    ["topre", ["t1"]],
    ["empty-pack", []],
  ]);

  it("요청한 팩이 로드돼 있으면 그 샘플을 준다", () => {
    expect(pickPackSamples(byPack, "topre")).toEqual(["t1"]);
  });

  it("미지정이면 기본 팩", () => {
    expect(pickPackSamples(byPack, undefined)).toEqual(["d1", "d2"]);
  });

  it("무효한 팩 id면 기본 팩으로 폴백", () => {
    expect(pickPackSamples(byPack, "deleted")).toEqual(["d1", "d2"]);
  });

  it("요청 팩이 비어 있으면(로드 실패) 기본 팩으로 폴백", () => {
    expect(pickPackSamples(byPack, "empty-pack")).toEqual(["d1", "d2"]);
  });

  it("기본 팩까지 비어 있으면 null (합성음 폴백)", () => {
    expect(pickPackSamples(new Map(), "topre")).toBeNull();
  });
});

describe("packGain", () => {
  it("메타데이터에 없는 팩은 보정 배율 1", () => {
    expect(packGain("some-unknown-pack")).toBe(1);
  });
});
