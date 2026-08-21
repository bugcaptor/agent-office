// src/renderer/office/scenes/sceneStorage.ts
//
// 풍경 선택의 영속 계층 — theme/applyTheme.ts와 같은 관례(레지스트리는 순수
// 데이터, localStorage를 만지는 코드는 여기 한 곳).
//
// 테마와 달리 DOM 부수효과는 없다(CSS 토큰을 쓰지 않고 Pixi 씬만 바뀐다) —
// 실제 재구축은 `OfficeScene.setScene`이 하고, 여기서는 저장/복원만 한다.
// 스토어 초기값이 `loadStoredSceneId()`라 첫 렌더부터 저장된 풍경이 뜬다.
import { DEFAULT_SCENE_ID, isSceneId } from "./scenes";
import type { SceneId } from "./sceneTypes";

export const SCENE_STORAGE_KEY = "agent-office.scene";

/** 저장된 풍경 id를 읽는다. 없거나 알 수 없는 값이면 기본(office). */
export function loadStoredSceneId(): SceneId {
  try {
    const raw = localStorage.getItem(SCENE_STORAGE_KEY);
    return isSceneId(raw) ? raw : DEFAULT_SCENE_ID;
  } catch {
    return DEFAULT_SCENE_ID; // localStorage 부재(node) 포함
  }
}

/** 풍경 선택을 영속한다. 저장 불가 환경(프라이빗 모드/node)에서는 조용히 넘어간다. */
export function persistSceneId(id: SceneId): void {
  try {
    localStorage.setItem(SCENE_STORAGE_KEY, id);
  } catch {
    // 적용 자체는 유효하므로 무시 — applyTheme과 같은 판단.
  }
}
