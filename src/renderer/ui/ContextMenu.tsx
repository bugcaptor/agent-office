// src/renderer/ui/ContextMenu.tsx
//
// 범용 인앱 컨텍스트 메뉴. 항목 배열을 받아 커서 위치에 fixed 렌더,
// 뷰포트 경계는 호버 카드와 동일한 clampCardPosition으로 당긴다. 외부
// mousedown/Escape로 닫힘. 추후 "세션 종료" 등 항목 추가를 전제로 한 구조.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { clampCardPosition } from "../portrait/AgentHoverCard";
import "./contextMenu.css";

const MENU_MARGIN = 4;

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  /** true면 회색 표시 + 클릭 무시(메뉴도 닫히지 않음). */
  disabled?: boolean;
}

export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPos(
      clampCardPosition(
        x,
        y,
        rect.width,
        rect.height,
        window.innerWidth,
        window.innerHeight,
        MENU_MARGIN
      )
    );
  }, [x, y]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="context-menu"
      role="menu"
      style={{
        left: pos ? pos.x : x,
        top: pos ? pos.y : y,
        visibility: pos ? "visible" : "hidden",
      }}
    >
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          role="menuitem"
          className="context-menu-item"
          disabled={item.disabled}
          onClick={() => {
            item.onSelect();
            onClose();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
