// src/renderer/diary/workLogPersister.ts
//
// 캐릭터 일기(#60) 작업 로그 영속화기. 렌더러 버퍼(`workLog`)는 원래 비영속이라
// 앱을 껐다 켜면 미기록 세션 로그가 유실됐다. 이 모듈이 버퍼 변경을 감지해
// 백엔드 스냅샷(`worklogs/<agentId>.json`)에 디바운스 저장하고, 부팅 시 복원한다.
//
// 게이트: `appSettings.diaryEnabled`가 **켜져 있을 때만** 디스크에 쓴다 — 일기
// 기능을 안 쓰는 사람에게 디스크 쓰기를 강요하지 않는다(요구사항). OFF인 동안의
// 변경은 `dirtyWhileOff`에 모아두고, OFF→ON 전환 시 일괄 저장한다. ON→OFF는 쓰기만
// 멈추고 기존 파일은 지우지 않는다(실수 토글로 밀린 로그를 날리지 않기 위해).
//
// 읽기(복원)는 게이트와 무관하게 항상 시도한다 — 읽기는 공짜고, 나중에 ON을 켜면
// 그대로 살아난다. 복원 시 너무 오래된(RESTORE_MAX_AGE_MS 초과) 항목은 프루닝한다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { workLog, type WorkLog } from "./workLog";
import type { WorkLogItem } from "@shared/types";

/** 버퍼 변경 후 디스크 저장까지의 트레일링 디바운스(ms). */
export const PERSIST_DEBOUNCE_MS = 1000;
/** 복원 시 이보다 오래된 항목은 버린다(불멸 쓰레기 방지). 자동 생성 컷오프(3일)보다
 *  넉넉히 잡아, 3~14일 구간은 수동 생성 여지를 남긴다. */
export const RESTORE_MAX_AGE_MS = 14 * 24 * 60 * 60 * 1000; // 14일

/**
 * 부팅 복원으로 버퍼에 채워진 `${agentId}:${sessionId}` 키 집합. diaryFlusher가
 * 이 세션들만 "이미 일기가 있는지" 중복 검사한다 — "appendDiaryEntry는 됐는데 로그
 * clear 스냅샷은 디스크에 못 간 채 크래시"한 창에서 같은 세션이 두 번 일기화되는
 * 걸 막는다(§1-4). 런타임에 새로 생긴 세션은 메모리 상태가 정본이라 불필요.
 * flusher가 처리(생성/스킵)하면 키를 지운다.
 */
export const restoredSessionKeys = new Set<string>();

export interface WorkLogPersisterDeps {
  /** 주입용 버퍼(테스트). 기본은 전역 workLog. */
  log?: WorkLog;
  /** 스냅샷 저장(테스트 주입). 기본은 tauriApi.saveWorkLog. */
  save?: (agentId: string, items: WorkLogItem[]) => Promise<void>;
  /** 디바운스 ms 오버라이드(테스트). */
  debounceMs?: number;
}

/** 살아있는 영속화기 핸들. install이 돌려준다. */
export interface WorkLogPersister {
  /** 펜딩(디바운스 대기 중)인 저장을 전부 즉시 수행하고 완료를 기다린다.
   *  앱 정상 종료 직전에 호출 — 마지막 변경분까지 디스크에 안착시킨다. */
  flushNow(): Promise<void>;
  /** 구독·타이머를 해제한다. */
  dispose(): void;
}

/**
 * 작업 로그 영속화기를 설치한다. 앱 부트에서 1회 호출(bootstrap.ts).
 * `workLog.setOnChange`로 버퍼 변경을 구독하고, appSettings를 구독해 diaryEnabled
 * 전환을 감지한다.
 */
/** 현재 설치된 영속화기(앱당 1개). 종료 경로가 flushNow를 부르려고 전역 접근한다. */
let active: WorkLogPersister | null = null;

/** 설치된 영속화기 핸들(없으면 null). 종료 flush가 `flushNow`를 부를 때 쓴다. */
export function activeWorkLogPersister(): WorkLogPersister | null {
  return active;
}

export function installWorkLogPersister(deps: WorkLogPersisterDeps = {}): WorkLogPersister {
  const log = deps.log ?? workLog;
  const save = deps.save ?? ((id, items) => tauriApi.saveWorkLog(id, items));
  const debounceMs = deps.debounceMs ?? PERSIST_DEBOUNCE_MS;

  // 디바운스 저장이 걸려 있는 agentId → 타이머.
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  // OFF인 동안 변경돼 아직 저장 못 한 agentId — OFF→ON 시 일괄 저장.
  const dirtyWhileOff = new Set<string>();

  const diaryEnabled = () => useAppStore.getState().appSettings.diaryEnabled;

  /** 한 캐릭터의 현재 버퍼를 디스크에 반영. items 비면 백엔드가 파일 삭제. */
  async function saveAgent(agentId: string): Promise<void> {
    timers.delete(agentId);
    // 타이머가 OFF 전환 이후에 발화한 경우 — 쓰지 말고 OFF 목록으로 이관.
    if (!diaryEnabled()) {
      dirtyWhileOff.add(agentId);
      return;
    }
    try {
      await save(agentId, log.items(agentId));
    } catch (err) {
      console.warn(`workLogPersister: 스냅샷 저장 실패(agent=${agentId})`, err);
    }
  }

  function scheduleSave(agentId: string): void {
    const existing = timers.get(agentId);
    if (existing) clearTimeout(existing);
    timers.set(
      agentId,
      setTimeout(() => void saveAgent(agentId), debounceMs),
    );
  }

  // 버퍼 변경 훅. ON이면 디바운스 저장, OFF면 dirtyWhileOff에 모은다.
  log.setOnChange((agentId) => {
    if (diaryEnabled()) scheduleSave(agentId);
    else dirtyWhileOff.add(agentId);
  });

  // diaryEnabled 전환 감지. OFF→ON이면 그동안 밀린 변경을 일괄 저장.
  let prevEnabled = diaryEnabled();
  const unsubSettings = useAppStore.subscribe(
    (s) => s.appSettings.diaryEnabled,
    (enabled) => {
      if (enabled && !prevEnabled) {
        for (const agentId of dirtyWhileOff) scheduleSave(agentId);
        dirtyWhileOff.clear();
      }
      prevEnabled = enabled;
    },
  );

  const handle: WorkLogPersister = {
    async flushNow() {
      const pending = [...timers.keys()];
      for (const agentId of pending) {
        const t = timers.get(agentId);
        if (t) clearTimeout(t);
      }
      await Promise.all(pending.map((agentId) => saveAgent(agentId)));
    },
    dispose() {
      for (const t of timers.values()) clearTimeout(t);
      timers.clear();
      dirtyWhileOff.clear();
      log.setOnChange(undefined);
      unsubSettings();
      if (active === handle) active = null;
    },
  };
  active = handle;
  return handle;
}

export interface RestoreWorkLogsDeps {
  log?: WorkLog;
  loadAll?: () => Promise<Record<string, WorkLogItem[]>>;
  save?: (agentId: string, items: WorkLogItem[]) => Promise<void>;
  now?: () => number;
  maxAgeMs?: number;
}

/**
 * 디스크 스냅샷을 버퍼로 복원한다. 앱 부트에서 영속화기 설치 전에 1회 호출.
 * RESTORE_MAX_AGE_MS 초과 항목은 버리고, 프루닝으로 내용이 바뀌었고 diaryEnabled면
 * 디스크도 정리한다(오래된 스냅샷이 영영 남지 않게). 읽기 자체는 게이트와 무관.
 */
export async function restoreWorkLogs(deps: RestoreWorkLogsDeps = {}): Promise<void> {
  const log = deps.log ?? workLog;
  const loadAll = deps.loadAll ?? (() => tauriApi.loadWorkLogs());
  const save = deps.save ?? ((id, items) => tauriApi.saveWorkLog(id, items));
  const now = deps.now ?? Date.now;
  const maxAgeMs = deps.maxAgeMs ?? RESTORE_MAX_AGE_MS;

  let all: Record<string, WorkLogItem[]>;
  try {
    all = await loadAll();
  } catch (err) {
    console.warn("workLogPersister: 작업 로그 복원 로드 실패", err);
    return;
  }

  const cutoff = now() - maxAgeMs;
  const enabled = useAppStore.getState().appSettings.diaryEnabled;
  for (const [agentId, items] of Object.entries(all)) {
    const fresh = items.filter((i) => i.at >= cutoff);
    log.seed(agentId, fresh);
    // 복원된 세션을 중복 검사 대상으로 등록(flusher가 처리 시 지운다).
    for (const i of fresh) restoredSessionKeys.add(`${agentId}:${i.sessionId}`);
    // 프루닝으로 줄었으면 디스크도 맞춰 정리(ON일 때만 — 게이트 존중).
    if (enabled && fresh.length !== items.length) {
      try {
        await save(agentId, fresh);
      } catch (err) {
        console.warn(`workLogPersister: 복원 프루닝 저장 실패(agent=${agentId})`, err);
      }
    }
  }
}
