import { useMemo } from "react";
import { useAppStore } from "../store/appStore";
import { generateSpritePreview } from "../office/gen/characterFactory";
import { resolveArchetype } from "../office/gen/archetypes";
export function AgentFace({ agentId }: { agentId: string }) {
  const profile = useAppStore((s) => s.agents[agentId]);
  const portrait = useAppStore((s) => s.portraits[agentId]);
  const sprite = useAppStore((s) => s.spritePreviews[agentId]);
  const fallback = useMemo(() => profile ? generateSpritePreview(profile.seed || profile.id, 2, undefined, undefined, resolveArchetype(profile.archetype, profile.seed || profile.id), profile.colors) : undefined, [profile]);
  const image = portrait ?? sprite ?? fallback;
  return image ? <img className="work-agent-face" src={image} alt="" /> : null;
}
