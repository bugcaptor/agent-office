// src/renderer/portrait/AgentHoverCard.tsx
//
// 오피스 캔버스 위 HTML 오버레이 호버 카드. officeBus의
// agentHoverChanged를 구독해, 150ms 지연 후 등장 위치를 고정한 채 초상+이름+역할을
// 보여준다. pointerout/클릭(=emitAgentClicked가 null 방출) 시 즉시 숨김. 이미지 폴백
// 순서: 초상(portraitUrl) -> 커스텀 스프라이트 프리뷰(spritePreviewUrl) ->
// generateSpritePreview(절차 생성).
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import "./portrait.css";
import { officeBus } from "../ipc/sessionBridge";
import { useAppStore } from "../store/appStore";
import { generateSpritePreview } from "../office/gen/characterFactory";
import { resolveArchetype } from "../office/gen/archetypes";

const SHOW_DELAY_MS = 150;
const CARD_MARGIN = 8;

interface HoverState {
  agentId: string;
  x: number;
  y: number;
}

/**
 * 카드 위치를 뷰포트 안으로 clamp한다(순수 함수, DOM 비의존).
 * 우/하단으로 넘치면 카드가 뷰포트 안에 들어오도록 당기고, 그래도(카드가
 * 뷰포트보다 큰 경우) margin 아래로는 내려가지 않도록 바닥을 둔다.
 */
export function clampCardPosition(
  rawX: number,
  rawY: number,
  cardW: number,
  cardH: number,
  viewportW: number,
  viewportH: number,
  margin: number
): { x: number; y: number } {
  const maxX = viewportW - cardW - margin;
  const maxY = viewportH - cardH - margin;
  const x = Math.max(margin, Math.min(rawX, maxX));
  const y = Math.max(margin, Math.min(rawY, maxY));
  return { x, y };
}

export function AgentHoverCard() {
  const [hover, setHover] = useState<HoverState | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cardRef = useRef<HTMLDivElement | null>(null);
  // 화면에 실제로 그려질 위치(clamp 적용 후). 아직 측정 전이면 null -> 카드는
  // visibility: hidden으로 숨겨 offscreen 원위치가 잠깐 보이는 점프를 막는다.
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);

  const agent = useAppStore((s) => (hover ? s.agents[hover.agentId] : undefined));
  const portraitUrl = useAppStore((s) =>
    hover ? s.portraits[hover.agentId] : undefined
  );
  const spritePreviewUrl = useAppStore((s) =>
    hover ? s.spritePreviews[hover.agentId] : undefined
  );

  useLayoutEffect(() => {
    if (!hover) {
      setPos(null);
      return;
    }
    const el = cardRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPos(
      clampCardPosition(
        hover.x + 12,
        hover.y + 12,
        rect.width,
        rect.height,
        window.innerWidth,
        window.innerHeight,
        CARD_MARGIN
      )
    );
  }, [hover]);

  useEffect(() => {
    const off = officeBus.onAgentHoverChanged((agentId, x, y) => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      if (agentId === null) {
        setHover(null);
        return;
      }
      timerRef.current = setTimeout(() => {
        setHover({ agentId, x, y });
        timerRef.current = null;
      }, SHOW_DELAY_MS);
    });
    return () => {
      off();
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  if (!hover || !agent) return null;
  // 월드(createCharacterAssets)와 동일한 아키타입 해석 — 누락 시 폴백
  // 프리뷰가 항상 human으로 렌더되는 버그.
  const src =
    portraitUrl ??
    spritePreviewUrl ??
    generateSpritePreview(
      agent.seed || agent.id,
      6,
      undefined,
      undefined,
      resolveArchetype(agent.archetype, agent.seed || agent.id)
    );
  // pos가 아직 측정 전(useLayoutEffect 실행 전)이면 원위치로 렌더하되 숨겨서,
  // clamp 적용 전 위치가 잠깐 보이는 점프를 방지한다.
  const left = pos ? pos.x : hover.x + 12;
  const top = pos ? pos.y : hover.y + 12;

  return (
    <div
      ref={cardRef}
      className="agent-hover-card"
      style={{ left, top, visibility: pos ? "visible" : "hidden" }}
    >
      <img className="agent-hover-portrait" src={src} alt={agent.name} />
      <div className="agent-hover-meta">
        <div className="agent-hover-name">{agent.name}</div>
        <div className="agent-hover-role">{agent.role}</div>
      </div>
    </div>
  );
}
