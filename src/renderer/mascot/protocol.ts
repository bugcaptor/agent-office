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

import type { ColorOverrides } from "@shared/types";

/** 창 크기와 스프라이트 배치(논리 px). tauri.conf.json의 mascot 창 크기와 짝. */
export const MASCOT_WINDOW_W = 120;
export const MASCOT_WINDOW_H = 140;
/** 스프라이트 렌더 박스 한 변(px) = CELL(16) × 프리뷰 관례 배율(6). */
export const MASCOT_SPRITE_PX = 96;
/** idle 프레임 교체 주기(ms). CharacterEntity의 ANIM_IDLE_MS와 같은 값 —
 *  오피스 씬의 캐릭터와 호흡이 어긋나 보이지 않게. */
export const MASCOT_ANIM_IDLE_MS = 480;

/** 신호등 램프(원) 지름(논리 px). */
export const LIGHT_PX = 18;
/** 램프 사이 간격(논리 px). */
export const LIGHT_GAP = 6;
/** strip 내부 여백(논리 px, 사방 동일). */
export const LIGHT_STRIP_PAD = 6;
/** 신호등 최대 칸 수(오버플로 칩 포함) — 이를 넘으면 앞 MAX_LIGHTS-1칸 + `+k` 칩. */
export const MAX_LIGHTS = 12;

/** 신호등 램프 하나의 상태(docs/mascot-lights-design.md §3). */
export type MascotLightState = "off" | "working" | "attention";

/** 신호등 램프 하나 — 에이전트(agents 모드) 또는 프로젝트 폴더(projects 모드). */
export interface MascotLight {
  /** 안정 키 — agentId 또는 프로젝트 폴더 경로(설정 원문). */
  id: string;
  /** 툴팁 텍스트 — 에이전트 이름 또는 폴더 basename. 비번역(마스코트 창 안은 번역 문자열 금지). */
  label: string;
  state: MascotLightState;
  /** 클릭 시 활성화할 대표 에이전트. null이면 클릭 no-op. */
  clickAgentId: string | null;
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

/**
 * 램프 항목 하나의 형태 가드. id/label은 문자열 필수, state는 3종 밖이면 "off"로
 * 강등(전송 측 버그로 창이 죽는 것보다 안전 쪽으로), clickAgentId는 문자열이거나
 * null. id/label이 없으면 항목 자체를 버린다(호출부가 개별 드롭 처리).
 */
function lightOf(v: unknown): MascotLight | null {
  if (!isRecord(v)) return null;
  const id = str(v.id);
  const label = str(v.label);
  if (id === null || label === null) return null;
  const rawState = typeof v.state === "string" ? v.state : "off";
  const state = (LIGHT_STATES.has(rawState) ? rawState : "off") as MascotLightState;
  return { id, label, state, clickAgentId: str(v.clickAgentId) };
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

/** 두 램프 목록이 같은가(dedupe용) — 길이와 항목별 4필드를 순서대로 비교. */
function sameLights(a: MascotLight[], b: MascotLight[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((la, i) => {
    const lb = b[i];
    return (
      la.id === lb.id &&
      la.label === lb.label &&
      la.state === lb.state &&
      la.clickAgentId === lb.clickAgentId
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
    a.lightsVertical === b.lightsVertical
  );
}
