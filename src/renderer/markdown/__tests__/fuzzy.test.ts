// src/renderer/markdown/__tests__/fuzzy.test.ts
//
// 퍼지 매칭 순위(이슈 #10). 순수 로직이라 기본 node 환경.
import { describe, expect, it } from "vitest";
import { fuzzyScore, fuzzyFilter, fuzzyRank } from "../fuzzy";

interface Entry {
  relPath: string;
  name: string;
}
function entry(relPath: string): Entry {
  const name = relPath.split("/").pop() ?? relPath;
  return { relPath, name };
}

describe("fuzzyScore", () => {
  it("부분 수열이 아니면 null", () => {
    expect(fuzzyScore("xyz", "readme")).toBeNull();
    expect(fuzzyScore("readx", "readme")).toBeNull();
  });

  it("부분 수열이면 점수(양수)를 낸다", () => {
    expect(fuzzyScore("rdm", "readme")).not.toBeNull();
    expect(fuzzyScore("rdm", "readme")!).toBeGreaterThan(0);
  });

  it("빈 쿼리는 0(중립)", () => {
    expect(fuzzyScore("", "anything")).toBe(0);
  });

  it("연속·선두 매치가 흩어진 매치보다 높은 점수", () => {
    const consecutive = fuzzyScore("read", "readme.md")!; // 선두 연속
    const scattered = fuzzyScore("read", "rxexaxd.md")!; // 흩어짐
    expect(consecutive).toBeGreaterThan(scattered);
  });

  it("단어 경계(구분자 뒤) 매치에 가산점", () => {
    // 'notes'가 경로 세그먼트 선두에 붙는 쪽이 중간에 박힌 쪽보다 높다.
    const boundary = fuzzyScore("notes", "docs/notes.md")!;
    const midword = fuzzyScore("notes", "docs/mynotesx.md")!;
    expect(boundary).toBeGreaterThan(midword);
  });
});

describe("fuzzyFilter", () => {
  it("빈 쿼리는 전부 relPath 사전순으로", () => {
    const items = [entry("b.md"), entry("a.md"), entry("docs/c.md")];
    const out = fuzzyFilter(items, "").map((r) => r.item.relPath);
    expect(out).toEqual(["a.md", "b.md", "docs/c.md"]);
  });

  it("매치 안 되는 항목은 제외", () => {
    const items = [entry("readme.md"), entry("license.txt")];
    const out = fuzzyFilter(items, "read").map((r) => r.item.relPath);
    expect(out).toEqual(["readme.md"]);
  });

  it("파일명 매치가 경로만 매치되는 후보보다 위로", () => {
    // 'read'는 README.md의 파일명에 걸리고, docs/read-team/notes.md는 경로에만 걸린다.
    const items = [entry("docs/read-team/notes.md"), entry("README.md")];
    const out = fuzzyFilter(items, "read").map((r) => r.item.relPath);
    expect(out[0]).toBe("README.md");
  });

  it("동점(빈 쿼리 아님)은 relPath 사전순 타이브레이크", () => {
    // 동일 파일명 'a.md'가 서로 다른 경로에 — 점수 동률이면 relPath 사전순.
    const items = [entry("z/a.md"), entry("a/a.md")];
    const out = fuzzyFilter(items, "a").map((r) => r.item.relPath);
    expect(out).toEqual(["a/a.md", "z/a.md"]);
  });
});

describe("fuzzyRank(이슈 #67 -- 서버사이드 검색 결과 rank-only 정렬)", () => {
  it("fuzzyFilter와 달리 매치 실패 항목도 탈락시키지 않고 남긴다", () => {
    const items = [entry("readme.md"), entry("license.txt")];
    const out = fuzzyRank(items, "read").map((r) => r.item.relPath);
    // fuzzyFilter라면 license.txt는 제외되지만, fuzzyRank는 순위만 매겨 남긴다.
    expect(out).toHaveLength(2);
    expect(out).toContain("license.txt");
  });

  it("매치되는 항목이 매치 안 되는 항목보다 위로 온다", () => {
    const items = [entry("license.txt"), entry("readme.md")];
    const out = fuzzyRank(items, "read").map((r) => r.item.relPath);
    expect(out).toEqual(["readme.md", "license.txt"]);
  });

  it("빈 쿼리는 fuzzyFilter와 동일하게 relPath 사전순", () => {
    const items = [entry("b.md"), entry("a.md"), entry("docs/c.md")];
    const out = fuzzyRank(items, "").map((r) => r.item.relPath);
    expect(out).toEqual(["a.md", "b.md", "docs/c.md"]);
  });

  it("파일명 매치가 경로만 매치되는 후보보다 위로 온다(fuzzyFilter와 동일 가중치)", () => {
    const items = [entry("docs/read-team/notes.md"), entry("README.md")];
    const out = fuzzyRank(items, "read").map((r) => r.item.relPath);
    expect(out[0]).toBe("README.md");
  });
});
