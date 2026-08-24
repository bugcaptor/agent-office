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

/**
 * 섹션 헤더. 구분선만으로는 어느 항목이 어느 그룹인지 알 수 없을 때(예:
 * "풍경"/"테마"처럼 라벨만으론 구분 안 되는 두 그룹) 클릭 불가 표제로 쓴다.
 * 클릭/포커스 대상이 아니므로 role/키보드 순회에서 빠진다.
 */
export interface ContextMenuHeader {
  header: string;
}

export type ContextMenuEntry = ContextMenuItem | ContextMenuSeparator | ContextMenuHeader;

function isSeparator(e: ContextMenuEntry): e is ContextMenuSeparator {
  return "separator" in e && e.separator === true;
}

function isHeader(e: ContextMenuEntry): e is ContextMenuHeader {
  return "header" in e;
}

/**
 * 항목 정규화:
 * - 헤더 뒤(다음 헤더 전까지)에 실제 항목이 하나도 없으면 그 헤더를 버린다
 *   (섹션이 통째로 disabled/제외돼도 표제만 남지 않게).
 * - 구분선은 맨 앞/맨 뒤, 연속된 것, 그리고 헤더 바로 앞/뒤에 붙은 것을
 *   제거한다 — 헤더 자체가 이미 시각적 구분 역할을 하므로 옆에 구분선이
 *   남을 필요가 없다.
 * 소비처가 그룹 사이에 무심코 넣은 중복 구분선/빈 섹션을 렌더 단계에서
 * 흡수해, 항목이 disabled/조건부로 통째로 빠져도 빈 구분선·헤더가 남지
 * 않게 한다.
 */
function normalizeEntries(items: ContextMenuEntry[]): ContextMenuEntry[] {
  // 1) 항목이 하나도 없는(다음 헤더 전까지) 헤더는 버린다.
  const withoutOrphanHeaders: ContextMenuEntry[] = [];
  for (let i = 0; i < items.length; i++) {
    const e = items[i];
    if (isHeader(e)) {
      let hasItem = false;
      for (let j = i + 1; j < items.length; j++) {
        const next = items[j];
        if (isHeader(next)) break; // 다음 섹션 시작 — 이 섹션은 여기까지
        if (!isSeparator(next)) {
          hasItem = true;
          break;
        }
      }
      if (!hasItem) continue;
    }
    withoutOrphanHeaders.push(e);
  }

  // 2) 구분선 정규화 (맨 앞/연속/헤더 인접) + 헤더 직전 구분선 제거.
  const out: ContextMenuEntry[] = [];
  for (const e of withoutOrphanHeaders) {
    if (isSeparator(e)) {
      if (out.length === 0) continue; // 맨 앞
      const prev = out[out.length - 1];
      if (isSeparator(prev) || isHeader(prev)) continue; // 연속, 또는 헤더 직후
      out.push(e);
    } else {
      if (isHeader(e) && out.length > 0 && isSeparator(out[out.length - 1])) {
        out.pop(); // 헤더 직전 구분선 제거
      }
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
        isHeader(entry) ? (
          // 클릭 불가·비대화형 표제 — role/키보드 포커스 대상에서 뺀다.
          <div key={`hdr-${i}`} className="context-menu-header">
            {entry.header}
          </div>
        ) : isSeparator(entry) ? (
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
