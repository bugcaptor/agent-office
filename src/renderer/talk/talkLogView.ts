// src/renderer/talk/talkLogView.ts
//
// 대화 감사 로그(`<app_data>/talks/YYYY-MM-DD.jsonl`)를 화면에 올리기 전의
// 순수 변환. 로그는 시간순 한 줄씩 쌓이는데 여러 대화가 뒤섞여 있으므로,
// 여기서 convId 단위로 묶고 각 묶음 안을 다시 시간순으로 정렬한다.
// DOM/스토어 의존이 없어 단위 테스트가 쉽다(analytics/aggregate.ts와 같은 관례).
import type { TalkLogEntry } from "@shared/types";

/** convId 하나로 묶인 대화. */
export interface TalkLogGroup {
  convId: string;
  /** 이 대화의 가장 이른 기록 시각 — 묶음 정렬 기준. */
  startedAt: number;
  /** 시간 오름차순(같은 시각이면 파일 순서 유지). */
  entries: TalkLogEntry[];
}

/**
 * 로그 줄들을 대화(convId) 단위로 묶는다. 묶음은 시작 시각 오름차순,
 * 묶음 안은 시각 오름차순(동시각은 원래 순서 유지 = 안정 정렬).
 */
export function groupByConversation(entries: readonly TalkLogEntry[]): TalkLogGroup[] {
  const byConv = new Map<string, TalkLogEntry[]>();
  for (const e of entries) {
    const bucket = byConv.get(e.convId);
    if (bucket) bucket.push(e);
    else byConv.set(e.convId, [e]);
  }
  const groups: TalkLogGroup[] = [];
  for (const [convId, list] of byConv) {
    // Array.prototype.sort는 ES2019부터 안정 정렬 — 동시각 줄의 파일 순서가 보존된다.
    const sorted = [...list].sort((a, b) => a.at - b.at);
    groups.push({ convId, startedAt: sorted[0].at, entries: sorted });
  }
  return groups.sort((a, b) => a.startedAt - b.startedAt);
}

/** 대화 참가자 이름을 등장 순으로 모은다(표시용 헤더). `resolveName`은 agentId -> 이름. */
export function participantsOf(
  group: TalkLogGroup,
  resolveName: (agentId: string) => string,
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const e of group.entries) {
    for (const [id, name] of [
      [e.from, e.fromName] as const,
      [e.to, resolveName(e.to)] as const,
    ]) {
      if (seen.has(id)) continue;
      seen.add(id);
      out.push(name);
    }
  }
  return out;
}

/** 로그 종류 표시 문구. 알 수 없는 값은 그대로 보여 준다(백엔드가 늘려도 안 깨지게). */
export function kindLabel(kind: string): string {
  switch (kind) {
    case "send":
      return "말함";
    case "deliver":
      return "전달됨";
    case "expire":
      return "전달 실패(만료)";
    default:
      return kind;
  }
}

/** 하루치 로그라 날짜는 생략하고 시:분:초만. */
export function formatTalkTime(at: number): string {
  const d = new Date(at);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}
