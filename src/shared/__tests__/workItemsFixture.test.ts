// `work-items-file.json` 픽스처가 TS `WorkItem`의 필드를 모두 담는지 본다.
// 픽스처가 Rust `WorkItem`으로 그대로 왕복하는지는 src-tauri/tests/contract_fixtures.rs가 본다.
// 둘을 합치면, TS에 필드를 더하고 Rust 미러를 빠뜨렸을 때 저장 왕복에서 값이 조용히
// 사라지는 대신 테스트가 실패한다.
import { describe, expect, it } from "vitest";
import fixture from "@shared/contract-fixtures/work-items-file.json";
import type { WorkItem } from "../types/workPanels";

// `WorkItem`에 키를 더하면 여기서 타입 검사가 실패한다 — 픽스처의 첫 업무에도 그 키를 채울 것.
const WORK_ITEM_KEYS: Record<keyof WorkItem, true> = {
  id: true,
  title: true,
  body: true,
  assigneeAgentId: true,
  agent: true,
  status: true,
  order: true,
  sessionIds: true,
  lastError: true,
  dependsOn: true,
  plan: true,
  maxFollowUps: true,
  permissionWaitMinutes: true,
  autoRun: true,
  schedule: true,
  scheduledFrom: true,
  createdAt: true,
  updatedAt: true,
};

describe("work-items-file.json contract fixture", () => {
  it("fills every WorkItem field in the full item", () => {
    expect(Object.keys(fixture[0]).sort()).toEqual(Object.keys(WORK_ITEM_KEYS).sort());
  });
});
