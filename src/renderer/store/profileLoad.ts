// src/renderer/store/profileLoad.ts
//
// 프로필(profiles.json) 로드와 그 실패 상태(수정 F2). 백엔드는 파일이 있는데
// 읽을 수 없으면 `loadState`를 거부하고 원본을 `profiles.json.corrupt-*`로
// 보존한다. 이때 렌더러는 빈 목록으로 부팅하되 **읽기 전용**으로 머문다 —
// 그 빈 목록을 저장하면 원본이 덮여 캐릭터가 전부 사라지기 때문이다
// (업무 목록 workItemStore의 load 실패 처리와 같은 정책). 재시도가 성공하면
// 실제 프로필로 hydrate하고, 부팅 때 건너뛴 세션 입양도 이어서 한다.
import { tauriApi } from "../ipc/tauriApi";
import { adoptDetachedSessions } from "../ipc/sessionAdoption";
import { useAppStore } from "./appStore";
import { useProfileLoadStore } from "./profileLoadState";

export { profilesReadOnly, useProfileLoadStore } from "./profileLoadState";

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * 부팅 1회: `loadState` → `hydrate`. 실패해도 throw하지 않는다 — 빈 상태로
 * hydrate하고 읽기 전용 표시를 남긴다. 성공 여부를 돌려준다.
 */
export async function loadProfiles(): Promise<boolean> {
  try {
    const state = await tauriApi.loadState();
    useProfileLoadStore.setState({ error: null });
    useAppStore.getState().hydrate(state);
    return true;
  } catch (err) {
    console.warn("bootApp: loadState failed, continuing with empty read-only state", err);
    useProfileLoadStore.setState({ error: errorText(err) });
    useAppStore.getState().hydrate({ agents: [], version: 1 });
    return false;
  }
}

/** 배너의 "다시 시도". 성공하면 실제 프로필로 바꾸고 세션 입양을 이어 한다. */
export async function retryProfileLoad(): Promise<boolean> {
  if (useProfileLoadStore.getState().retrying) return false;
  useProfileLoadStore.setState({ retrying: true });
  try {
    const state = await tauriApi.loadState();
    useProfileLoadStore.setState({ error: null });
    useAppStore.getState().hydrate(state);
  } catch (err) {
    console.warn("profiles: reload failed", err);
    useProfileLoadStore.setState({ error: errorText(err) });
    return false;
  } finally {
    useProfileLoadStore.setState({ retrying: false });
  }
  await adoptDetachedSessions();
  return true;
}
