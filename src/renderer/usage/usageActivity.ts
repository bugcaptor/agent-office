// src/renderer/usage/usageActivity.ts
//
// 하단 뱃지 고르기(usageView.pickBadges)의 "관찰된 최근 사용 시각"을
// 앱 재시작 너머로 보존한다. 뷰어별 편의값이라 localStorage에 두고, 읽기·쓰기
// 실패(저장소 없음·파손)는 빈 기록으로 넘어간다 — 그때는 추정값만으로 고른다.
// 스토어(appStore)가 초기값을 여기서 읽으므로 이 모듈은 스토어를 import하지
// 않는다.
import { BADGE_IDS, type UsageActivity } from "./usageView";

const STORAGE_KEY = "agent-office.usageActivity";

export function loadUsageActivity(): UsageActivity {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: UsageActivity = {};
    for (const id of BADGE_IDS) {
      const v = (parsed as Record<string, unknown>)[id];
      if (typeof v === "number" && Number.isFinite(v)) out[id] = v;
    }
    return out;
  } catch {
    return {};
  }
}

export function saveUsageActivity(activity: UsageActivity): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(activity));
  } catch {
    // 저장 실패는 무시한다 — 다음 실행은 추정값으로 시작할 뿐이다.
  }
}
