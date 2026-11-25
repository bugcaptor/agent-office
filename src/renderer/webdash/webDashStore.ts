// src/renderer/webdash/webDashStore.ts
//
// 웹 상태판의 앱 쪽 상태: 서버 상태(접속 수·주소)와 웹에서 들어온 명령 알림. 그리고 렌더러에만
// 있는 캐릭터 라벨(목표·작업 폴더·브랜치·지금 세션 토큰과 추정 비용)을 서버가 떠 있는 동안 백엔드로 밀어 준다.
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import { OFFICE_AGENT_ID } from "@shared/types";
import type { WebDashActivity, WebDashLabel, WebDashStatus, WebDashUsage } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { currentSessionTotals, formatSessionCost, formatSessionTokens, grossTokens } from "../usage/sessionCost";

interface WebDashNotice extends WebDashActivity {
  id: number;
  at: number;
}

interface WebDashState {
  status: WebDashStatus | null;
  notices: WebDashNotice[];
}

export const useWebDashStore = create<WebDashState>(() => ({ status: null, notices: [] }));

/** 알림이 화면에 머무는 시간. */
const NOTICE_MS = 5000;
const LABEL_DEBOUNCE_MS = 1000;
let noticeSeq = 0;

function pushWebDashNotice(activity: WebDashActivity, now = Date.now()): void {
  const notice = { ...activity, id: ++noticeSeq, at: now };
  useWebDashStore.setState((s) => ({ notices: [...s.notices, notice].slice(-3) }));
  setTimeout(() => {
    useWebDashStore.setState((s) => ({ notices: s.notices.filter((n) => n.id !== notice.id) }));
  }, NOTICE_MS);
}

export function setWebDashStatus(status: WebDashStatus): void {
  useWebDashStore.setState({ status });
}

/** 캐릭터의 지금 세션 사용량 표기(터미널 요약 바와 같은 숫자). 설정이 꺼졌거나 아직 쓴 게 없으면 undefined. */
function webDashUsage(agentId: string): WebDashUsage | undefined {
  const { appSettings, sessionUsage, sessionUsageSeed } = useAppStore.getState();
  if (!appSettings.sessionCostEnabled) return undefined;
  const current = currentSessionTotals(sessionUsage, sessionUsageSeed, agentId);
  if (!current) return undefined;
  const { totals } = current;
  if (totals.turns === 0 && grossTokens(totals) === 0) return undefined;
  return {
    sessionId: current.sessionId,
    tokens: formatSessionTokens(totals),
    cost: formatSessionCost(totals),
    turns: totals.turns,
  };
}

/** 스토어의 작업 라벨 → 웹에 보일 라벨. 일반 에이전트는 사용량만 싣는다. */
function webDashLabels(): Record<string, WebDashLabel> {
  const { taskLabels, gitBranches, agents } = useAppStore.getState();
  const out: Record<string, WebDashLabel> = {};
  for (const agentId of Object.keys(agents)) {
    const label = taskLabels[agentId];
    const cwd = label?.cwd ?? agents[agentId]?.cwd;
    const goal = label?.goal ?? label?.goalFallback;
    const branch = cwd ? gitBranches[cwd] : undefined;
    const usage = webDashUsage(agentId);
    if (!goal && !cwd && !usage) continue;
    out[agentId] = { ...(goal ? { goal } : {}), ...(cwd ? { cwd } : {}), ...(branch ? { branch } : {}), ...(usage ? { usage } : {}) };
  }
  const officeUsage = webDashUsage(OFFICE_AGENT_ID);
  if (officeUsage) out[OFFICE_AGENT_ID] = { usage: officeUsage };
  return out;
}

export function installWebDash(): () => void {
  const offs: (() => void)[] = [];
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let lastSent = "";

  const publish = () => {
    timer = undefined;
    if (!useWebDashStore.getState().status?.running) return;
    const labels = webDashLabels();
    const key = JSON.stringify(labels);
    if (key === lastSent) return;
    lastSent = key;
    void tauriApi.webDashPublishLabels(labels).catch((err: unknown) => console.warn("web dash: publish labels failed", err));
  };
  const schedule = () => {
    if (!timer) timer = setTimeout(publish, LABEL_DEBOUNCE_MS);
  };

  const track = (p: Promise<() => void>) => {
    void p.then((off) => (disposed ? off() : offs.push(off)));
  };
  track(listen<WebDashStatus>(Events.webDashStatus, (event) => {
    const wasRunning = useWebDashStore.getState().status?.running;
    setWebDashStatus(event.payload);
    // 서버가 새로 떴으면 라벨을 처음부터 다시 보낸다(백엔드는 빈 상태로 시작한다).
    if (event.payload.running && !wasRunning) lastSent = "";
    schedule();
  }));
  track(listen<WebDashActivity>(Events.webDashActivity, (event) => pushWebDashNotice(event.payload)));
  void tauriApi.webDashStatus().then((status) => {
    if (disposed) return;
    setWebDashStatus(status);
    schedule();
  }).catch(() => {});
  offs.push(useAppStore.subscribe((s, prev) => {
    if (
      s.taskLabels !== prev.taskLabels || s.gitBranches !== prev.gitBranches || s.agents !== prev.agents
      || s.sessionUsage !== prev.sessionUsage || s.sessionUsageSeed !== prev.sessionUsageSeed
      || s.appSettings.sessionCostEnabled !== prev.appSettings.sessionCostEnabled
    ) schedule();
  }));

  return () => {
    if (disposed) return;
    disposed = true;
    if (timer) clearTimeout(timer);
    for (const off of offs.splice(0)) off();
  };
}
