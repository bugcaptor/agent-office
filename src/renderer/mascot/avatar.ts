// src/renderer/mascot/avatar.ts
//
// 신호등 칸에 얹는 "프로필 사진" — 그 칸의 대표 에이전트 얼굴이다.
//
// 스프라이트 확보 경로는 마스코트 본체와 같은 `loadMascotFrames`를 그대로 쓴다
// (커스텀 시트 → 실패 시 절차 생성 폴백). 다른 점은 두 가지뿐이다:
//  1) 애니메이션이 없다 — idle0 한 장만 쓴다(칸이 최대 8개라 칸마다 raf를 돌릴
//     이유가 없다. 움직이는 정보는 램프 색·펄스가 이미 나른다).
//  2) 전신이 아니라 **머리 부분만 잘라** 원판에 채운다 — 28px 원 안에 16px 전신을
//     넣으면 얼굴이 3~4px로 뭉개져 누구인지 알아볼 수 없다.
//
// 같은 에이전트가 여러 칸(중첩 프로젝트 폴더)에 나오거나 리렌더가 반복돼도
// 시트를 다시 만들지 않도록 좌표+배율 키로 프라미스를 캐시한다.
import { loadMascotFrames, type MascotFrames } from "./sheet";
import type { MascotLightAvatar } from "./protocol";

/**
 * 16×16 셀에서 얼굴로 잘라낼 사각형(셀 한 변에 대한 비율).
 * parts.ts의 BODY_BASE_FRONT 기준: 머리는 y=2..7, 어깨까지가 y=8..11, 좌우
 * 윤곽선은 x=3..12다. 사방으로 1px씩 여유를 둔 (2,1)-(13,12) 정사각형을 쓰면
 * 헤어 실루엣이 잘리지 않으면서 얼굴이 원판을 꽉 채운다.
 */
export const AVATAR_CROP = { x: 2 / 16, y: 1 / 16, size: 12 / 16 } as const;

/** 캐시 키 — 얼굴을 바꾸는 좌표 전부 + 렌더 배율(커스텀 시트 프리필터가 배율에 의존). */
export function avatarKey(avatar: MascotLightAvatar, dpr: number): string {
  return [
    avatar.agentId,
    avatar.seed,
    avatar.archetype ?? "",
    JSON.stringify(avatar.colors ?? null),
    avatar.spriteUpdatedAt ?? "",
    dpr,
  ].join("|");
}

const cache = new Map<string, Promise<MascotFrames | null>>();

/** 프로필용 프레임 확보(캐시). 실패는 호출부가 null로 받아 첫 글자 폴백. */
export function loadAvatarFrames(
  avatar: MascotLightAvatar,
  dpr: number,
): Promise<MascotFrames | null> {
  const key = avatarKey(avatar, dpr);
  const hit = cache.get(key);
  if (hit) return hit;
  const promise = loadMascotFrames(
    {
      agentId: avatar.agentId,
      seed: avatar.seed,
      archetype: avatar.archetype,
      colors: avatar.colors,
      spriteUpdatedAt: avatar.spriteUpdatedAt,
    },
    dpr,
  ).catch((err) => {
    // 캐시에 실패한 프라미스를 남기면 영영 얼굴이 안 나온다 — 지우고 null.
    cache.delete(key);
    console.warn("mascot: failed to build light avatar", err);
    return null;
  });
  cache.set(key, promise);
  return promise;
}

/** idle0의 얼굴 영역을 캔버스에 꽉 채워 그린다(nearest — 픽셀아트 유지). */
export function drawAvatar(canvas: HTMLCanvasElement, frames: MascotFrames): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const cell = frames.cell;
  const sx = Math.round(cell * AVATAR_CROP.x);
  const sy = Math.round(cell * AVATAR_CROP.y);
  const s = Math.max(1, Math.round(cell * AVATAR_CROP.size));
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(frames.idle[0], sx, sy, s, s, 0, 0, canvas.width, canvas.height);
}
