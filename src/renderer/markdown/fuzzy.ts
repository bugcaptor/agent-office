// src/renderer/markdown/fuzzy.ts
//
// 의존성 없는 자체 퍼지 매칭(이슈 #10 팔레트용). VS Code Ctrl+P 유사 순위:
// 쿼리 문자를 대상 문자열의 부분 수열(subsequence)로 매칭하되, 연속 매치·
// 단어 경계(구분자 뒤, 카멜케이스 전환)·선두 매치에 가산점을 주어 점수를 낸다.
// 파일명 매치는 경로 매치보다 가중치를 높게 둔다(같은 쿼리라도 파일명에 걸린
// 후보가 위로 오게).

/** 파일명 매치에 얹는 가산점 — 경로만 걸린 후보보다 확실히 위로 올린다. */
const NAME_WEIGHT = 12;
/** 매칭된 문자당 기본 점수. */
const BASE = 1;
/** 직전 매치 바로 다음 문자까지 연속으로 걸렸을 때 가산점. */
const CONSECUTIVE = 6;
/** 단어 경계(구분자 뒤/선두/카멜케이스 전환)에서 매치됐을 때 가산점. */
const BOUNDARY = 9;

const SEPARATORS = new Set(["/", "\\", "-", "_", ".", " "]);

/** `target[i]`가 단어 경계(선두, 구분자 뒤, 소문자→대문자 전환)인가. */
function isBoundary(target: string, i: number): boolean {
  if (i === 0) return true;
  const prev = target[i - 1];
  if (SEPARATORS.has(prev)) return true;
  // 카멜케이스 전환: 이전이 소문자/숫자이고 현재가 대문자.
  const cur = target[i];
  const prevLower = prev.toLowerCase() === prev && prev.toUpperCase() !== prev;
  const curUpper = cur.toUpperCase() === cur && cur.toLowerCase() !== cur;
  return prevLower && curUpper;
}

/**
 * `query`를 `target`의 부분 수열로 매칭한 점수. 매칭 불가(쿼리 문자를 다 소비
 * 못함)면 null. 빈 쿼리는 0(중립)을 돌려준다 — 필터 쪽에서 별도 처리한다.
 * 대소문자는 무시하되 경계 판정은 원본 문자열로 한다(카멜케이스 보존).
 */
export function fuzzyScore(query: string, target: string): number | null {
  if (query.length === 0) return 0;
  const q = query.toLowerCase();
  const t = target.toLowerCase();

  let score = 0;
  let qi = 0;
  let prevMatch = -2; // 직전 매치 인덱스(연속 판정용)
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] !== q[qi]) continue;
    let bonus = BASE;
    if (ti === prevMatch + 1) bonus += CONSECUTIVE;
    if (isBoundary(target, ti)) bonus += BOUNDARY;
    score += bonus;
    prevMatch = ti;
    qi++;
  }
  if (qi < q.length) return null; // 쿼리 문자를 다 못 걸었으면 매치 실패
  return score;
}

/** 퍼지 필터 결과 항목(원본 + 점수). 점수는 디버깅/테스트 편의용으로 노출. */
export interface FuzzyRanked<T> {
  item: T;
  score: number;
}

/**
 * `items`를 퍼지 필터·정렬한다. 각 항목의 파일명(`name`)과 경로(`relPath`)를
 * 모두 채점해 더 높은 쪽을 쓰되, 파일명 매치에는 NAME_WEIGHT를 얹는다.
 * 빈 쿼리는 전부 통과시키고 relPath 사전순으로 정렬한다. 동점은 relPath 사전순.
 */
export function fuzzyFilter<T extends { name: string; relPath: string }>(
  items: readonly T[],
  query: string,
): FuzzyRanked<T>[] {
  const trimmed = query.trim();
  if (trimmed.length === 0) {
    return [...items]
      .sort((a, b) => a.relPath.localeCompare(b.relPath))
      .map((item) => ({ item, score: 0 }));
  }

  const ranked: FuzzyRanked<T>[] = [];
  for (const item of items) {
    const nameScore = fuzzyScore(trimmed, item.name);
    const pathScore = fuzzyScore(trimmed, item.relPath);
    if (nameScore === null && pathScore === null) continue;
    const score = Math.max(
      nameScore !== null ? nameScore + NAME_WEIGHT : Number.NEGATIVE_INFINITY,
      pathScore !== null ? pathScore : Number.NEGATIVE_INFINITY,
    );
    ranked.push({ item, score });
  }
  ranked.sort((a, b) => b.score - a.score || a.item.relPath.localeCompare(b.item.relPath));
  return ranked;
}
