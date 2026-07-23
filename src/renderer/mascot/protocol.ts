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

/** 창 크기와 스프라이트 배치(논리 px). tauri.conf.json의 mascot 창 크기와 짝. */
export const MASCOT_WINDOW_W = 120;
export const MASCOT_WINDOW_H = 140;
/** 스프라이트 렌더 박스 한 변(px) = CELL(16) × 프리뷰 관례 배율(6). */
export const MASCOT_SPRITE_PX = 96;
/** idle 프레임 교체 주기(ms). CharacterEntity의 ANIM_IDLE_MS와 같은 값 —
 *  오피스 씬의 캐릭터와 호흡이 어긋나 보이지 않게. */
export const MASCOT_ANIM_IDLE_MS = 480;

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
  /** 커스텀 시트 존재 표시 + 캐시 무효화 키. null이면 절차 생성 경로. */
  spriteUpdatedAt: number | null;
  /** 알림 대기 중 — 배지 + 바운스. */
  hasPending: boolean;
  /** 턴 진행 중. */
  working: boolean;
}

export const HIDDEN_MASCOT_STATE: MascotState = {
  visible: false,
  agentId: null,
  name: null,
  seed: null,
  archetype: null,
  spriteUpdatedAt: null,
  hasPending: false,
  working: false,
};

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null;

const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

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
    spriteUpdatedAt: num(payload.spriteUpdatedAt),
    hasPending: payload.hasPending === true,
    working: payload.working === true,
  };
}

/** 두 상태가 같은가(방출 dedupe용). 필드가 전부 원시값이라 얕은 비교로 충분. */
export function sameMascotState(a: MascotState, b: MascotState): boolean {
  return (
    a.visible === b.visible &&
    a.agentId === b.agentId &&
    a.name === b.name &&
    a.seed === b.seed &&
    a.archetype === b.archetype &&
    a.spriteUpdatedAt === b.spriteUpdatedAt &&
    a.hasPending === b.hasPending &&
    a.working === b.working
  );
}
