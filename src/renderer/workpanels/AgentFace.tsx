import { useMemo } from "react";
import { useAppStore } from "../store/appStore";
import { generateSpritePreview } from "../office/gen/characterFactory";
import { resolveArchetype } from "../office/gen/archetypes";
import { findRemote, remoteProfile, useRemoteFloors } from "../link/remoteFloors";
export function AgentFace({ agentId }: { agentId: string }) {
  const local = useAppStore((s) => s.agents[agentId]);
  // 머신 연동 호스트: 다른 머신 캐릭터(`기기id:캐릭터id`)는 절차 생성 모습으로(커스텀 시트·초상은 오지 않는다).
  const remote = useRemoteFloors((s) => (local || !agentId.includes(":") ? null : findRemote(s.office, agentId)?.character ?? null));
  const profile = useMemo(() => local ?? (remote ? remoteProfile(remote) : undefined), [local, remote]);
  const portrait = useAppStore((s) => s.portraits[agentId]);
  const sprite = useAppStore((s) => s.spritePreviews[agentId]);
  const fallback = useMemo(() => profile ? generateSpritePreview(profile.seed || profile.id, 2, undefined, undefined, resolveArchetype(profile.archetype, profile.seed || profile.id), profile.colors) : undefined, [profile]);
  const image = portrait ?? sprite ?? fallback;
  return image ? <img className="work-agent-face" src={image} alt="" /> : null;
}
