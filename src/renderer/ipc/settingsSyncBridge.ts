// 백엔드 설정 변경(app-settings-changed)을 렌더러 사본에 반영한다(수정 F6).
// `ctl settings set`은 렌더러를 거치지 않고 저장하므로, 이 구독이 없으면
// 화면은 옛 값을 보여 준다. GUI 자신의 저장도 같은 이벤트로 돌아오며,
// 이벤트는 저장 순서대로 오므로 사본은 마지막 저장 값에 수렴한다.
import { tauriApi } from "./tauriApi";
import { useAppStore } from "../store/appStore";
import { applyLanguageSetting } from "../i18n";

export function installSettingsSync(): () => void {
  return tauriApi.onAppSettingsChanged((settings) => {
    useAppStore.getState().receiveAppSettings(settings);
    // 언어는 스토어 값만으로 화면이 바뀌지 않는다 — 같은 언어면 no-op.
    applyLanguageSetting(settings.language);
  });
}
