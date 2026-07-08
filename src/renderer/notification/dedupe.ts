// src/renderer/notification/dedupe.ts
//
// Pure helper: the ticker shows
// at most one card per agent (the most recent), even though the store's
// `notifications` queue can hold several per agent — this dedupe step runs
// only at render time, the underlying queue is untouched.
import type { Notification } from "../store/types";

/**
 * Keeps only the newest notification per agent, preserving relative order.
 * Input is assumed already sorted newest-first (the store's own invariant —
 * see `appStore.ts`'s `pushNotification`, which prepends), so the first
 * occurrence of an `agentId` encountered while scanning is its latest one.
 */
export function dedupeLatestPerAgent(list: readonly Notification[]): Notification[] {
  const seen = new Set<string>();
  const out: Notification[] = [];
  for (const n of list) {
    if (seen.has(n.agentId)) continue;
    seen.add(n.agentId);
    out.push(n);
  }
  return out;
}
