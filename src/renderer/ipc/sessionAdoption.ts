// src/renderer/ipc/sessionAdoption.ts
//
// 부팅 시(그리고 프로필 로드 재시도 성공 시) sessiond에 남아 있던 세션을
// 되찾아 스토어에 시드한다. bootstrap.ts에서 분리 — profileLoad의 재시도도
// 같은 경로를 쓴다.
import { useAppStore } from "../store/appStore";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { tauriApi } from "./tauriApi";

/**
 * 세션 핸드오프(docs/session-handoff-design.md §핵심 6) 입양: 데몬에 남아있던
 * 세션들을 Running으로 스토어에 시드하고, 그 터미널이 처음 activate될 때
 * redraw nudge(TerminalRegistry.markAdopted)를 태우도록 표시한다. 미지원
 * 플랫폼/데몬 없음은 빈 배열 응답이라 자연히 no-op. 실패해도 부팅은 계속
 * (loadState/getAppSettings와 동일한 폴백 패턴) — 최악의 경우 이전 세션을
 * 못 되찾을 뿐, 새 세션 시작 자체는 영향받지 않는다.
 */
export async function adoptDetachedSessions(): Promise<void> {
  try {
    const adopted = await tauriApi.adoptDetachedSessions();
    if (adopted.length === 0) return;
    for (const info of adopted) {
      useAppStore
        .getState()
        .setSessionState({ agentId: info.agentId, status: "running" });
      useAppStore.getState().setSessionSize(info.agentId, info.cols, info.rows);
      // 세션 사용량(터미널 요약 바, docs/session-analytics-design.md §11.2):
      // 이 방어적 시드 루프도 session-state 이벤트와 마찬가지로 sessionId를
      // 잡아둬야 한다 — 이벤트가 유실/지연되면 sessionUsage[agentId]가
      // undefined로 남아 시드에 누계가 있어도 사용량 스팬이 아예 안 뜬다.
      // 같은 sessionId면 noteUsageSession이 no-op이라 정상 도착 경로와 안
      // 충돌한다.
      useAppStore.getState().noteUsageSession(info.agentId, info.sessionId);
    }
    terminalRegistry.markAdopted(adopted.map((a) => a.agentId));
  } catch (err) {
    console.warn(
      "bootstrap: session adoption failed, continuing without prior sessions",
      err,
    );
  }
}
