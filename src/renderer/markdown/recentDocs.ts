// src/renderer/markdown/recentDocs.ts
//
// 문서 보기에서 최근 연 문서 기록. 팔레트를 검색어 없이 열었을 때 최근 본
// 문서 몇 개를 맨 위로 올려 주기 위한 순수 로직 + localStorage 영속이다.
// 스토어/Tauri 의존이 없어 테스트에서 그대로 부를 수 있다(terminalViewMode.ts와 같은 결).
//
// 목록은 root를 가리지 않고 한 배열에 최신순으로 쌓고, 표시할 때 root로 거른다.
// 다른 저장소를 오가도 각자의 최근 문서가 남는다.

export interface RecentDoc {
  root: string;
  relPath: string;
  /** 마지막으로 연 시각(Date.now()). */
  openedAt: number;
}

export const RECENT_DOCS_STORAGE_KEY = "agent-office.markdown.recent-docs";

/** 영속하는 최대 개수(여러 저장소를 오가도 각자 몇 개는 남을 만큼). */
export const MAX_RECENT_DOCS = 40;

/** 팔레트 상단에 끌어올릴 개수. */
export const RECENT_DOCS_SHOWN = 4;

function isRecentDoc(v: unknown): v is RecentDoc {
  if (typeof v !== "object" || v === null) return false;
  const d = v as Record<string, unknown>;
  return typeof d.root === "string" && typeof d.relPath === "string" && typeof d.openedAt === "number";
}

/** 같은 문서는 하나만 남기고 맨 앞으로 올린다. 오래된 꼬리는 잘라낸다. */
export function pushRecentDoc(
  list: readonly RecentDoc[],
  root: string,
  relPath: string,
  now: number = Date.now(),
): RecentDoc[] {
  const rest = list.filter((d) => !(d.root === root && d.relPath === relPath));
  return [{ root, relPath, openedAt: now }, ...rest].slice(0, MAX_RECENT_DOCS);
}

/** 해당 root의 최근 문서 경로를 최신순으로 최대 `limit`개. */
export function recentPathsFor(
  list: readonly RecentDoc[],
  root: string,
  limit: number = RECENT_DOCS_SHOWN,
): string[] {
  return list
    .filter((d) => d.root === root)
    .slice(0, limit)
    .map((d) => d.relPath);
}

/** 저장된 목록을 읽는다. 없거나 깨졌으면 빈 배열. localStorage 부재(node)도 안전. */
export function loadRecentDocs(): RecentDoc[] {
  try {
    const raw = localStorage.getItem(RECENT_DOCS_STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isRecentDoc).slice(0, MAX_RECENT_DOCS);
  } catch {
    return [];
  }
}

/** 목록을 localStorage에 영속한다. 저장 불가 환경에서는 조용히 무시. */
export function persistRecentDocs(list: readonly RecentDoc[]): void {
  try {
    localStorage.setItem(RECENT_DOCS_STORAGE_KEY, JSON.stringify(list));
  } catch {
    // 프라이빗 모드/노드 환경 등 저장 불가 — 화면 동작은 유효하므로 무시.
  }
}
