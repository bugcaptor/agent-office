// src/renderer/labels/gitBranchWatcher.ts
//
// 라벨 표면("프로젝트 (브랜치) · 목표")이 쓰는 cwd→브랜치 맵을 채워주는 폴링
// 브리지. 살아 있는(starting/running) 에이전트의 유니크한 실효 cwd만 대상으로
// `workdirGitBranch`를 30초 간격으로 부르고, 새 cwd가 나타난 순간에는 다음
// 틱을 기다리지 않고 즉시 한 번 조회한다(탭을 새로 열자마자 브랜치가 뜨도록).
//
// 왜 폴링인가: 브랜치는 앱 밖(터미널의 `git switch`)에서 바뀌므로 앱이 관측할
// 이벤트가 없다. 반대로 라벨은 장식이라 실시간성이 필요 없어, 30초면 충분하고
// 조회 자체도 `.git/HEAD` 한 번 읽기(백엔드 타임아웃 2초)라 부담이 없다.
//
// 갱신 로직은 전부 순수 함수(liveAgentCwds/nextGitBranches)로 빼고, 여기서는
// 타이머·구독·in-flight 가드만 맡는다. `setInterval`은 전역(bootstrap.ts의
// installSnapshotUploader와 동일 컨벤션) — `window` 없는 Node 테스트에서도 안전.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { effectiveCwd } from "./labelText";
import type { AgentProfile, AgentTaskLabel, SessionRuntime } from "../store/types";

/** 브랜치 폴링 간격(ms). 라벨은 장식이라 실시간성이 필요 없다. */
export const GIT_BRANCH_POLL_INTERVAL_MS = 30_000;

/** 폴링·가지치기가 보는 스토어 조각(테스트가 통짜 스토어 없이 부를 수 있게). */
export interface GitBranchSource {
  agents: Record<string, AgentProfile>;
  sessions: Record<string, Pick<SessionRuntime, "status">>;
  taskLabels: Record<string, AgentTaskLabel>;
}

/**
 * 조회 대상 cwd 목록: 라이브(starting/running) 세션을 가진 에이전트의 실효
 * cwd(`taskLabels[id]?.cwd ?? agents[id].cwd`)를 중복 없이 모은다. 종료된 탭은
 * 대상이 아니다 — 그 라벨은 이미 stale이라 브랜치를 새로 물을 이유가 없다.
 * 순서는 `agents` 열거 순서(안정적) 그대로라 결과 비교가 흔들리지 않는다.
 */
export function liveAgentCwds(s: GitBranchSource): string[] {
  const seen = new Set<string>();
  for (const id of Object.keys(s.agents)) {
    const status = s.sessions[id]?.status;
    if (status !== "starting" && status !== "running") continue;
    const cwd = effectiveCwd(s.taskLabels[id], s.agents[id]?.cwd);
    if (cwd) seen.add(cwd);
  }
  return [...seen];
}

/**
 * 폴링 1회분 결과를 이전 맵에 반영한다. `live`에 없는 키는 떨군다(종료된 탭의
 * cwd가 영원히 쌓이지 않게), `live`에 있지만 이번에 조회하지 않은 키는 이전
 * 값을 유지한다(조회 중 새로 뜬 탭이 다음 틱까지 깜빡이지 않게).
 * `branch`가 null인 결과 — 비저장소·detached HEAD·조회 실패 — 는 키를 지운다:
 * 스토어 계약이 "키가 있으면 표시할 브랜치가 있다"이기 때문.
 */
export function nextGitBranches(
  prev: Record<string, string>,
  results: ReadonlyArray<{ cwd: string; branch: string | null }>,
  live: ReadonlyArray<string>
): Record<string, string> {
  const liveSet = new Set(live);
  const next: Record<string, string> = {};
  for (const [cwd, branch] of Object.entries(prev)) {
    if (liveSet.has(cwd)) next[cwd] = branch;
  }
  for (const r of results) {
    if (!liveSet.has(r.cwd)) continue;
    if (r.branch) next[r.cwd] = r.branch;
    else delete next[r.cwd];
  }
  return next;
}

/** 맵 두 개가 같은 내용인가(불필요한 setState로 라벨을 리렌더하지 않기 위해). */
function sameBranches(a: Record<string, string>, b: Record<string, string>): boolean {
  const ka = Object.keys(a);
  if (ka.length !== Object.keys(b).length) return false;
  return ka.every((k) => a[k] === b[k]);
}

/**
 * cwd들의 브랜치를 병렬 조회한다. 개별 실패는 `branch: null`로 접어 한 폴더의
 * 오류가 나머지 조회를 무너뜨리지 않게 한다(백엔드도 비저장소를 에러가 아니라
 * `isRepo=false`로 답하므로, 여기 catch에 걸리는 건 IPC 자체 장애뿐이다).
 */
async function queryBranches(
  cwds: ReadonlyArray<string>
): Promise<Array<{ cwd: string; branch: string | null }>> {
  return await Promise.all(
    cwds.map(async (cwd) => {
      try {
        const r = await tauriApi.workdirGitBranch(cwd);
        return { cwd, branch: r.isRepo ? r.branch : null };
      } catch {
        return { cwd, branch: null };
      }
    })
  );
}

/**
 * 폴링을 설치하고 teardown을 돌려준다(bootstrap에서 1회 호출). 30초 타이머와,
 * "새 cwd 등장 → 즉시 1회" 스토어 구독 두 갈래가 같은 `refresh`로 모인다.
 *
 * 중복 방지: `inFlight`로 조회가 겹치지 않게 한다(30초보다 오래 걸릴 일은
 * 없지만, 즉시 조회와 타이머가 같은 순간에 겹칠 수는 있다). `disposed`는
 * teardown 이후 늦게 해소된 조회가 스토어를 건드리지 못하게 막는다.
 */
export function installGitBranchWatcher(): () => void {
  let disposed = false;
  let inFlight = false;

  const refresh = async (): Promise<void> => {
    if (disposed || inFlight) return;
    const cwds = liveAgentCwds(useAppStore.getState());
    if (cwds.length === 0) {
      // 라이브 탭이 없으면 조회는 건너뛰되, 남아 있는 항목은 정리한다.
      const pruned = nextGitBranches(useAppStore.getState().gitBranches, [], []);
      if (!sameBranches(useAppStore.getState().gitBranches, pruned)) {
        useAppStore.getState().setGitBranches(pruned);
      }
      return;
    }
    inFlight = true;
    try {
      const results = await queryBranches(cwds);
      if (disposed) return;
      // 조회 사이에 탭이 바뀌었을 수 있어 live 목록을 다시 읽는다.
      const state = useAppStore.getState();
      const next = nextGitBranches(state.gitBranches, results, liveAgentCwds(state));
      if (!sameBranches(state.gitBranches, next)) state.setGitBranches(next);
    } finally {
      inFlight = false;
    }
  };

  const timer = setInterval(() => {
    void refresh();
  }, GIT_BRANCH_POLL_INTERVAL_MS);

  // 라이브 cwd 집합이 바뀌면(탭 시작/종료, 세션 cwd 확정) 즉시 한 번 더 조회한다.
  // 셀렉터는 문자열을 돌려줘 zustand 기본 Object.is 비교가 그대로 먹는다 —
  // 배열을 돌려주면 매 setState마다 새 참조라 리스너가 항상 발화한다.
  const unsubscribe = useAppStore.subscribe(
    (s) => liveAgentCwds(s).join("\n"),
    () => {
      void refresh();
    }
  );

  // 부팅 시점에 이미 라이브 탭이 있을 수 있다(세션 입양) — 첫 조회를 즉시.
  void refresh();

  return () => {
    disposed = true;
    clearInterval(timer);
    unsubscribe();
  };
}
