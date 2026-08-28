// src/renderer/mascot/protocol.ts
//
// 데스크톱 마스코트 창(이슈 #72, docs/mascot-window-design.md)의 창 간 계약.
//
// main 창(zustand = 진실의 원천)이 `mascot-state` 이벤트로 이 페이로드를
// 브로드캐스트하고, mascot 창은 순수 소비자로 받아 그린다. Rust serde를 거치지
// 않는 renderer↔renderer 이벤트이므로 `shared/types`의 frozen contract /
// contract-fixture 대상이 아니다 — 대신 여기 파서가 런타임 가드를 맡는다.
//
// 스프라이트 **픽셀**은 이벤트로 나르지 않는다: 절차 생성은 seed+archetype으로
// mascot이 결정적으로 재생성하고(main과 같은 외형 보장), 커스텀 시트는 mascot이
// `load_sprite` 커맨드로 직접 읽는다. 페이로드는 항상 작다.

import type { ColorOverrides, MascotLightsFace } from "@shared/types";

/** 창 크기와 스프라이트 배치(논리 px). tauri.conf.json의 mascot 창 크기와 짝. */
export const MASCOT_WINDOW_W = 120;
/** 스프라이트 렌더 박스 한 변(px) = CELL(16) × 프리뷰 관례 배율(6). */
export const MASCOT_SPRITE_PX = 96;
/** 스프라이트 위 여유(논리 px) — 알림 시 hop(-4px)·배지가 창 밖으로 잘리지
 *  않을 만큼만. 예전 값(140)은 스프라이트 96 아래로 44px의 죽은 공간을 남겨
 *  캐릭터가 신호등 위에 붕 떠 보였다. */
export const MASCOT_SPRITE_HEADROOM = 6;
/** 스프라이트 영역이 차지하는 창 높이(논리 px). */
export const MASCOT_WINDOW_H = MASCOT_SPRITE_PX + MASCOT_SPRITE_HEADROOM;
/** idle 프레임 교체 주기(ms). CharacterEntity의 ANIM_IDLE_MS와 같은 값 —
 *  오피스 씬의 캐릭터와 호흡이 어긋나 보이지 않게. */
export const MASCOT_ANIM_IDLE_MS = 480;

/** 램프 안 프로필 그림(원) 지름(논리 px). */
export const LIGHT_AVATAR_PX = 28;
/** 램프 칸(타일) 폭(논리 px) — 프로필 그림 + 이름 한 줄이 들어간다. */
export const LIGHT_TILE_W = 54;
/** 램프 칸(타일) 높이(논리 px). */
export const LIGHT_TILE_H = 48;
/** 램프 사이 간격(논리 px). */
export const LIGHT_GAP = 6;
/** strip 내부 여백(논리 px, 사방 동일). */
export const LIGHT_STRIP_PAD = 6;
/** 신호등 최대 칸 수(오버플로 칩 포함) — 이를 넘으면 앞 MAX_LIGHTS-1칸 + `+k` 칩.
 *  칸이 이름표를 단 타일이 되면서 폭이 커져(54px) 12칸이면 창이 600px을 넘는다 —
 *  데스크톱 위젯 한도에 맞춰 8칸으로 줄였다(8칸 가로 = 486px). */
export const MAX_LIGHTS = 8;
/** 작업명(`mascotLightsLabel==="task"`) 표시 시 칸 폭(논리 px) — 60자 절단
 *  텍스트가 한 줄 안에서 조금 더 읽히도록 54→96px로 넓힌다. */
export const LIGHT_TILE_W_WIDE = 96;
/** wide 칸일 때 가로 배열의 최대 칸 수 — 5칸 가로 = 12+96*5+6*4 = 516px로
 *  기존 8칸(486px)과 비슷한 창 폭을 유지한다(오버플로 칩 포함). */
export const MAX_LIGHTS_WIDE = 5;

/**
 * 신호등 최대 칸 수를 wide/vertical에 맞춰 고른다. **세로 배열은 줄이지
 * 않는다** — 세로 strip은 칸이 늘어도 화면 폭(taile 폭 1개분)만 차지하고
 * 늘어나는 건 높이뿐이라, wide 타일이 늘려 잡아먹는 자원은 가로 배열에서만
 * 문제가 된다(사용자 확정).
 */
export function maxLightsFor(wide: boolean, vertical: boolean): number {
  return wide && !vertical ? MAX_LIGHTS_WIDE : MAX_LIGHTS;
}

/** 신호등 램프 하나의 상태(docs/mascot-lights-design.md §3). */
export type MascotLightState = "off" | "working" | "attention";

/**
 * 램프 칸에 얼굴을 띄울 에이전트의 스프라이트 좌표(설계 §6 개정). 에이전트
 * 모드는 그 에이전트 자신, 프로젝트 모드는 대표 에이전트(clickAgentId)다.
 * 픽셀이 아니라 재생성 좌표만 나른다 — `MascotState`의 스프라이트 필드와 같은
 * 규약이라 마스코트 창이 결정적으로 같은 얼굴을 다시 만든다.
 */
export interface MascotLightAvatar {
  /** 커스텀 시트 로드 키이자 캐시 키. */
  agentId: string;
  /** 절차 생성 시드(프로필의 seed || id). */
  seed: string;
  archetype: string | null;
  colors: ColorOverrides | null;
  /** 커스텀 시트 존재 표시 + 캐시 무효화 키. null이면 절차 생성 경로. */
  spriteUpdatedAt: number | null;
  /** 초상 존재 표시 + 캐시 무효화 키. null이면 초상 없음(스프라이트 폴백). */
  portraitUpdatedAt: number | null;
}

/** 신호등 램프 하나 — 에이전트(agents 모드) 또는 프로젝트 폴더(projects 모드). */
export interface MascotLight {
  /** 안정 키 — agentId 또는 프로젝트 폴더 경로(설정 원문). */
  id: string;
  /** 칸 아래 표시할 이름 — `mascotLightsLabel` 설정으로 고른 텍스트(에이전트
   *  이름/프로젝트명/작업명 중 하나, 값이 비면 auto로 폴백). 잘릴 수 있다(칸
   *  폭 안에서 말줄임). 비번역(마스코트 창 안은 번역 문자열 금지). */
  label: string;
  /** 호버 툴팁 — [에이전트 이름, 프로젝트명, 작업명] 중 있는 것만 " · "로 이어
   *  붙인 전체 텍스트(잘린 label을 여기서 확인). 비번역. */
  tooltip: string;
  state: MascotLightState;
  /** 클릭 시 활성화할 대표 에이전트. null이면 클릭 no-op. */
  clickAgentId: string | null;
  /** 칸에 얼굴을 띄울 에이전트. null이면 이름 첫 글자 원판으로 대체한다
   *  (세션이 없는 프로젝트 폴더). */
  avatar: MascotLightAvatar | null;
}

/** main → mascot 상태 스냅샷. 항상 전체 상태(델타 아님)라 수신측이 멱등하다. */
export interface MascotState {
  /** 창을 띄울지. mascotEnabled && (활동 캐릭터 있음 || linger 중). */
  visible: boolean;
  agentId: string | null;
  name: string | null;
  /** 절차 생성 시드(프로필의 seed || id). */
  seed: string | null;
  /** resolveArchetype에 넘길 원본 값(미지정이면 null → "human" 폴백). */
  archetype: string | null;
  /** 사용자가 고른 팔레트 색 오버라이드. 절차 생성 시트를 오피스뷰와 같은 색으로 그린다. */
  colors: ColorOverrides | null;
  /** 커스텀 시트 존재 표시 + 캐시 무효화 키. null이면 절차 생성 경로. */
  spriteUpdatedAt: number | null;
  /** 알림 대기 중 — 배지 + 바운스. */
  hasPending: boolean;
  /** 턴 진행 중. */
  working: boolean;
  /** 신호등 칸 목록. 빈 배열 = 기능 꺼짐(strip 미렌더). */
  lights: MascotLight[];
  /** true = 신호등을 세로로 배열. */
  lightsVertical: boolean;
  /** 신호등 칸 얼굴 원판에 스프라이트/초상화 중 무엇을 띄울지(설정 미러). */
  lightsFace: MascotLightsFace;
  /** true = 칸을 넓게(96px) 그린다 — `mascotLightsLabel==="task"`일 때만.
   *  마스코트 창은 설정 의미를 모르고 렌더 관심사(칸 폭)만 받는다(기존
   *  lightsFace/lightsVertical과 같은 규약). */
  lightsWide: boolean;
}

export const HIDDEN_MASCOT_STATE: MascotState = {
  visible: false,
  agentId: null,
  name: null,
  seed: null,
  archetype: null,
  colors: null,
  spriteUpdatedAt: null,
  hasPending: false,
  working: false,
  lights: [],
  lightsVertical: false,
  lightsFace: "sprite",
  lightsWide: false,
};

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null;

const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

/** 색 오버라이드 가드 — 슬롯 3종의 문자열 값만 통과시킨다(그 외는 버린다).
 *  값의 hex 형식 검증은 팔레트 쪽(`parseHexColor`)이 하므로 여기선 모양만 본다. */
const colorsOf = (v: unknown): ColorOverrides | null => {
  if (!isRecord(v)) return null;
  const out: ColorOverrides = {};
  for (const slot of ["skin", "hair", "shirt"] as const) {
    const c = str(v[slot]);
    if (c !== null) out[slot] = c;
  }
  return Object.keys(out).length > 0 ? out : null;
};

/** 색 오버라이드 동치(방출 dedupe용) — 슬롯 3종만 보면 된다. */
const sameColors = (a: ColorOverrides | null, b: ColorOverrides | null): boolean =>
  (["skin", "hair", "shirt"] as const).every((s) => (a?.[s] ?? null) === (b?.[s] ?? null));

const LIGHT_STATES = new Set(["off", "working", "attention"]);

/** 아바타 좌표 가드 — agentId/seed 문자열이 없으면 통째로 버린다(얼굴 없이
 *  이름 첫 글자로 폴백). 나머지는 부재를 null로 접는다. */
function avatarOf(v: unknown): MascotLightAvatar | null {
  if (!isRecord(v)) return null;
  const agentId = str(v.agentId);
  const seed = str(v.seed);
  if (agentId === null || seed === null) return null;
  return {
    agentId,
    seed,
    archetype: str(v.archetype),
    colors: colorsOf(v.colors),
    spriteUpdatedAt: num(v.spriteUpdatedAt),
    portraitUpdatedAt: num(v.portraitUpdatedAt),
  };
}

/** 두 아바타가 같은가(dedupe용) — 얼굴을 바꾸는 6필드만 본다. */
function sameAvatar(a: MascotLightAvatar | null, b: MascotLightAvatar | null): boolean {
  if (a === null || b === null) return a === b;
  return (
    a.agentId === b.agentId &&
    a.seed === b.seed &&
    a.archetype === b.archetype &&
    a.spriteUpdatedAt === b.spriteUpdatedAt &&
    a.portraitUpdatedAt === b.portraitUpdatedAt &&
    sameColors(a.colors, b.colors)
  );
}

/**
 * 램프 항목 하나의 형태 가드. id/label은 문자열 필수, state는 3종 밖이면 "off"로
 * 강등(전송 측 버그로 창이 죽는 것보다 안전 쪽으로), clickAgentId는 문자열이거나
 * null. id/label이 없으면 항목 자체를 버린다(호출부가 개별 드롭 처리).
 * tooltip은 문자열이 아니면 label로 접는다(구버전/손상 페이로드도 호버가
 * 아예 비지 않게).
 */
function lightOf(v: unknown): MascotLight | null {
  if (!isRecord(v)) return null;
  const id = str(v.id);
  const label = str(v.label);
  if (id === null || label === null) return null;
  const rawState = typeof v.state === "string" ? v.state : "off";
  const state = (LIGHT_STATES.has(rawState) ? rawState : "off") as MascotLightState;
  return {
    id,
    label,
    tooltip: str(v.tooltip) ?? label,
    state,
    clickAgentId: str(v.clickAgentId),
    avatar: avatarOf(v.avatar),
  };
}

/** 램프 목록 가드 — 부재/비배열은 하위호환으로 빈 배열, 항목별 실패는 개별 드롭. */
function lightsOf(v: unknown): MascotLight[] {
  if (!Array.isArray(v)) return [];
  const out: MascotLight[] = [];
  for (const item of v) {
    const light = lightOf(item);
    if (light !== null) out.push(light);
  }
  return out;
}

/** 두 램프 목록이 같은가(dedupe용) — 길이와 항목별 6필드를 순서대로 비교. */
function sameLights(a: MascotLight[], b: MascotLight[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((la, i) => {
    const lb = b[i];
    return (
      la.id === lb.id &&
      la.label === lb.label &&
      la.tooltip === lb.tooltip &&
      la.state === lb.state &&
      la.clickAgentId === lb.clickAgentId &&
      sameAvatar(la.avatar, lb.avatar)
    );
  });
}

/**
 * 이벤트 페이로드 → MascotState. 형태가 아니면 null을 돌려 수신측이 조용히
 * 무시하게 한다(창 간 버전 불일치·손상 페이로드에 마스코트가 깨지지 않도록).
 */
export function parseMascotState(payload: unknown): MascotState | null {
  if (!isRecord(payload)) return null;
  if (typeof payload.visible !== "boolean") return null;
  return {
    visible: payload.visible,
    agentId: str(payload.agentId),
    name: str(payload.name),
    seed: str(payload.seed),
    archetype: str(payload.archetype),
    colors: colorsOf(payload.colors),
    spriteUpdatedAt: num(payload.spriteUpdatedAt),
    hasPending: payload.hasPending === true,
    working: payload.working === true,
    lights: lightsOf(payload.lights),
    lightsVertical: payload.lightsVertical === true,
    lightsFace: payload.lightsFace === "portrait" ? "portrait" : "sprite",
    lightsWide: payload.lightsWide === true,
  };
}

/** 두 상태가 같은가(방출 dedupe용). colors·lights만 객체/배열이라 별도 비교한다. */
export function sameMascotState(a: MascotState, b: MascotState): boolean {
  return (
    a.visible === b.visible &&
    a.agentId === b.agentId &&
    a.name === b.name &&
    a.seed === b.seed &&
    a.archetype === b.archetype &&
    sameColors(a.colors, b.colors) &&
    a.spriteUpdatedAt === b.spriteUpdatedAt &&
    a.hasPending === b.hasPending &&
    a.working === b.working &&
    sameLights(a.lights, b.lights) &&
    a.lightsVertical === b.lightsVertical &&
    a.lightsFace === b.lightsFace &&
    a.lightsWide === b.lightsWide
  );
}
