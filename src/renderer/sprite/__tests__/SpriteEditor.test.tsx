// @vitest-environment jsdom
//
// src/renderer/sprite/__tests__/SpriteEditor.test.tsx
//
// SpriteEditor 모달의 열림/닫힘/저장 가드 TDD. 이미지 로딩 없는 상태에서
// jsdom-safe(redraw는 이미지 없으면 early-return, 캔버스 드로잉 미발생).
import { StrictMode } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const saveSprite = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { saveSprite: (...args: unknown[]) => saveSprite(...args) },
}));

// initialImage 경로는 캔버스 2D를 실제로 쓰므로 jsdom에서는 전부 스텁한다.
// 정규화 로직 자체는 spriteNormalize.test.ts가 실캔버스(@napi-rs)로 검증 —
// 여기서는 "initialImage가 들어오면 업로드 없이 저장 가능 상태가 된다"는
// 오케스트레이션만 다룬다.
const fakeSheet = {
  toDataURL: () => "data:image/png;base64,SHEET",
} as unknown as HTMLCanvasElement;
/** ingestImage 1회당 isFullyOpaque가 정확히 1회 호출된다 — ingest 횟수 관측용. */
const ingest = vi.hoisted(() => ({ count: 0 }));
vi.mock("../spriteNormalize", () => ({
  applyBackgroundKey: vi.fn(),
  dataUrlToBase64: (u: string) => u.split(",")[1] ?? "",
  detectSheet: () => ({ kind: "single" }),
  isFullyOpaque: () => {
    ingest.count += 1;
    return false;
  },
  normalizeCrop: () => ({ sheet: fakeSheet, n: 64 }),
  normalizeSheet: () => ({ sheet: fakeSheet, n: 64 }),
}));
vi.mock("../spriteCache", () => ({
  sheetPreviewUrl: () => "data:image/png;base64,PREVIEW",
}));

/** jsdom Image는 리소스를 로드하지 않는다 — src 대입 시 onload를 비동기로 발화. */
class FakeImage {
  /** src로 대입된 URL 기록 — "initialImage는 1회만 소비" 계약 단언용. */
  static loadedSrcs: string[] = [];
  onload: null | (() => void) = null;
  onerror: null | (() => void) = null;
  naturalWidth = 64;
  naturalHeight = 64;
  set src(v: string) {
    FakeImage.loadedSrcs.push(v);
    queueMicrotask(() => this.onload?.());
  }
}

const { SpriteEditor } = await import("../SpriteEditor");

afterEach(() => cleanup());
beforeEach(() => saveSprite.mockClear());

describe("SpriteEditor", () => {
  it("제목/파일 입력/버튼을 렌더한다", () => {
    render(<SpriteEditor agentId="a1" onClose={() => {}} />);
    expect(screen.getByText("픽셀아트 편집")).toBeTruthy();
    expect(screen.getByText("저장")).toBeTruthy();
    expect(screen.getByText("취소")).toBeTruthy();
  });

  it("이미지가 없으면 저장 버튼이 비활성이고 저장이 호출되지 않는다", () => {
    render(<SpriteEditor agentId="a1" onClose={() => {}} />);
    const save = screen.getByText("저장") as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    fireEvent.click(save);
    expect(saveSprite).not.toHaveBeenCalled();
  });

  it("취소를 누르면 onClose가 호출된다", () => {
    const onClose = vi.fn();
    render(<SpriteEditor agentId="a1" onClose={onClose} />);
    fireEvent.click(screen.getByText("취소"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("백드롭 자신을 mousedown해야만 닫힌다 (내부 클릭은 무시)", () => {
    const onClose = vi.fn();
    const { container } = render(<SpriteEditor agentId="a1" onClose={onClose} />);
    const backdrop = container.querySelector(".modal-backdrop")!;
    fireEvent.mouseDown(screen.getByText("픽셀아트 편집"));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.mouseDown(backdrop);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("배경 투명화 체크박스를 라벨과 함께 렌더하고 기본은 해제 상태다", () => {
    render(<SpriteEditor agentId="a1" onClose={() => {}} />);
    const cb = screen.getByLabelText("배경 투명화 (좌상단 픽셀 색 기준)") as HTMLInputElement;
    expect(cb).toBeTruthy();
    expect(cb.type).toBe("checkbox");
    expect(cb.checked).toBe(false); // 이미지 없으면 해제
  });

  it("이미지가 없을 때 체크박스를 토글하면 상태만 바뀌고 캔버스 연산은 없다", () => {
    render(<SpriteEditor agentId="a1" onClose={() => {}} />);
    const cb = screen.getByLabelText("배경 투명화 (좌상단 픽셀 색 기준)") as HTMLInputElement;
    fireEvent.click(cb);
    expect(cb.checked).toBe(true);
    fireEvent.click(cb);
    expect(cb.checked).toBe(false);
  });

  describe("initialImage", () => {
    beforeEach(() => {
      FakeImage.loadedSrcs = [];
      ingest.count = 0;
      vi.stubGlobal("Image", FakeImage);
      // jsdom getContext는 null — 컴포넌트가 쓰는 최소 표면만 스텁.
      // (spyOn은 오버로드 중 마지막 시그니처로 추론되므로 메서드 반환형으로 캐스팅.)
      vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
        imageSmoothingEnabled: false,
        drawImage: () => {},
        clearRect: () => {},
      } as unknown as ReturnType<HTMLCanvasElement["getContext"]>);
    });
    afterEach(() => {
      vi.unstubAllGlobals();
      vi.restoreAllMocks();
      // jsdom URL에는 createObjectURL이 없어 테스트에서 임시 부착 — 원상 복구.
      delete (URL as { createObjectURL?: unknown }).createObjectURL;
      delete (URL as { revokeObjectURL?: unknown }).revokeObjectURL;
    });

    it("initialImage가 있으면 업로드 없이 저장 버튼이 활성화된다", async () => {
      render(
        <SpriteEditor agentId="a1" onClose={() => {}} initialImage="data:image/png;base64,GEN" />,
      );
      const save = screen.getByText("저장") as HTMLButtonElement;
      await waitFor(() => expect(save.disabled).toBe(false));
    });

    it("initialImage 로드 후 저장하면 saveSprite가 정규화된 시트로 호출되고 닫힌다", async () => {
      const onClose = vi.fn();
      render(
        <SpriteEditor agentId="a1" onClose={onClose} initialImage="data:image/png;base64,GEN" />,
      );
      const save = screen.getByText("저장") as HTMLButtonElement;
      await waitFor(() => expect(save.disabled).toBe(false));
      fireEvent.click(save);
      await waitFor(() => expect(saveSprite).toHaveBeenCalledWith("a1", "SHEET"));
      await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    });

    it("initialImage가 없으면 기존과 동일 (저장 비활성)", () => {
      render(<SpriteEditor agentId="a1" onClose={() => {}} />);
      expect((screen.getByText("저장") as HTMLButtonElement).disabled).toBe(true);
    });

    it("initialImage는 1회만 소비된다 — mode 전이·사용자 업로드 후에도 재로드 없음", async () => {
      const GEN = "data:image/png;base64,GEN";
      const { container } = render(
        <SpriteEditor agentId="a1" onClose={() => {}} initialImage={GEN} />,
      );
      const save = screen.getByText("저장") as HTMLButtonElement;
      await waitFor(() => expect(save.disabled).toBe(false));
      // ingest가 mode를 empty→crop으로 바꾸면 ingestImage identity가 변해 효과
      // deps가 바뀐다 — one-shot 가드가 없으면 여기서 GEN이 두 번 디코드된다.
      await new Promise((r) => setTimeout(r, 0));
      expect(FakeImage.loadedSrcs.filter((s) => s === GEN)).toHaveLength(1);

      // 사용자 파일 업로드가 GEN 재로드로 덮어써지지 않는다.
      (URL as { createObjectURL?: unknown }).createObjectURL = () => "blob:user-upload";
      (URL as { revokeObjectURL?: unknown }).revokeObjectURL = () => {};
      const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement;
      fireEvent.change(fileInput, {
        target: { files: [new File(["x"], "u.png", { type: "image/png" })] },
      });
      await waitFor(() => expect(FakeImage.loadedSrcs).toContain("blob:user-upload"));
      await new Promise((r) => setTimeout(r, 0));
      expect(FakeImage.loadedSrcs.filter((s) => s === GEN)).toHaveLength(1);
      expect(FakeImage.loadedSrcs[FakeImage.loadedSrcs.length - 1]).toBe("blob:user-upload");
    });

    it("StrictMode 이중 효과에서도 initialImage가 로드되고 ingest는 정확히 1회다", async () => {
      // StrictMode dev 이중 효과(run→cleanup→re-run): 진입부에서 consumed를
      // 마킹하면 run#1이 마킹 후 취소되고 run#2가 가드에 막혀 영원히 빈 에디터가
      // 된다 — 실제 소비(onload→ingest) 시점 마킹을 회귀 고정한다.
      render(
        <StrictMode>
          <SpriteEditor
            agentId="a1"
            onClose={() => {}}
            initialImage="data:image/png;base64,GEN"
          />
        </StrictMode>,
      );
      const save = screen.getByText("저장") as HTMLButtonElement;
      await waitFor(() => expect(save.disabled).toBe(false));
      // 디코드 시도(loadedSrcs)는 run#1/run#2 각 1회일 수 있으나,
      // 상태 반영(ingest)은 정확히 1회여야 한다.
      await new Promise((r) => setTimeout(r, 0));
      expect(ingest.count).toBe(1);
    });

    it("마운트 후 initialImage 주입(늦은 생성 응답 난입)은 무시된다", async () => {
      // 사용자가 생성 대기 중 업로드 버튼으로 에디터를 수동 오픈(initialImage 없음)
      // → 작업 중 늦은 응답이 prop을 undefined→dataURL로 바꿔도 덮어쓰지 않는다.
      const GEN = "data:image/png;base64,GEN";
      const { rerender } = render(<SpriteEditor agentId="a1" onClose={() => {}} />);
      const save = screen.getByText("저장") as HTMLButtonElement;
      expect(save.disabled).toBe(true);
      rerender(<SpriteEditor agentId="a1" onClose={() => {}} initialImage={GEN} />);
      await new Promise((r) => setTimeout(r, 0));
      expect(FakeImage.loadedSrcs).not.toContain(GEN); // 디코드 시도 자체가 없음
      expect(ingest.count).toBe(0);
      expect(save.disabled).toBe(true);
    });
  });
});
