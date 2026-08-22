// src/web/avatar.ts
//
// 캐릭터 아바타 dataURL 해석. 우선순위는 데스크톱과 같다:
//
//   ① 커스텀 초상 — `portraitUpdatedAt`이 있으면 `media.portrait` RPC로 PNG를
//      base64로 받아 그대로 쓴다.
//   ② 절차 생성  — 없으면 seed+archetype으로 스프라이트 시트를 만들고 idle
//      첫 프레임(16px)을 nearest-neighbor로 확대한다.
//
// 렌더러의 `office/gen`을 그대로 import한다. 그 폴더는 Pixi 비의존이라
// (마스코트 창 `mascot/sheet.ts`가 같은 이유로 먼저 쓰고 있다) 웹 번들에
// Pixi가 딸려 오지 않는다.
//
// 결과는 모듈 캐시에 남는다 — 목록·채팅 헤더·브라우저 알림 아이콘이 같은
// 아바타를 서로 다른 시점에 요구하므로, 캐시가 없으면 절차 생성이 매 렌더마다
// 돈다.

import { CELL, defaultCanvasFactory } from "@renderer/office/gen/compositor";
import { generateSheet } from "@renderer/office/gen/sheetGen";
import { resolveArchetype } from "@renderer/office/gen/archetypes";

import type { RemoteAgent } from "./protocol";
import { RpcCmd } from "./protocol";
import type { WebRemoteSocket } from "./ws";

/** 아바타 한 변 픽셀 수. 16px 셀의 정수배(4배)라 확대가 또렷하다. */
export const AVATAR_PX = 64;

const cache = new Map<string, string>();
/** 같은 키에 대한 중복 RPC/생성을 합친다(목록 행이 한꺼번에 뜬다). */
const inflight = new Map<string, Promise<string | null>>();

/**
 * 캐시 키. 초상은 `portraitUpdatedAt`이 곧 무효화 키이고(데스크톱 규약과 동일),
 * 절차 생성은 seed+archetype이 결과를 결정한다.
 */
function colorKey(colors: RemoteAgent["colors"]): string {
  if (!colors) return "";
  return (["skin", "hair", "shirt"] as const).map((s) => colors[s] ?? "").join(",");
}

export function avatarKey(agent: RemoteAgent): string {
  const seed = agent.seed || agent.agentId;
  return agent.portraitUpdatedAt
    ? `portrait:${agent.agentId}:${agent.portraitUpdatedAt}`
    : `gen:${seed}:${agent.archetype ?? ""}:${colorKey(agent.colors)}`;
}

/** 절차 생성 아바타. 캔버스를 못 얻으면 null(호출부는 텍스트만 그린다). */
function generatedAvatar(agent: RemoteAgent): string | null {
  try {
    const seed = agent.seed || agent.agentId;
    const { sheet } = generateSheet(
      seed,
      defaultCanvasFactory,
      resolveArchetype(agent.archetype ?? undefined, seed),
      agent.colors ?? undefined
    );
    const out = document.createElement("canvas");
    out.width = AVATAR_PX;
    out.height = AVATAR_PX;
    const ctx = out.getContext("2d");
    if (!ctx) return null;
    // 픽셀아트라 보간을 끈다 — 마스코트·오피스뷰와 같은 규약.
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(
      sheet.canvas as CanvasImageSource,
      0,
      0,
      CELL,
      CELL,
      0,
      0,
      AVATAR_PX,
      AVATAR_PX
    );
    return out.toDataURL("image/png");
  } catch (err) {
    console.warn("avatar: 절차 생성 실패", err);
    return null;
  }
}

/**
 * 아바타 dataURL을 얻는다. 커스텀 초상 조회가 실패하면 절차 생성으로 조용히
 * 폴백한다(아바타가 통째로 사라지는 것보다 낫다 — 마스코트의 선례).
 */
export function resolveAvatar(
  socket: WebRemoteSocket,
  agent: RemoteAgent
): Promise<string | null> {
  const key = avatarKey(agent);
  const hit = cache.get(key);
  if (hit) return Promise.resolve(hit);
  const running = inflight.get(key);
  if (running) return running;

  const job = (async () => {
    if (agent.portraitUpdatedAt) {
      try {
        const png = await socket.rpc<string | null>(RpcCmd.mediaPortrait, {
          agentId: agent.agentId,
        });
        if (png) return `data:image/png;base64,${png}`;
      } catch {
        /* 폴백 */
      }
    }
    return generatedAvatar(agent);
  })()
    .then((url) => {
      if (url) cache.set(key, url);
      return url;
    })
    .finally(() => {
      inflight.delete(key);
    });

  inflight.set(key, job);
  return job;
}

/** 이미 풀린 아바타(동기). 알림처럼 기다릴 수 없는 자리에서 쓴다. */
export function cachedAvatar(agent: RemoteAgent): string | null {
  return cache.get(avatarKey(agent)) ?? null;
}
