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
  /** 라벨 좌측에 표시할 아이콘(유니코드 이모지). 미지정이어도 슬롯 폭은 유지. */
  icon?: string;
  /** 파괴적 항목(삭제/퇴근 등). 경고색으로 강조. */
  danger?: boolean;
}

/** 그룹 구분선. items 배열 안에 섞어 배치한다. */
export interface ContextMenuSeparator {
  separator: true;
}

export type ContextMenuEntry = ContextMenuItem | ContextMenuSeparator;

function isSeparator(e: ContextMenuEntry): e is ContextMenuSeparator {
  return "separator" in e && e.separator === true;
}

/**
 * 구분선 정규화: 맨 앞/맨 뒤, 그리고 연속된 구분선을 제거한다.
 * 소비처가 그룹 사이에 무심코 넣은 중복 구분선을 렌더 단계에서 흡수해
 * 항목이 disabled로 통째로 빠져도 빈 구분선이 남지 않게 한다.
 */
function normalizeEntries(items: ContextMenuEntry[]): ContextMenuEntry[] {
  const out: ContextMenuEntry[] = [];
  for (const e of items) {
    if (isSeparator(e)) {
      if (out.length === 0) continue; // 맨 앞
      if (isSeparator(out[out.length - 1])) continue; // 연속
      out.push(e);
    } else {
      out.push(e);
    }
  }
  // 맨 뒤 구분선 제거
  while (out.length > 0 && isSeparator(out[out.length - 1])) out.pop();
  return out;
}

export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: ContextMenuEntry[];
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
      {normalizeEntries(items).map((entry, i) =>
        isSeparator(entry) ? (
          <div
            key={`sep-${i}`}
            className="context-menu-separator"
            role="separator"
          />
        ) : (
          <button
            key={entry.label}
            type="button"
            role="menuitem"
            className={
              "context-menu-item" + (entry.danger ? " context-menu-item-danger" : "")
            }
            disabled={entry.disabled}
            onClick={() => {
              entry.onSelect();
              onClose();
            }}
          >
            <span className="context-menu-icon" aria-hidden="true">
              {entry.icon ?? ""}
            </span>
            <span className="context-menu-label">{entry.label}</span>
          </button>
        )
      )}
    </div>
  );
}
