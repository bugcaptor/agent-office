// src/renderer/shared/createListingCache.ts
//
// 이슈 #67: markdownStore/workdirStore가 공통으로 쓰는 "root별 목록 캐시 +
// TTL 재스캔 + in-flight 중복 방지" 로직을 순수 함수/작은 유틸로 뽑아낸 것.
// 스토어 구조(zustand set/get)에는 관여하지 않는다 — 각 스토어가 이 유틸을
// 가져다 자신의 refreshListing 액션 안에서 조합해 쓴다.

/** TTL 판정 대상 캐시 엔트리가 최소로 가져야 하는 필드. */
export interface FetchedAtEntry {
  /** 이 캐시가 마지막으로 채워진 시각(Date.now()). */
  fetchedAt: number;
}

/** entry가 없거나(캐시 미존재) ttlMs보다 오래됐으면 true(=재조회 필요). */
export function isStale<T extends FetchedAtEntry>(entry: T | undefined, ttlMs: number): boolean {
  if (!entry) return true;
  return Date.now() - entry.fetchedAt > ttlMs;
}

/** root(key)별 in-flight 여부를 추적하는 작은 트래커. 스토어 밖 모듈 수준에서
 *  스토어당 하나씩 만들어 두고, refresh 액션 시작/끝에서 begin/end로 감싼다. */
export interface InFlightTracker {
  /** key가 이미 진행 중이면 false(호출자는 스킵). 아니면 진행 중으로 표시하고 true. */
  begin(key: string): boolean;
  /** key의 진행 중 표시를 해제한다(성공/실패 무관하게 finally에서 호출). */
  end(key: string): void;
}

/** 새 in-flight 트래커를 만든다(Set<string> 기반, 모듈 수준에서 1회 생성해 재사용). */
export function createInFlightTracker(): InFlightTracker {
  const keys = new Set<string>();
  return {
    begin(key) {
      if (keys.has(key)) return false;
      keys.add(key);
      return true;
    },
    end(key) {
      keys.delete(key);
    },
  };
}
