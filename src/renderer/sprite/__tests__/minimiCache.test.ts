// src/renderer/sprite/__tests__/minimiCache.test.ts
//
// minimiCache 순수 헬퍼 + 시작 로드 + 삭제 브리지 TDD. spriteCache 테스트를
// 미러링한다 — tauriApi만 목킹하고 디코드/프리뷰 생성은 io 주입으로 대체한다.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "../../store/types";

const { mockApi } = vi.hoisted(() => ({
  mockApi: {
    loadMinimi: vi.fn(),
    deleteMinimi: vi.fn(),
    // installSpriteCache는 이 테스트에서 설치하지 않지만, 스토어 캐스케이드가
    // 다른 브리지를 건드리지 않는지 확인할 필요는 없으므로 최소 목만 둔다.
  },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: mockApi }));

import { useAppStore } from "../../store/appStore";
import { getMinimiOverride, resetMinimiOverrides } from "../../office/gen/minimiOverrides";
import {
  agentsNeedingMinimis,
  loadMinimisFor,
  installMinimiCache,
  minimiCanvasDims,
  minimiPreviewUrl,
} from "../minimiCache";
import { createCanvas, type Canvas } from "@napi-rs/canvas";
import { CELL } from "../../office/gen/compositor";
import type { SpriteCanvasFactory } from "../spriteNormalize";

const initial = useAppStore.getState();
const FAKE_FRAME = { fake: "frame" } as unknown as CanvasImageSource;
const io = {
  decode: vi.fn(async () => FAKE_FRAME),
  toPreviewUrl: vi.fn(() => "data:image/png;base64,MINIMI"),
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
  resetMinimiOverrides();
  mockApi.loadMinimi.mockReset().mockResolvedValue("QUJD");
  mockApi.deleteMinimi.mockReset().mockResolvedValue(undefined);
  io.decode.mockClear();
  io.toPreviewUrl.mockClear();
});

afterEach(() => resetMinimiOverrides());

describe("agentsNeedingMinimis", () => {
  it("minimiUpdatedAt이 있는 에이전트 id만 돌려준다", () => {
    const agents = {
      a1: mkProfile({ id: "a1", minimiUpdatedAt: 1 }),
      // 스프라이트 커스텀만 있는 에이전트는 미니미 로드 대상이 아니다.
      a2: mkProfile({ id: "a2", spriteUpdatedAt: 9 }),
      a3: mkProfile({ id: "a3" }),
    };
    expect(agentsNeedingMinimis(agents)).toEqual(["a1"]);
  });
});

describe("loadMinimisFor", () => {
  it("로드 성공 시 오버라이드 등록 + 프리뷰 캐시를 채운다", async () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1", minimiUpdatedAt: 1 }));
    await loadMinimisFor(["a1"], io);
    expect(mockApi.loadMinimi).toHaveBeenCalledWith("a1");
    expect(io.decode).toHaveBeenCalledWith("QUJD");
    expect(getMinimiOverride("a1")).toBe(FAKE_FRAME);
    expect(useAppStore.getState().minimiPreviews["a1"]).toBe("data:image/png;base64,MINIMI");
  });

  it("파일이 없으면(null) 아무 것도 등록하지 않는다", async () => {
    mockApi.loadMinimi.mockResolvedValue(null);
    await loadMinimisFor(["a1"], io);
    expect(getMinimiOverride("a1")).toBeUndefined();
    expect(useAppStore.getState().minimiPreviews["a1"]).toBeUndefined();
  });

  it("한 건의 실패는 다른 건의 로드를 막지 않는다", async () => {
    mockApi.loadMinimi.mockRejectedValueOnce(new Error("boom")).mockResolvedValueOnce("QUJD");
    await loadMinimisFor(["bad", "good"], io);
    expect(getMinimiOverride("good")).toBe(FAKE_FRAME);
    expect(getMinimiOverride("bad")).toBeUndefined();
  });
});

describe("installMinimiCache", () => {
  it("시작 시 minimiUpdatedAt 보유 에이전트만 로드한다", async () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1", minimiUpdatedAt: 1 }));
    useAppStore.getState().addAgent(mkProfile({ id: "a2", spriteUpdatedAt: 1 }));
    const off = installMinimiCache(io);
    await vi.waitFor(() => expect(getMinimiOverride("a1")).toBe(FAKE_FRAME));
    expect(mockApi.loadMinimi).toHaveBeenCalledTimes(1);
    off();
  });

  it("에이전트 제거 시 파일 삭제 + 오버라이드/프리뷰 정리", async () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1", minimiUpdatedAt: 1 }));
    const off = installMinimiCache(io);
    await vi.waitFor(() => expect(getMinimiOverride("a1")).toBe(FAKE_FRAME));

    useAppStore.getState().removeAgent("a1");
    await vi.waitFor(() => expect(mockApi.deleteMinimi).toHaveBeenCalledWith("a1"));
    expect(getMinimiOverride("a1")).toBeUndefined();
    expect(useAppStore.getState().minimiPreviews["a1"]).toBeUndefined();
    off();
  });
});

describe("minimiCanvasDims", () => {
  it("정사각 N×N은 그대로(클램프 범위 안)", () => {
    expect(minimiCanvasDims(32, 32)).toEqual({ n: 32, side: 32 });
    expect(minimiCanvasDims(16, 16)).toEqual({ n: 16, side: 16 });
  });

  it("4N×N 시트가 들어오면 첫 프레임(N×N)만 쓴다", () => {
    expect(minimiCanvasDims(128, 32)).toEqual({ n: 32, side: 32 });
    expect(minimiCanvasDims(1024, 256)).toEqual({ n: 256, side: 256 });
  });

  it("비정사각/범위 밖은 짧은 변 기준 + [16,256] 클램프", () => {
    expect(minimiCanvasDims(40, 24)).toEqual({ n: 24, side: 24 });
    expect(minimiCanvasDims(8, 8)).toEqual({ n: 16, side: 8 }); // 하한 클램프
    expect(minimiCanvasDims(512, 512)).toEqual({ n: 256, side: 512 }); // 상한 클램프
  });
});

describe("minimiPreviewUrl", () => {
  it("단일 프레임 전체(N×N)를 CELL*scale px로 확대한다", () => {
    const N = 32;
    const frame = createCanvas(N, N);
    const fctx = frame.getContext("2d");
    fctx.fillStyle = "#0000ff";
    fctx.fillRect(0, 0, N, N);
    fctx.fillStyle = "#ff0000";
    fctx.fillRect(0, 0, 16, 16); // 좌상단 사분면만 빨강

    let captured: Canvas | null = null;
    const factory: SpriteCanvasFactory = (w, h) => {
      const c = createCanvas(w, h);
      captured = c;
      return {
        ctx: c.getContext("2d") as unknown as CanvasRenderingContext2D,
        canvas: c as unknown as ReturnType<SpriteCanvasFactory>["canvas"],
      };
    };

    const url = minimiPreviewUrl(frame as unknown as CanvasImageSource, 6, factory);
    expect(url.startsWith("data:image/png")).toBe(true);
    const out = captured!;
    expect(out.width).toBe(CELL * 6);
    expect(out.height).toBe(CELL * 6);
    // 배율 96/32 = 3. 소스(24,24) → 출력(72,72) = 파랑(프레임 전체를 봐야 나온다).
    expect(Array.from(out.getContext("2d").getImageData(72, 72, 1, 1).data)).toEqual([
      0, 0, 255, 255,
    ]);
    // 소스(8,8) → 출력(24,24) = 빨강.
    expect(Array.from(out.getContext("2d").getImageData(24, 24, 1, 1).data)).toEqual([
      255, 0, 0, 255,
    ]);
  });
});
