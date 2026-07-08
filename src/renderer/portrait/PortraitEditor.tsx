// src/renderer/portrait/PortraitEditor.tsx
//
// 임의 이미지 업로드 -> 3:4 고정 프레임 위 드래그(위치)/휠(줌) 크롭 -> 240x320
// PNG로 재인코딩해 저장하는 모달. 외부 라이브러리 없이 canvas + 포인터 이벤트로
// 구현한다. 크롭 좌표 변환/레트로 수학은 순수 모듈(cropMath/retroFilter)에
// 위임하고 여기서는 canvas 드로잉·상호작용·저장 배선만 담당한다.
import { useCallback, useEffect, useRef, useState } from "react";
import "./portrait.css";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { pngBase64ToDataUrl } from "./portraitCache";
import {
  initialCoverView,
  viewToSourceRect,
  zoomAt,
  panBy,
  type CropView,
} from "./cropMath";
import { RETRO_DOWNSCALE, RETRO_LEVELS, posterizeRgba } from "./retroFilter";

const OUT_W = 240;
const OUT_H = 320;
// 편집용 미리보기 프레임(출력과 동일 3:4 비율, 화면 표시 크기).
const FRAME_W = 240;
const FRAME_H = 320;

export function PortraitEditor({
  agentId,
  onClose,
}: {
  agentId: string;
  onClose: () => void;
}) {
  const setPortrait = useAppStore((s) => s.setPortrait);
  const updateAgent = useAppStore((s) => s.updateAgent);

  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const viewRef = useRef<CropView | null>(null);
  const dragRef = useRef<{ x: number; y: number } | null>(null);

  const [hasImage, setHasImage] = useState(false);
  const [retro, setRetro] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // 현재 뷰를 프레임 캔버스에 그린다(레트로 미리보기 반영).
  const redraw = useCallback(() => {
    const canvas = canvasRef.current;
    const img = imgRef.current;
    const view = viewRef.current;
    if (!canvas || !img || !view) return;
    const ctx = canvas.getContext("2d")!;
    ctx.imageSmoothingEnabled = false;
    ctx.clearRect(0, 0, FRAME_W, FRAME_H);
    const r = viewToSourceRect(view, FRAME_W, FRAME_H);
    ctx.drawImage(img, r.sx, r.sy, r.sw, r.sh, 0, 0, FRAME_W, FRAME_H);
    if (retro) applyRetroInPlace(ctx);
  }, [retro]);

  useEffect(() => {
    redraw();
  }, [redraw]);

  // 휠 줌: React onWheel(패시브)로는 preventDefault가 안 먹어 배경 스크롤이
  // 함께 발생한다. 캔버스에 네이티브 리스너를 { passive: false }로 붙여 막는다.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const handleWheel = (e: WheelEvent) => {
      const view = viewRef.current;
      if (!view) return;
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      // 화면 표시 크기 -> 프레임 좌표로 환산한 앵커.
      const px = ((e.clientX - rect.left) / rect.width) * FRAME_W;
      const py = ((e.clientY - rect.top) / rect.height) * FRAME_H;
      const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
      viewRef.current = zoomAt(view, factor, px, py);
      redraw();
    };
    canvas.addEventListener("wheel", handleWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", handleWheel);
  }, [redraw]);

  const onFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setError(null);
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      imgRef.current = img;
      viewRef.current = initialCoverView(
        img.naturalWidth,
        img.naturalHeight,
        FRAME_W,
        FRAME_H
      );
      setHasImage(true);
      redraw();
      URL.revokeObjectURL(url);
    };
    img.onerror = () => {
      setError("이미지를 읽을 수 없습니다. 다른 파일을 선택하세요.");
      URL.revokeObjectURL(url);
    };
    img.src = url;
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (!hasImage) return;
    dragRef.current = { x: e.clientX, y: e.clientY };
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const drag = dragRef.current;
    const view = viewRef.current;
    if (!drag || !view) return;
    viewRef.current = panBy(view, e.clientX - drag.x, e.clientY - drag.y);
    dragRef.current = { x: e.clientX, y: e.clientY };
    redraw();
  };
  const onPointerUp = (e: React.PointerEvent) => {
    dragRef.current = null;
    (e.target as HTMLElement).releasePointerCapture?.(e.pointerId);
  };
  const onSave = async () => {
    const img = imgRef.current;
    const view = viewRef.current;
    if (!img || !view) return;
    setSaving(true);
    setError(null);
    try {
      const base64 = renderOutputPng(img, view, retro);
      await tauriApi.savePortrait(agentId, base64);
      setPortrait(agentId, pngBase64ToDataUrl(base64));
      updateAgent(agentId, { portraitUpdatedAt: Date.now() });
      onClose();
    } catch (err) {
      console.warn("PortraitEditor: savePortrait failed", err);
      setError("저장에 실패했습니다.");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="modal-backdrop"
      // Target-guard (mirrors ProfileDialog's backdrop / TerminalOverlay
      // commit 7986f3d) rather than relying solely on the panel's
      // stopPropagation below: only a mousedown whose target is this
      // backdrop element itself (not a bubbled descendant event) closes
      // the editor.
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="pixel-panel portrait-editor"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 className="pixel-title">초상 편집</h2>
        <input type="file" accept="image/*" onChange={onFile} />
        <div className="portrait-crop-frame">
          <canvas
            ref={canvasRef}
            width={FRAME_W}
            height={FRAME_H}
            className="portrait-crop-canvas"
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
          />
          {!hasImage && (
            <div className="portrait-crop-hint">이미지를 선택하세요</div>
          )}
        </div>
        <label className="portrait-retro-toggle">
          <input
            type="checkbox"
            checked={retro}
            onChange={(e) => setRetro(e.target.checked)}
          />
          레트로 픽셀 필터
        </label>
        {error && <p className="portrait-error">{error}</p>}
        <div className="dialog-actions">
          <button
            className="pixel-btn primary"
            disabled={!hasImage || saving}
            onClick={onSave}
          >
            저장
          </button>
          <button className="pixel-btn" onClick={onClose}>
            취소
          </button>
        </div>
      </div>
    </div>
  );
}

/** ctx 내용을 1/4 해상도로 nearest 다운→업스케일 + 채널 포스터라이즈(제자리). */
function applyRetroInPlace(
  ctx: CanvasRenderingContext2D
): void {
  const { w, h } = RETRO_DOWNSCALE;
  const small = document.createElement("canvas");
  small.width = w;
  small.height = h;
  const sctx = small.getContext("2d")!;
  sctx.imageSmoothingEnabled = false;
  sctx.drawImage(ctx.canvas, 0, 0, w, h);
  const id = sctx.getImageData(0, 0, w, h);
  const posterized = posterizeRgba(id.data, RETRO_LEVELS);
  for (let i = 0; i < id.data.length; i++) id.data[i] = posterized[i];
  sctx.putImageData(id, 0, 0);
  ctx.imageSmoothingEnabled = false;
  ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height);
  ctx.drawImage(small, 0, 0, w, h, 0, 0, ctx.canvas.width, ctx.canvas.height);
}

/** 현재 뷰를 240x320 출력 캔버스에 렌더해 PNG base64(헤더 없음) 반환. */
function renderOutputPng(
  img: HTMLImageElement,
  view: CropView,
  retro: boolean
): string {
  const out = document.createElement("canvas");
  out.width = OUT_W;
  out.height = OUT_H;
  const ctx = out.getContext("2d")!;
  ctx.imageSmoothingEnabled = false;
  const r = viewToSourceRect(view, OUT_W, OUT_H);
  ctx.drawImage(img, r.sx, r.sy, r.sw, r.sh, 0, 0, OUT_W, OUT_H);
  if (retro) applyRetroInPlace(ctx);
  // "data:image/png;base64,XXXX" -> "XXXX" (백엔드는 헤더 없는 base64 기대).
  return out.toDataURL("image/png").split(",", 2)[1] ?? "";
}
