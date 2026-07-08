// src/renderer/sprite/__tests__/spriteCache.test.ts
//
// spriteCache 순수 헬퍼 + 시작 로드 + 삭제 브리지 TDD. tauriApi만 목킹하고
// 디코드/프리뷰 생성은 io 주입으로 대체해 node 환경에서 검증한다.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "../../store/types";

const { mockApi } = vi.hoisted(() => ({
  mockApi: {
    loadSprite: vi.fn(),
    deleteSprite: vi.fn(),
  },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: mockApi }));

import { useAppStore } from "../../store/appStore";
import {
  getSpriteOverride,
  resetSpriteOverrides,
} from "../../office/gen/spriteOverrides";
import {
  agentsNeedingSprites,
  loadSpritesFor,
  installSpriteCache,
  sheetCanvasDims,
} from "../spriteCache";
import { createCanvas, type Canvas } from "@napi-rs/canvas";
import { CELL } from "../../office/gen/compositor";
import { sheetPreviewUrl } from "../spriteCache";
import type { SpriteCanvasFactory } from "../spriteNormalize";

const initial = useAppStore.getState();
const FAKE_SHEET = { fake: "sheet" } as unknown as CanvasImageSource;
const io = {
  decode: vi.fn(async () => FAKE_SHEET),
  toPreviewUrl: vi.fn(() => "data:image/png;base64,PREVIEW"),
};

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Ada",
    role: "backend",
    note: "",
    seed: "seed",
    createdAt: 1,
    deskIndex: 0,
    ...overrides,
  };
}

beforeEach(() => {
  useAppStore.setState(initial, true);
  resetSpriteOverrides();
  mockApi.loadSprite.mockReset().mockResolvedValue("QUJD");
  mockApi.deleteSprite.mockReset().mockResolvedValue(undefined);
  io.decode.mockClear();
  io.toPreviewUrl.mockClear();
});

afterEach(() => resetSpriteOverrides());

describe("agentsNeedingSprites", () => {
  it("spriteUpdatedAt이 있는 에이전트 id만 돌려준다", () => {
    const agents = {
      a1: mkProfile({ id: "a1", spriteUpdatedAt: 1 }),
      a2: mkProfile({ id: "a2" }),
    };
    expect(agentsNeedingSprites(agents)).toEqual(["a1"]);
  });
});

describe("loadSpritesFor", () => {
  it("로드 성공 시 오버라이드 등록 + 프리뷰 캐시를 채운다", async () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1", spriteUpdatedAt: 1 }));
    await loadSpritesFor(["a1"], io);
    expect(mockApi.loadSprite).toHaveBeenCalledWith("a1");
    expect(io.decode).toHaveBeenCalledWith("QUJD");
    expect(getSpriteOverride("a1")).toBe(FAKE_SHEET);
    expect(useAppStore.getState().spritePreviews["a1"]).toBe(
      "data:image/png;base64,PREVIEW"
    );
  });

  it("파일이 없으면(null) 아무 것도 등록하지 않는다", async () => {
    mockApi.loadSprite.mockResolvedValue(null);
    await loadSpritesFor(["a1"], io);
    expect(getSpriteOverride("a1")).toBeUndefined();
    expect(useAppStore.getState().spritePreviews["a1"]).toBeUndefined();
  });

  it("한 건의 실패는 다른 건의 로드를 막지 않는다", async () => {
    mockApi.loadSprite
      .mockRejectedValueOnce(new Error("boom"))
      .mockResolvedValueOnce("QUJD");
    await loadSpritesFor(["bad", "good"], io);
    expect(getSpriteOverride("good")).toBe(FAKE_SHEET);
    expect(getSpriteOverride("bad")).toBeUndefined();
  });
});

describe("installSpriteCache", () => {
  it("시작 시 spriteUpdatedAt 보유 에이전트만 로드한다", async () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1", spriteUpdatedAt: 1 }));
    useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
    const off = installSpriteCache(io);
    await vi.waitFor(() => expect(getSpriteOverride("a1")).toBe(FAKE_SHEET));
    expect(mockApi.loadSprite).toHaveBeenCalledTimes(1);
    off();
  });

  it("에이전트 제거 시 파일 삭제 + 오버라이드/프리뷰 정리", async () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1", spriteUpdatedAt: 1 }));
    const off = installSpriteCache(io);
    await vi.waitFor(() => expect(getSpriteOverride("a1")).toBe(FAKE_SHEET));

    useAppStore.getState().removeAgent("a1");
    await vi.waitFor(() => expect(mockApi.deleteSprite).toHaveBeenCalledWith("a1"));
    expect(getSpriteOverride("a1")).toBeUndefined();
    expect(useAppStore.getState().spritePreviews["a1"]).toBeUndefined();
    off();
  });
});

describe("sheetPreviewUrl (N 일반화)", () => {
  it("커스텀 N셀 시트의 idle0 셀 전체(N×N)를 CELL*scale px로 확대한다 — 구식 고정 16×16 소스 샘플과 판별", () => {
    // 판별력: idle0(N=32) 내부를 비균일하게 칠한다 — 좌상단 16×16은 빨강, 나머지
    // (16..32 범위 포함)는 파랑. 신식 코드(소스 샘플 N×N=32×32, 배율 96/32=3)는
    // 출력(72,72)이 소스(24,24)=파랑 영역을 가리켜 파랑을 낸다. 구식 코드(소스 샘플
    // 고정 16×16, 배율 96/16=6)라면 출력(72,72)은 소스(12,12)=빨강 영역이 되어 이
    // 파랑 단언에서 실패한다 — 즉 이 테스트는 두 구현을 구별할 수 있다(기존처럼
    // idle0 전체를 균일한 색으로 칠하면 두 구현 모두 같은 색을 내어 판별력이 없었다).
    const N = 32;
    const sheet = createCanvas(4 * N, N);
    const sctx = sheet.getContext("2d");
    sctx.fillStyle = "#0000ff"; sctx.fillRect(0, 0, N, N); // idle0 나머지 = 파랑
    sctx.fillStyle = "#ff0000"; sctx.fillRect(0, 0, 16, 16); // idle0 좌상단 16x16 = 빨강
    sctx.fillStyle = "#00ff00"; sctx.fillRect(N, 0, N, N); // idle1 = 초록(샘플되면 안 됨)

    let captured: Canvas | null = null;
    const factory: SpriteCanvasFactory = (w, h) => {
      const c = createCanvas(w, h);
      captured = c;
      return {
        ctx: c.getContext("2d") as unknown as CanvasRenderingContext2D,
        canvas: c as unknown as ReturnType<SpriteCanvasFactory>["canvas"],
      };
    };

    const url = sheetPreviewUrl(sheet as unknown as CanvasImageSource, 6, factory);
    expect(url.startsWith("data:image/png")).toBe(true);
    const out = captured!;
    expect(out.width).toBe(CELL * 6);
    expect(out.height).toBe(CELL * 6);

    // 소스(24,24)에 대응하는 출력(72,72) — idle0 파랑 영역(N×N 전체를 봐야만 파랑).
    const blue = out.getContext("2d").getImageData(72, 72, 1, 1).data;
    expect(Array.from(blue)).toEqual([0, 0, 255, 255]);

    // 소스(8,8)에 대응하는 출력(24,24) — idle0 좌상단 빨강 영역(양쪽 구현 공통 기준점).
    const red = out.getContext("2d").getImageData(24, 24, 1, 1).data;
    expect(Array.from(red)).toEqual([255, 0, 0, 255]);
  });
});

describe("sheetCanvasDims", () => {
  it("256 이하 시트는 4N×N 그대로 유지한다", () => {
    expect(sheetCanvasDims(1024, 256)).toEqual({ w: 1024, h: 256 });
  });

  it("256 초과 시트는 1024×256으로 다운스케일한다", () => {
    expect(sheetCanvasDims(4096, 1024)).toEqual({ w: 1024, h: 256 });
  });

  it("예상 밖 크기는 64×16 레거시 폴백을 돌려준다", () => {
    expect(sheetCanvasDims(500, 500)).toEqual({ w: 64, h: 16 });
  });

  it("레거시 64×16 시트는 그대로 유지한다", () => {
    expect(sheetCanvasDims(64, 16)).toEqual({ w: 64, h: 16 });
  });
});
