// src/renderer/sprite/spriteCache.ts
//
// 커스텀 스프라이트 캐시 배선(설계 rC) — portraitCache를 미러링. 앱 시작 시
// spriteUpdatedAt이 있는 에이전트의 시트를 병렬 로드해 디코드된 캔버스를
// spriteOverrides에, 프리뷰 dataURL을 스토어에 채운다. 디코드/프리뷰 생성은
// io 주입점으로 분리해 node 환경 테스트를 가능하게 한다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { AgentProfile } from "../store/types";
import { CELL } from "../office/gen/compositor";
import {
  setSpriteOverride,
  clearSpriteOverride,
} from "../office/gen/spriteOverrides";
import {
  SHEET_W,
  SHEET_H,
  SHEET_COLS,
  detectSheet,
  defaultSpriteCanvasFactory,
  type SpriteCanvasFactory,
} from "./spriteNormalize";

export interface SpriteCacheIo {
  decode?: (b64: string) => Promise<CanvasImageSource>;
  toPreviewUrl?: (sheet: CanvasImageSource) => string;
}

/** spriteUpdatedAt이 있는(=커스텀 존재) 에이전트 id 목록. 순수. */
export function agentsNeedingSprites(
  agents: Record<string, AgentProfile>
): string[] {
  return Object.values(agents)
    .filter((a) => a.spriteUpdatedAt != null)
    .map((a) => a.id);
}

/** 디코드 캔버스 크기 결정(순수, detectSheet 위임). 256 초과 시트는 여기서
 *  이미 다운스케일된 목표 크기(det.n이 256으로 클램프됨)를 돌려주고, 예상 밖
 *  크기는 레거시 64×16 폴백을 돌려준다. jsdom이 실제 PNG를 디코드 못 해
 *  decodeSheet 자체는 유닛 테스트 불가하므로, 이 순수 함수로 분기를 검증한다. */
export function sheetCanvasDims(w: number, h: number): { w: number; h: number } {
  const det = detectSheet(w, h);
  return det.kind === "sheet"
    ? { w: SHEET_COLS * det.n, h: det.n }
    : { w: SHEET_W, h: SHEET_H };
}

/** base64 PNG → 4N×N 캔버스로 디코드. 저장물은 정규화된 시트이므로 원본 해상도를
 *  보존하되(N=min(h,256)) 256 초과 시 nearest 다운스케일, 예상 밖 크기는 64×16 폴백. */
function decodeSheet(b64: string): Promise<CanvasImageSource> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const { w, h } = sheetCanvasDims(img.naturalWidth, img.naturalHeight);
      const canvas = document.createElement("canvas");
      const ctx = canvas.getContext("2d")!;
      // 캔버스 리사이즈가 2D 컨텍스트 상태를 리셋하므로, 크기 설정 후에 플래그를 세운다.
      canvas.width = w;
      canvas.height = h;
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(img, 0, 0, w, h);
      resolve(canvas);
    };
    img.onerror = () => reject(new Error("sprite sheet decode failed"));
    img.src = `data:image/png;base64,${b64}`;
  });
}

/** 시트의 idle0 셀(N×N)을 CELL*scale px로 확대한 PNG dataURL(프로필/탭/호버 프리뷰용).
 *  배율은 (목표px = CELL*scale) / N으로 결정되어 셀 크기와 무관하게 겉보기 크기를 유지. */
export function sheetPreviewUrl(
  sheet: CanvasImageSource,
  scale = 6,
  factory: SpriteCanvasFactory = defaultSpriteCanvasFactory
): string {
  const n = (sheet as { height?: number }).height ?? CELL;
  const target = CELL * scale;
  const { ctx, canvas } = factory(target, target);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(sheet, 0, 0, n, n, 0, 0, target, target);
  return canvas.toDataURL("image/png");
}

/** 주어진 id들의 시트를 병렬 로드. 실패는 건별로 조용히 폴백. */
export async function loadSpritesFor(
  ids: string[],
  io: SpriteCacheIo = {}
): Promise<void> {
  const decode = io.decode ?? decodeSheet;
  const toPreviewUrl = io.toPreviewUrl ?? sheetPreviewUrl;
  await Promise.all(
    ids.map(async (id) => {
      try {
        const b64 = await tauriApi.loadSprite(id);
        if (!b64) return;
        const sheet = await decode(b64);
        setSpriteOverride(id, sheet);
        useAppStore.getState().setSpritePreview(id, toPreviewUrl(sheet));
      } catch (err) {
        console.warn(`spriteCache: loadSprite failed for ${id}`, err);
      }
    })
  );
}

/** 시작 로드 + 제거 브리지 설치. bootstrap에서 hydrate 후 1회 호출. */
export function installSpriteCache(io: SpriteCacheIo = {}): () => void {
  void loadSpritesFor(agentsNeedingSprites(useAppStore.getState().agents), io);

  let prevIds = new Set(Object.keys(useAppStore.getState().agents));
  const unsub = useAppStore.subscribe(
    (s) => s.agents,
    (agents) => {
      const nextIds = new Set(Object.keys(agents));
      for (const id of prevIds) {
        if (!nextIds.has(id)) {
          void tauriApi
            .deleteSprite(id)
            .catch((err) =>
              console.warn(`spriteCache: deleteSprite failed for ${id}`, err)
            );
          clearSpriteOverride(id);
          useAppStore.getState().removeSpritePreview(id);
        }
      }
      prevIds = nextIds;
    }
  );
  return unsub;
}
