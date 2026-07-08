// src/renderer/sprite/SpriteEditor.tsx
//
// 커스텀 픽셀 아트 업로드 모달(설계 C/D). PortraitEditor의 크롭/줌 UX를 1:1
// 프레임으로 재사용한다. 디코드 이미지를 소스 캔버스로 래스터화해(선택 시 배경
// 투명화 1회 적용) 크롭/시트 두 경로가 동일 소스를 소비한다. 4N×N 시트는 셀
// 해상도 보존 패스스루, 그 외는 크롭 → N×N nearest → 4프레임 시트. N ∈ [16,256].
import { useCallback, useEffect, useRef, useState } from "react";
import "../portrait/portrait.css";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { setSpriteOverride } from "../office/gen/spriteOverrides";
import {
  initialCoverView,
  viewToSourceRect,
  zoomAt,
  panBy,
  type CropView,
} from "../portrait/cropMath";
import {
  applyBackgroundKey,
  dataUrlToBase64,
  detectSheet,
  isFullyOpaque,
  normalizeCrop,
  normalizeSheet,
  type SpriteCanvas,
} from "./spriteNormalize";
import { sheetPreviewUrl } from "./spriteCache";
import { CELL } from "../office/gen/compositor";

/** 1:1 크롭 표시 프레임(화면 px). */
const FRAME = 192;
/** 스프라이트 상한(백엔드 MAX_SPRITE_BYTES=1MiB와 동일) — base64 팽창률 4/3 반영 가드. */
const MAX_SPRITE_B64_LEN = Math.ceil((1024 * 1024 * 4) / 3);

type Mode = "empty" | "crop" | "sheet";

export function SpriteEditor({
  agentId,
  onClose,
  initialImage,
}: {
  agentId: string;
  onClose: () => void;
  /** PixelLab 생성 결과 등 프리로드할 data URL. 지정 시 업로드 없이 시작. */
  initialImage?: string;
}) {
  const setSpritePreview = useAppStore((s) => s.setSpritePreview);
  const updateAgent = useAppStore((s) => s.updateAgent);

  const cropCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const previewCanvasRef = useRef<HTMLCanvasElement | null>(null);
  /** 디코드 원본(투명화 재적용 시 재래스터화용). */
  const rawImgRef = useRef<HTMLImageElement | null>(null);
  /** 크롭/시트 공통 소스 캔버스(투명화 적용 반영). */
  const srcRef = useRef<HTMLCanvasElement | null>(null);
  const dimsRef = useRef<{ w: number; h: number } | null>(null);
  const viewRef = useRef<CropView | null>(null);
  const dragRef = useRef<{ x: number; y: number } | null>(null);
  /** sheet 모드: 정규화해 둔 4N×N 시트와 셀 크기 N. */
  const sheetRef = useRef<SpriteCanvas | null>(null);
  const sheetNRef = useRef<number>(CELL);

  const [mode, setMode] = useState<Mode>("empty");
  const [transparentBg, setTransparentBg] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // 크롭 캔버스 + 16×16 라이브 프리뷰 다시 그리기.
  const redraw = useCallback(() => {
    const src = srcRef.current;
    const view = viewRef.current;
    if (mode === "sheet") {
      const preview = previewCanvasRef.current;
      const sheet = sheetRef.current;
      if (!preview || !sheet) return;
      const n = sheetNRef.current;
      const pctx = preview.getContext("2d")!;
      pctx.imageSmoothingEnabled = false;
      pctx.clearRect(0, 0, CELL, CELL);
      pctx.drawImage(sheet, 0, 0, n, n, 0, 0, CELL, CELL);
      return;
    }
    if (!src || !view) return;
    const canvas = cropCanvasRef.current;
    if (canvas) {
      const ctx = canvas.getContext("2d")!;
      ctx.imageSmoothingEnabled = false;
      ctx.clearRect(0, 0, FRAME, FRAME);
      const r = viewToSourceRect(view, FRAME, FRAME);
      ctx.drawImage(src, r.sx, r.sy, r.sw, r.sh, 0, 0, FRAME, FRAME);
    }
    const preview = previewCanvasRef.current;
    if (preview) {
      const pctx = preview.getContext("2d")!;
      pctx.imageSmoothingEnabled = false;
      pctx.clearRect(0, 0, CELL, CELL);
      const r = viewToSourceRect(view, FRAME, FRAME);
      pctx.drawImage(src, r.sx, r.sy, r.sw, r.sh, 0, 0, CELL, CELL);
    }
  }, [mode]);

  useEffect(() => {
    redraw();
  }, [redraw]);

  // 휠 줌: PortraitEditor와 동일하게 non-passive 네이티브 리스너로 preventDefault.
  useEffect(() => {
    const canvas = cropCanvasRef.current;
    if (!canvas) return;
    const handleWheel = (e: WheelEvent) => {
      const view = viewRef.current;
      if (!view) return;
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const px = ((e.clientX - rect.left) / rect.width) * FRAME;
      const py = ((e.clientY - rect.top) / rect.height) * FRAME;
      const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
      viewRef.current = zoomAt(view, factor, px, py);
      redraw();
    };
    canvas.addEventListener("wheel", handleWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", handleWheel);
  }, [redraw]);

  /** rawImg를 소스 캔버스로 래스터화하고(키 적용 여부에 따라) 시트를 재정규화. */
  const rebuildSource = useCallback((applyKey: boolean) => {
    const img = rawImgRef.current;
    const dims = dimsRef.current;
    if (!img || !dims) return;
    const src = document.createElement("canvas");
    src.width = dims.w;
    src.height = dims.h;
    const sctx = src.getContext("2d")!;
    sctx.imageSmoothingEnabled = false;
    sctx.drawImage(img, 0, 0);
    if (applyKey) applyBackgroundKey(src);
    srcRef.current = src;
    if (detectSheet(dims.w, dims.h).kind === "sheet") {
      const { sheet, n } = normalizeSheet(src, dims.w, dims.h);
      sheetRef.current = sheet;
      sheetNRef.current = n;
    }
  }, []);

  /** 디코드된 이미지를 에디터 상태로 반영 (파일 업로드/PixelLab 생성 공통). */
  const ingestImage = useCallback(
    (img: HTMLImageElement) => {
      const w = img.naturalWidth;
      const h = img.naturalHeight;
      rawImgRef.current = img;
      dimsRef.current = { w, h };

      // 소스 래스터화 → 완전 불투명일 때만 투명화 기본 체크.
      const probe = document.createElement("canvas");
      probe.width = w;
      probe.height = h;
      const pctx = probe.getContext("2d")!;
      pctx.imageSmoothingEnabled = false;
      pctx.drawImage(img, 0, 0);
      const key = isFullyOpaque(probe);
      setTransparentBg(key);
      rebuildSource(key);

      if (detectSheet(w, h).kind === "sheet") {
        viewRef.current = null;
        setMode("sheet");
      } else {
        viewRef.current = initialCoverView(w, h, FRAME, FRAME);
        sheetRef.current = null;
        setMode("crop");
      }
      redraw();
    },
    [rebuildSource, redraw],
  );

  const onFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setError(null);
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      ingestImage(img);
      URL.revokeObjectURL(url);
    };
    img.onerror = () => {
      setError("이미지를 읽을 수 없습니다. 다른 파일을 선택하세요.");
      URL.revokeObjectURL(url);
    };
    img.src = url;
  };

  // PixelLab 생성 이미지 프리로드. data URL이라 revoke 불필요.
  /** 마운트 시점의 initialImage만 소비 — 마운트 후 prop 변경(늦은 생성 응답의
   * 난입)은 무시한다. 재생성 정상 경로는 에디터가 닫혔다 새로 마운트되므로 영향 없음. */
  const initialImageAtMount = useRef(initialImage).current;
  /** initialImage는 정확히 1회만 소비 — mode 변경으로 ingestImage identity가
   * 바뀌어 효과가 재발화해도 사용자 업로드를 덮어쓰지 않는다. */
  const initialConsumedRef = useRef(false);
  useEffect(() => {
    if (!initialImageAtMount || initialConsumedRef.current) return;
    let cancelled = false;
    const img = new Image();
    img.onload = () => {
      if (cancelled || initialConsumedRef.current) return;
      // 실제 소비 시점에 마킹 — StrictMode 이중 효과(run→cleanup→re-run)에서
      // run#1이 취소돼도 run#2가 로드하고, ingest 후 재발화는 여전히 차단된다.
      initialConsumedRef.current = true;
      ingestImage(img);
    };
    img.onerror = () => {
      // attempt-once: 재발화 시 손상 initialImage 재시도로 업로드 성공 후
      // 스테일 에러가 재출현하는 경로 차단.
      if (cancelled || initialConsumedRef.current) return;
      initialConsumedRef.current = true;
      setError("생성 이미지를 읽을 수 없습니다.");
    };
    img.src = initialImageAtMount;
    return () => {
      cancelled = true;
    };
  }, [initialImageAtMount, ingestImage]);

  const onToggleTransparent = (checked: boolean) => {
    setTransparentBg(checked);
    if (!rawImgRef.current) return; // 이미지 없으면 상태만 변경(캔버스 연산 없음)
    rebuildSource(checked);
    redraw();
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (mode !== "crop") return;
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
    let sheet: SpriteCanvas | null = null;
    if (mode === "sheet") {
      sheet = sheetRef.current;
    } else if (mode === "crop" && srcRef.current && viewRef.current) {
      const rect = viewToSourceRect(viewRef.current, FRAME, FRAME);
      sheet = normalizeCrop(srcRef.current, rect).sheet;
    }
    if (!sheet) return;
    setSaving(true);
    setError(null);
    try {
      const base64 = dataUrlToBase64(sheet.toDataURL("image/png"));
      if (!base64 || base64.length > MAX_SPRITE_B64_LEN) {
        setError("이미지 인코딩에 실패했거나 1MiB 상한을 초과합니다.");
        return;
      }
      await tauriApi.saveSprite(agentId, base64);
      setSpriteOverride(agentId, sheet);
      setSpritePreview(agentId, sheetPreviewUrl(sheet));
      updateAgent(agentId, { spriteUpdatedAt: Date.now() });
      onClose();
    } catch (err) {
      console.warn("SpriteEditor: saveSprite failed", err);
      setError("저장에 실패했습니다.");
    } finally {
      setSaving(false);
    }
  };

  const hasImage = mode !== "empty";
  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="pixel-panel sprite-editor"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 className="pixel-title">픽셀아트 편집</h2>
        <input type="file" accept="image/*" onChange={onFile} />
        <div className="sprite-edit-row">
          {mode !== "sheet" && (
            <div className="sprite-crop-frame">
              <canvas
                ref={cropCanvasRef}
                width={FRAME}
                height={FRAME}
                className="sprite-crop-canvas"
                onPointerDown={onPointerDown}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
              />
              {mode === "empty" && (
                <div className="portrait-crop-hint">이미지를 선택하세요</div>
              )}
            </div>
          )}
          <div className="sprite-preview-box">
            <canvas ref={previewCanvasRef} width={CELL} height={CELL} />
            <span className="sprite-preview-label">16×16 미리보기</span>
          </div>
        </div>
        <label className="sprite-transparent-toggle">
          <input
            type="checkbox"
            checked={transparentBg}
            onChange={(e) => onToggleTransparent(e.target.checked)}
          />
          배경 투명화 (좌상단 픽셀 색 기준)
        </label>
        {mode === "sheet" && (
          <p className="sprite-sheet-note">
            4프레임 시트로 인식했습니다. 셀 해상도를 보존해 저장됩니다.
          </p>
        )}
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
