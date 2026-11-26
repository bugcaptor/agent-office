// src/renderer/portrait/agentThumb.ts
//
// 캐릭터의 작은 얼굴 그림. 올린 초상이 있으면 그것, 없으면 스프라이트 미리보기, 그것도 없으면 시드로
// 그린 픽셀 캐릭터다. 터미널 탭과 대화 보기가 같은 그림을 쓴다.
import { useMemo } from "react";
import type { AgentProfile } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { generateSpritePreview } from "../office/gen/characterFactory";
import { resolveArchetype } from "../office/gen/archetypes";

export function agentThumb(agent: AgentProfile | undefined, portrait?: string, spritePreview?: string): string | undefined {
  if (portrait) return portrait;
  if (spritePreview) return spritePreview;
  if (!agent) return undefined;
  // 월드(createCharacterAssets)와 같은 아키타입 해석 — 빠지면 썸네일이 늘 human으로 그려진다.
  const seed = agent.seed || agent.id;
  return generateSpritePreview(seed, 6, undefined, undefined, resolveArchetype(agent.archetype, seed), agent.colors);
}

/** 이 캐릭터의 얼굴 그림(캐릭터가 아니면 undefined). */
export function useAgentThumb(agentId: string): string | undefined {
  const agent = useAppStore((s) => s.agents[agentId]);
  const portrait = useAppStore((s) => s.portraits[agentId]);
  const sprite = useAppStore((s) => s.spritePreviews[agentId]);
  return useMemo(() => agentThumb(agent, portrait, sprite), [agent, portrait, sprite]);
}
