// src/renderer/memo/memoCleanup.ts
//
// 캐릭터 삭제 시 메모 폴더(`memos/<agentId>/`) 정리 브리지.
//
// 초상/스프라이트 정리와 **같은 지점·같은 방식**이다(portraitCache.ts /
// spriteCache.ts): 이 저장소에는 "캐릭터 삭제" 백엔드 커맨드가 없고, 삭제는
// appStore의 `removeAgent`로만 일어난다. 그래서 각 서브시스템이 `agents`
// 셀렉터를 구독해 사라진 id의 파일을 스스로 지운다.
//
// 별 파일로 뺀 이유: memoStore는 appStore와 비커플링(독립 스토어 관례)을
// 유지해야 하므로, 두 스토어를 잇는 이 브리지만 appStore를 import한다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";

/** bootstrap에서 hydrate 후 1회 호출. 반환값은 구독 해제 함수(테스트/대칭용). */
export function installMemoCleanup(): () => void {
  let prevIds = new Set(Object.keys(useAppStore.getState().agents));
  return useAppStore.subscribe(
    (s) => s.agents,
    (agents) => {
      const nextIds = new Set(Object.keys(agents));
      for (const id of prevIds) {
        if (!nextIds.has(id)) {
          void tauriApi
            .deleteMemos(id)
            .catch((err) => console.warn(`memoCleanup: deleteMemos failed for ${id}`, err));
        }
      }
      prevIds = nextIds;
    }
  );
}
