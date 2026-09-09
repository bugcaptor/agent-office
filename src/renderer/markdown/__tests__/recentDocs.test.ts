// src/renderer/markdown/__tests__/recentDocs.test.ts
//
// 최근 본 문서 목록의 순수 로직: 중복 승격, 개수 상한, root 필터, 영속 왕복.
import { describe, expect, it } from "vitest";
import {
  MAX_RECENT_DOCS,
  pushRecentDoc,
  recentPathsFor,
  type RecentDoc,
} from "../recentDocs";

describe("recentDocs", () => {
  it("새 문서를 맨 앞에 넣는다", () => {
    const list = pushRecentDoc(pushRecentDoc([], "/r", "a.md", 1), "/r", "b.md", 2);
    expect(recentPathsFor(list, "/r")).toEqual(["b.md", "a.md"]);
  });

  it("같은 문서를 다시 열면 중복 없이 맨 앞으로 올라간다", () => {
    let list: RecentDoc[] = [];
    for (const [p, t] of [["a.md", 1], ["b.md", 2], ["a.md", 3]] as const) {
      list = pushRecentDoc(list, "/r", p, t);
    }
    expect(recentPathsFor(list, "/r")).toEqual(["a.md", "b.md"]);
    expect(list[0].openedAt).toBe(3);
  });

  it("상한을 넘으면 오래된 것부터 잘린다", () => {
    let list: RecentDoc[] = [];
    for (let i = 0; i < MAX_RECENT_DOCS + 5; i++) list = pushRecentDoc(list, "/r", `f${i}.md`, i);
    expect(list).toHaveLength(MAX_RECENT_DOCS);
    expect(list[0].relPath).toBe(`f${MAX_RECENT_DOCS + 4}.md`);
  });

  it("root가 다른 문서는 섞이지 않고 표시 개수만큼만 준다", () => {
    let list: RecentDoc[] = [];
    for (const [root, p, t] of [
      ["/r", "a.md", 1],
      ["/other", "x.md", 2],
      ["/r", "b.md", 3],
      ["/r", "c.md", 4],
    ] as const) {
      list = pushRecentDoc(list, root, p, t);
    }
    expect(recentPathsFor(list, "/r", 2)).toEqual(["c.md", "b.md"]);
    expect(recentPathsFor(list, "/other")).toEqual(["x.md"]);
  });
});

// @vitest-environment jsdom 이 아닌 node 환경에서도 안전해야 한다(localStorage 부재).
describe("recentDocs 영속", () => {
  it("localStorage가 없어도 빈 목록을 주고 저장이 던지지 않는다", async () => {
    const { loadRecentDocs, persistRecentDocs } = await import("../recentDocs");
    expect(loadRecentDocs()).toEqual([]);
    expect(() => persistRecentDocs([{ root: "/r", relPath: "a.md", openedAt: 1 }])).not.toThrow();
  });
});
