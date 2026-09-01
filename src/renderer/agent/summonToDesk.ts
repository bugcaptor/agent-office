// src/renderer/agent/summonToDesk.ts
//
// "전체 자리로" 오케스트레이터 — 출근은 했지만 터미널(세션)이 없어 탕비실에
// 있는 에이전트 전원의 세션을 한꺼번에 띄운다.
//
// 퇴근(clockOut.ts)과는 다른 축이다. 퇴근한 에이전트는 캔버스에서 아예
// 사라지므로 `clockInAll`이 다루고, 여기서 다루는 것은 캔버스에 있으나
// 세션이 idle/exited 라 behaviorFsm 규칙에 따라 탕비실로 간 사람들이다.
//
// 각 에이전트에 대해:
//   ① suppressNotifications — 소환 직후 밀려드는 잔여 알림 억제
//      (summonSuppress.ts, 기본 3초).
//   ② ensureSession — 상태를 starting 으로 선점하고 PTY 생성. 세션이 이미
//      살아 있거나 생성 중이면 no-op 이라 중복 생성이 없다.
//
// 터미널 오버레이는 열지 않는다 — 여러 명을 한꺼번에 부르는 기능이라
// 마지막 한 명의 터미널만 덮어씌우듯 뜨는 것은 의도가 아니다. 세션이
// starting 이 되는 순간 캐릭터는 알아서 자리로 돌아간다(behaviorFsm).
import { useAppStore } from "../store/appStore";
import { awayFromDeskIds } from "../store/selectors";
import { ensureSession } from "../ipc/sessionBridge";
import { suppressNotifications } from "./summonSuppress";

export function summonAllToDesk(): void {
  // ensureSession이 스토어를 바꾸므로 대상 목록 스냅샷을 먼저 뜬다.
  const ids = awayFromDeskIds(useAppStore.getState());
  for (const id of ids) {
    suppressNotifications(id);
    ensureSession(id);
  }
}
