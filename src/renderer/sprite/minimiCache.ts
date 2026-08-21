// src/renderer/sprite/minimiCache.ts
//
// 서브에이전트 미니미 픽셀아트 캐시 배선 — `spriteCache`를 미러링한다. 앱 시작 시
// minimiUpdatedAt이 있는 에이전트의 PNG를 병렬 로드해 디코드된 캔버스를
// minimiOverrides에, 프리뷰 dataURL을 스토어(minimiPreviews)에 채운다.
// 스프라이트와 다른 점은 저장물이 4N×N 시트가 아니라 **단일 N×N 프레임**이라는 것뿐.
// 디코드/프리뷰 생성은 io 주입점으로 분리해 node 환경 테스트를 가능하게 한다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { AgentProfile } from "../store/types";
import { CELL } from "../office/gen/compositor";
import { setMinimiOverride, clearMinimiOverride } from "../office/gen/minimiOverrides";
import {
  CELL_MAX,
  CELL_MIN,
  defaultSpriteCanvasFactory,
  detectSheet,
  type SpriteCanvasFactory,
} from "./spriteNormalize";

export interface MinimiCacheIo {
  decode?: (b64: string) => Promise<CanvasImageSource>;
  toPreviewUrl?: (frame: CanvasImageSource) => string;
}

/** minimiUpdatedAt이 있는(=커스텀 존재) 에이전트 id 목록. 순수. */
export function agentsNeedingMinimis(agents: Record<string, AgentProfile>): string[] {
  return Object.values(agents)
    .filter((a) => a.minimiUpdatedAt != null)
    .map((a) => a.id);
}

/**
 * 디코드 캔버스 크기 결정(순수). 저장물은 정규화된 단일 N×N이지만, 손으로 넣은
 * 파일이나 구버전 산출물도 관용적으로 받는다:
 * - 4N×N 시트면 첫 프레임만 쓰므로 N×N
 * - 그 외는 짧은 변 기준 정사각, [16,256] 클램프
 * jsdom이 실제 PNG를 디코드 못 해 `decodeMinimi` 자체는 유닛 테스트 불가하므로,
 * 이 순수 함수로 분기를 검증한다.
 */
export function minimiCanvasDims(w: number, h: number): { n: number; side: number } {
  const det = detectSheet(w, h);
  const side = det.kind === "sheet" ? h : Math.min(w, h);
  return { n: Math.max(CELL_MIN, Math.min(CELL_MAX, Math.round(side))), side };
}

/** base64 PNG → 단일 N×N 캔버스로 디코드. 시트가 들어오면 첫 프레임만 잘라 쓴다. */
function decodeMinimi(b64: string): Promise<CanvasImageSource> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const { n, side } = minimiCanvasDims(img.naturalWidth, img.naturalHeight);
      const canvas = document.createElement("canvas");
      const ctx = canvas.getContext("2d")!;
      // 캔버스 리사이즈가 2D 컨텍스트 상태를 리셋하므로, 크기 설정 후에 플래그를 세운다.
      canvas.width = n;
      canvas.height = n;
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(img, 0, 0, side, side, 0, 0, n, n);
      resolve(canvas);
    };
    img.onerror = () => reject(new Error("minimi decode failed"));
    img.src = `data:image/png;base64,${b64}`;
  });
}

/** 미니미 프레임을 CELL*scale px로 확대한 PNG dataURL(프로필 프리뷰용).
 *  `sheetPreviewUrl`과 같은 규약이되 소스가 단일 프레임이다. */
export function minimiPreviewUrl(
  frame: CanvasImageSource,
  scale = 6,
  factory: SpriteCanvasFactory = defaultSpriteCanvasFactory
): string {
  const n = (frame as { height?: number }).height ?? CELL;
  const target = CELL * scale;
  const { ctx, canvas } = factory(target, target);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(frame, 0, 0, n, n, 0, 0, target, target);
  return canvas.toDataURL("image/png");
}

/** 주어진 id들의 미니미를 병렬 로드. 실패는 건별로 조용히 폴백. */
export async function loadMinimisFor(ids: string[], io: MinimiCacheIo = {}): Promise<void> {
  const decode = io.decode ?? decodeMinimi;
  const toPreviewUrl = io.toPreviewUrl ?? minimiPreviewUrl;
  await Promise.all(
    ids.map(async (id) => {
      try {
        const b64 = await tauriApi.loadMinimi(id);
        if (!b64) return;
        const frame = await decode(b64);
        setMinimiOverride(id, frame);
        useAppStore.getState().setMinimiPreview(id, toPreviewUrl(frame));
      } catch (err) {
        console.warn(`minimiCache: loadMinimi failed for ${id}`, err);
      }
    })
  );
}

/** 시작 로드 + 제거 브리지 설치. bootstrap에서 hydrate 후 1회 호출. */
export function installMinimiCache(io: MinimiCacheIo = {}): () => void {
  void loadMinimisFor(agentsNeedingMinimis(useAppStore.getState().agents), io);

  let prevIds = new Set(Object.keys(useAppStore.getState().agents));
  const unsub = useAppStore.subscribe(
    (s) => s.agents,
    (agents) => {
      const nextIds = new Set(Object.keys(agents));
      for (const id of prevIds) {
        if (!nextIds.has(id)) {
          void tauriApi
            .deleteMinimi(id)
            .catch((err) => console.warn(`minimiCache: deleteMinimi failed for ${id}`, err));
          clearMinimiOverride(id);
          useAppStore.getState().removeMinimiPreview(id);
        }
      }
      prevIds = nextIds;
    }
  );
  return unsub;
}
