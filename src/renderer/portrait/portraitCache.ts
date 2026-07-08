// src/renderer/portrait/portraitCache.ts
//
// 초상 dataURL 캐시의 로드/무효화 배선. 앱 시작 시 portraitUpdatedAt이
// 있는 에이전트의 초상을 병렬 로드하고, 이후 에이전트 제거 시 백엔드 파일 삭제 +
// 캐시 정리를 수행한다. 순수 헬퍼는 Vitest로 직접 검증한다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { AgentProfile } from "../store/types";

/** portraitUpdatedAt이 있는(=초상 존재) 에이전트 id 목록. 순수. */
export function agentsNeedingPortraits(
  agents: Record<string, AgentProfile>
): string[] {
  return Object.values(agents)
    .filter((a) => a.portraitUpdatedAt != null)
    .map((a) => a.id);
}

/** 백엔드 base64(헤더 없음) -> <img src> 용 dataURL. 순수. */
export function pngBase64ToDataUrl(b64: string): string {
  return `data:image/png;base64,${b64}`;
}

/** 주어진 id들의 초상을 병렬 로드해 스토어 캐시에 채운다. 실패는 조용히 폴백. */
export async function loadPortraitsFor(ids: string[]): Promise<void> {
  await Promise.all(
    ids.map(async (id) => {
      try {
        const b64 = await tauriApi.loadPortrait(id);
        if (b64) useAppStore.getState().setPortrait(id, pngBase64ToDataUrl(b64));
      } catch (err) {
        console.warn(`portraitCache: loadPortrait failed for ${id}`, err);
      }
    })
  );
}

/**
 * 시작 로드 + 제거 브리지 설치. bootstrap에서 hydrate 후 1회 호출.
 * 반환값은 구독 해제 함수(테스트/대칭용).
 */
export function installPortraitCache(): () => void {
  // 시작 로드: 초상이 있는 에이전트만.
  void loadPortraitsFor(agentsNeedingPortraits(useAppStore.getState().agents));

  // 제거 감지: agents 셀렉터 변화에서 사라진 id의 파일을 삭제하고 캐시를 비운다.
  let prevIds = new Set(Object.keys(useAppStore.getState().agents));
  const unsub = useAppStore.subscribe(
    (s) => s.agents,
    (agents) => {
      const nextIds = new Set(Object.keys(agents));
      for (const id of prevIds) {
        if (!nextIds.has(id)) {
          void tauriApi
            .deletePortrait(id)
            .catch((err) =>
              console.warn(`portraitCache: deletePortrait failed for ${id}`, err)
            );
          useAppStore.getState().removePortrait(id);
        }
      }
      prevIds = nextIds;
    }
  );
  return unsub;
}
