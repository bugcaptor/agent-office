// src/renderer/profile/ColorPickerDialog.tsx
//
// 키 컬러 하나를 고르는 모달(kbm #2fj). 프로필 편집창의 색 칩을 누르면 뜬다.
//
// 외부 라이브러리 없이 CSS 그러데이션 + 포인터 이벤트로 만든다(앱 전체가 그
// 방침이고, 캔버스 픽셀을 읽지 않아 테스트/성능 부담도 없다):
//   · 채도·명도 사각형 — 배경은 [색상 단색] 위에 [흰→투명] 가로,
//     [투명→검정] 세로를 겹친 것. x가 채도, y가 명도(위가 밝다).
//   · 색상 슬라이더 — 무지개 가로 그러데이션. x가 색상(0..360).
// 좌표 ↔ 색 변환은 전부 순수 모듈(`colorPickerMath`)에 있다.
//
// "기본값으로"는 오버라이드를 지우는 것이지 기본색을 오버라이드로 박는 것이
// 아니다 — 나중에 시드나 아키타입을 바꾸면 색이 따라 움직여야 하기 때문이다.
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import {
  PRESET_SWATCHES,
  hexToHsv,
  hsvToHex,
  normalizeHex,
  ratioAt,
  sameHex,
  type Hsv,
} from "./colorPickerMath";

/** 채도·명도 사각형과 색상 슬라이더의 표시 크기(px). CSS와 짝을 맞춘다. */
const SV_W = 240;
const SV_H = 160;

export interface ColorPickerDialogProps {
  /** 다이얼로그 제목에 쓰는 슬롯 이름(예: "머리"). */
  label: string;
  /** 열 때의 색("#rrggbb"). 오버라이드가 없으면 시드 기본색이 들어온다.
   *  초기값으로만 읽으므로, 부모는 슬롯이 바뀌면 `key`로 새로 마운트시킨다. */
  value: string;
  /** 이 슬롯의 시드 기본색. "기본값으로" 버튼의 미리보기 겸 되돌림 목표. */
  defaultValue: string;
  /** 지금 오버라이드가 걸려 있는가 — false면 "기본값으로"를 비활성화한다. */
  overridden: boolean;
  /** 색 확정. 부모가 draft에 반영한다. */
  onApply: (hex: string) => void;
  /** 오버라이드 해제(시드 기본색으로 되돌림). */
  onReset: () => void;
  onClose: () => void;
}

export function ColorPickerDialog({
  label,
  value,
  defaultValue,
  overridden,
  onApply,
  onReset,
  onClose,
}: ColorPickerDialogProps) {
  const { t } = useTranslation("profile");
  // HSV를 상태로 들고 hex는 그때그때 파생한다 — 채도 0/명도 0에서도 색상이
  // 살아 있어야 슬라이더가 제자리를 지킨다(hex만 들고 있으면 정보가 사라진다).
  const [hsv, setHsv] = useState<Hsv>(() => hexToHsv(value));
  // hex 입력창은 타이핑 중간값("#1a2")을 그대로 보여야 해서 별도 문자열 상태다.
  const [hexText, setHexText] = useState<string>(() => normalizeHex(value) ?? "#000000");
  const svRef = useRef<HTMLDivElement | null>(null);
  const hueRef = useRef<HTMLDivElement | null>(null);

  useEscapeToClose(true, onClose);

  const hex = hsvToHex(hsv);

  /** HSV를 바꾸면서 hex 입력창도 따라 맞춘다(양방향 편집의 한쪽 방향). */
  const setColor = (next: Hsv) => {
    setHsv(next);
    setHexText(hsvToHex(next));
  };

  const pickFromSv = (e: React.PointerEvent) => {
    const box = svRef.current?.getBoundingClientRect();
    if (!box) return;
    setColor({
      h: hsv.h,
      s: ratioAt(e.clientX, box.left, box.width),
      v: 1 - ratioAt(e.clientY, box.top, box.height),
    });
  };

  const pickFromHue = (e: React.PointerEvent) => {
    const box = hueRef.current?.getBoundingClientRect();
    if (!box) return;
    setColor({ ...hsv, h: ratioAt(e.clientX, box.left, box.width) * 360 });
  };

  /** 드래그 = 포인터 캡처 + move 동안 계속 집기. 창 밖으로 나가도 이어진다. */
  const dragHandlers = (pick: (e: React.PointerEvent) => void) => ({
    onPointerDown: (e: React.PointerEvent) => {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      pick(e);
    },
    onPointerMove: (e: React.PointerEvent) => {
      if (e.buttons & 1) pick(e);
    },
  });

  const onHexInput = (raw: string) => {
    setHexText(raw);
    const norm = normalizeHex(raw);
    // 형식이 될 때만 사각형/슬라이더를 옮긴다. 이전 색상(hsv.h)을 유지해
    // 무채색을 찍어도 색상 슬라이더가 빨강으로 튀지 않게 한다.
    if (norm) setHsv(hexToHsv(norm, hsv.h));
  };

  return (
    <div
      className="modal-backdrop"
      // ProfileDialog / PortraitEditor와 같은 규약: 이 배경 자체를 누른
      // mousedown만 닫는다(자손에서 버블링된 이벤트는 무시).
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="pixel-panel color-picker" onMouseDown={(e) => e.stopPropagation()}>
        <h2 className="pixel-title">{t("color.title", { label })}</h2>

        <div
          ref={svRef}
          className="cp-sv"
          role="application"
          aria-label={t("color.svAriaLabel", { label })}
          style={{
            width: SV_W,
            height: SV_H,
            backgroundColor: hsvToHex({ h: hsv.h, s: 1, v: 1 }),
          }}
          {...dragHandlers(pickFromSv)}
        >
          <div className="cp-sv-white" />
          <div className="cp-sv-black" />
          <div
            className="cp-thumb"
            style={{ left: `${hsv.s * 100}%`, top: `${(1 - hsv.v) * 100}%`, background: hex }}
          />
        </div>

        <div
          ref={hueRef}
          className="cp-hue"
          role="application"
          aria-label={t("color.hueAriaLabel", { label })}
          style={{ width: SV_W }}
          {...dragHandlers(pickFromHue)}
        >
          <div
            className="cp-thumb cp-thumb-hue"
            style={{
              left: `${(hsv.h / 360) * 100}%`,
              background: hsvToHex({ h: hsv.h, s: 1, v: 1 }),
            }}
          />
        </div>

        <div className="cp-row">
          <span className="cp-preview" style={{ background: hex }} aria-hidden />
          <label className="cp-hex">
            <span className="form-label-text">HEX</span>
            <input
              value={hexText}
              spellCheck={false}
              maxLength={7}
              onChange={(e) => onHexInput(e.target.value)}
            />
          </label>
        </div>

        <div className="cp-presets" aria-label={t("color.presets")}>
          {PRESET_SWATCHES.map((row, ri) => (
            <div className="cp-preset-row" key={ri}>
              {row.map((c) => (
                <button
                  key={c}
                  type="button"
                  className={sameHex(c, hex) ? "cp-swatch cp-swatch-on" : "cp-swatch"}
                  style={{ background: c }}
                  title={c}
                  aria-label={c}
                  onClick={() => setColor(hexToHsv(c, hsv.h))}
                />
              ))}
            </div>
          ))}
        </div>

        <div className="cp-actions">
          <button
            className="pixel-btn"
            disabled={!overridden}
            title={t("color.resetTitle", { hex: defaultValue })}
            onClick={() => {
              onReset();
              onClose();
            }}
          >
            {t("color.reset")}
          </button>
          <span className="cp-spacer" />
          <button className="pixel-btn" onClick={onClose}>
            {t("color.cancel")}
          </button>
          <button
            className="pixel-btn"
            onClick={() => {
              onApply(hex);
              onClose();
            }}
          >
            {t("color.apply")}
          </button>
        </div>
      </div>
    </div>
  );
}
