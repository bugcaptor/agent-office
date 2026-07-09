// src/renderer/desk/DeskAssignMenu.tsx
//
// 책상 클릭 → 주인 지정 컨텍스트 메뉴 (C 레이어).
//
// 오피스 씬(B)이 빈 책상 히트영역의 pointertap을 officeBus.emitDeskClicked
// (deskIndex, 화면좌표)로 발행하면, 이 컴포넌트가 커서 위치에 범용
// ContextMenu를 띄운다. 항목 선택은 appStore.assignDesk로 흘러가
// agents[..].assignedDeskIndex에 기록되고(책상당 주인 1명 보장은 액션이
// 담당), persist가 프로필과 함께 자동 저장한다. 지정된 적 없는 책상은
// 자리 없는 에이전트가 자동(해시) 배정으로 선점할 수 있다 — 그 규칙은
// office/map/deskAssignment.ts 소관.
import { useEffect, useState } from "react";

import { officeBus } from "../ipc/sessionBridge";
import { useAppStore } from "../store/appStore";
import { ContextMenu, type ContextMenuItem } from "../ui/ContextMenu";

interface DeskClickTarget {
  deskIndex: number;
  x: number;
  y: number;
}

export function DeskAssignMenu() {
  const [target, setTarget] = useState<DeskClickTarget | null>(null);
  const agents = useAppStore((s) => s.agents);
  const agentOrder = useAppStore((s) => s.agentOrder);
  const assignDesk = useAppStore((s) => s.assignDesk);

  useEffect(
    () => officeBus.onDeskClicked((deskIndex, x, y) => setTarget({ deskIndex, x, y })),
    [],
  );

  if (!target) return null;
  const { deskIndex } = target;
  const ownerId = agentOrder.find((id) => agents[id]?.assignedDeskIndex === deskIndex);

  const items: ContextMenuItem[] = [
    // 헤더(선택 불가): 어느 책상인지 표시.
    { label: `${deskIndex + 1}번 책상 주인`, onSelect: () => {}, disabled: true },
    ...agentOrder
      .filter((id) => agents[id])
      .map((id) => ({
        label: id === ownerId ? `✓ ${agents[id].name}` : agents[id].name,
        onSelect: () => assignDesk(deskIndex, id),
      })),
    {
      label: "지정 해제",
      onSelect: () => assignDesk(deskIndex, null),
      disabled: ownerId === undefined,
    },
  ];

  return <ContextMenu x={target.x} y={target.y} items={items} onClose={() => setTarget(null)} />;
}
