import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  formatBytes,
  isValidAgentId,
  listAgentDirs,
  listLogFiles,
  logRootExists,
  parseLogFileName,
  readLogHeader,
  readTailSlice,
  sessionLogRoot,
} from "./logFiles";

let root: string;

beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "agent-office-ext-"));
});

afterEach(async () => {
  await fs.rm(root, { recursive: true, force: true });
});

/** 앱이 쓰는 것과 같은 헤더 + 본문으로 로그 파일을 만든다. */
async function writeLog(agentId: string, name: string, body: string): Promise<string> {
  const dir = path.join(root, agentId);
  await fs.mkdir(dir, { recursive: true });
  const file = path.join(dir, name);
  const header =
    "# agent-office session log v1\n" +
    `# agentId: ${agentId}\n` +
    "# sessionId: 569c7a13-dead-beef\n" +
    "# cwd: /Users/me/dev/proj\n" +
    "# started: 2026-07-25T19:10:54+09:00\n\n";
  await fs.writeFile(file, header + body);
  return file;
}

describe("sessionLogRoot", () => {
  it("<app_data>/session-logs/v1", () => {
    expect(sessionLogRoot("/a")).toBe(path.join("/a", "session-logs", "v1"));
  });
});

describe("parseLogFileName", () => {
  it("<stamp>-<sid8>.log을 쪼개고 표시 라벨을 만든다", () => {
    const parsed = parseLogFileName("20260725-191054-569c7a13.log");
    expect(parsed.stamp).toBe("20260725-191054");
    expect(parsed.sid8).toBe("569c7a13");
    expect(parsed.label).toBe("07/25 19:10");
    // stamp는 로컬 시간이므로 로컬 컴포넌트로 되돌려야 한다.
    const at = new Date(parsed.startedAt!);
    expect([at.getFullYear(), at.getMonth() + 1, at.getDate(), at.getHours(), at.getMinutes()]).toEqual(
      [2026, 7, 25, 19, 10],
    );
  });

  it("sessionId가 없던 세션의 nosessid도 받는다", () => {
    expect(parseLogFileName("20260726-142718-nosessid.log").sid8).toBe("nosessid");
  });

  it("규약을 벗어난 이름은 라벨을 파일명으로 되돌린다", () => {
    const parsed = parseLogFileName("weird.log");
    expect(parsed.sid8).toBe("");
    expect(parsed.startedAt).toBeUndefined();
    expect(parsed.label).toBe("weird.log");
  });
});

describe("isValidAgentId", () => {
  it("경로 탈출과 study를 막는다", () => {
    expect(isValidAgentId("kfJ7r_Kub6Vg6uh0xowBl")).toBe(true);
    expect(isValidAgentId("")).toBe(false);
    expect(isValidAgentId("a/b")).toBe(false);
    expect(isValidAgentId("a\\b")).toBe(false);
    expect(isValidAgentId("../evil")).toBe(false);
    expect(isValidAgentId("study")).toBe(false);
  });
});

describe("listLogFiles", () => {
  it("파일명 역순(=최신순)으로 돌려주고 .log만 센다", async () => {
    await writeLog("a1", "20260725-191054-569c7a13.log", "old\n");
    await writeLog("a1", "20260727-082853-f0f2dba5.log", "new\n");
    await writeLog("a1", "20260726-142718-25ef8f70.log", "mid\n");
    await fs.writeFile(path.join(root, "a1", "notes.md"), "not a log");

    const files = await listLogFiles(root, "a1");
    expect(files.map((f) => f.fileName)).toEqual([
      "20260727-082853-f0f2dba5.log",
      "20260726-142718-25ef8f70.log",
      "20260725-191054-569c7a13.log",
    ]);
    expect(files[0].bytes).toBeGreaterThan(0);
    expect(files[0].agentId).toBe("a1");
  });

  it("없는 캐릭터·잘못된 agentId는 에러가 아니라 빈 목록", async () => {
    expect(await listLogFiles(root, "nobody")).toEqual([]);
    expect(await listLogFiles(root, "study")).toEqual([]);
    expect(await listLogFiles(root, "../evil")).toEqual([]);
  });
});

describe("listAgentDirs / logRootExists", () => {
  it("study는 캐릭터가 아니다", async () => {
    await writeLog("a1", "20260725-191054-569c7a13.log", "x\n");
    await fs.mkdir(path.join(root, "study"), { recursive: true });
    await fs.mkdir(path.join(root, "a2"), { recursive: true });
    expect(await listAgentDirs(root)).toEqual(["a1", "a2"]);
  });

  it("로그 루트가 없으면 false", async () => {
    expect(await logRootExists(root)).toBe(true);
    expect(await logRootExists(path.join(root, "missing"))).toBe(false);
  });
});

describe("readLogHeader", () => {
  it("앞부분만 읽어 sessionId/cwd를 뽑는다", async () => {
    const file = await writeLog("a1", "20260725-191054-569c7a13.log", "본문\n");
    const header = await readLogHeader(file);
    expect(header.agentId).toBe("a1");
    expect(header.sessionId).toBe("569c7a13-dead-beef");
    expect(header.cwd).toBe("/Users/me/dev/proj");
    expect(header.started).toBe("2026-07-25T19:10:54+09:00");
  });

  it("헤더가 없어도 빈 값으로 흘린다", async () => {
    const dir = path.join(root, "a1");
    await fs.mkdir(dir, { recursive: true });
    const file = path.join(dir, "20260725-191054-569c7a13.log");
    await fs.writeFile(file, "그냥 본문\n");
    expect(await readLogHeader(file)).toEqual({});
    expect(await readLogHeader(path.join(dir, "missing.log"))).toEqual({});
  });
});

describe("readTailSlice", () => {
  it("작은 파일은 그대로 준다", async () => {
    const file = await writeLog("a1", "20260725-191054-569c7a13.log", "한 줄\n두 줄\n");
    const slice = await readTailSlice(file, 512);
    expect(slice.truncated).toBe(false);
    expect(slice.omittedBytes).toBe(0);
    expect(slice.text).toContain("한 줄\n두 줄\n");
  });

  it("잘릴 때는 줄 경계에 맞추고 안내 첫 줄을 넣는다", async () => {
    // 각 줄 20바이트 × 2000줄 ≈ 40KB.
    const lines = Array.from({ length: 2000 }, (_, i) => `line-${String(i).padStart(6, "0")}xxxxx`);
    const file = await writeLog("a1", "20260725-191054-569c7a13.log", `${lines.join("\n")}\n`);

    const slice = await readTailSlice(file, 4); // 4KB만
    expect(slice.truncated).toBe(true);
    expect(slice.omittedBytes).toBeGreaterThan(0);

    const out = slice.text.split("\n");
    expect(out[0]).toMatch(/^⋯ \(앞 .+ 생략 — "전체 불러오기"로 처음부터\)$/);
    // 두 번째 줄은 반드시 온전한 줄이어야 한다(반토막 금지).
    expect(out[1]).toMatch(/^line-\d{6}xxxxx$/);
    // 마지막 내용 줄은 파일의 마지막 줄이다.
    expect(out[out.length - 2]).toBe("line-001999xxxxx");
    expect(slice.totalBytes).toBeGreaterThan(4 * 1024);
    // 잘려 나간 앞부분 + 남긴 본문(바이트) = 전체.
    const kept = Buffer.byteLength(slice.text.slice(out[0].length + 1), "utf8");
    expect(slice.omittedBytes + kept).toBe(slice.totalBytes);
  });

  it("멀티바이트 문자를 반토막 내지 않는다", async () => {
    const lines = Array.from({ length: 1000 }, (_, i) => `${i}번째 한글 줄입니다`);
    const file = await writeLog("a1", "20260725-191054-569c7a13.log", `${lines.join("\n")}\n`);
    const slice = await readTailSlice(file, 4);
    expect(slice.truncated).toBe(true);
    expect(slice.text).not.toContain("�");
    const out = slice.text.split("\n");
    expect(out[1]).toMatch(/^\d+번째 한글 줄입니다$/);
  });

  it("개행이 전혀 없는 거대한 한 줄이면 자른 그대로 쓴다", async () => {
    const dir = path.join(root, "a1");
    await fs.mkdir(dir, { recursive: true });
    const file = path.join(dir, "20260725-191054-569c7a13.log");
    await fs.writeFile(file, "x".repeat(10 * 1024));
    const slice = await readTailSlice(file, 1);
    expect(slice.truncated).toBe(true);
    expect(slice.omittedBytes).toBe(9 * 1024);
    expect(slice.text.split("\n")[1]).toBe("x".repeat(1024));
  });
});

describe("formatBytes", () => {
  it("사람이 읽는 크기", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2 KB");
    expect(formatBytes(1024 * 1024 * 3.5)).toBe("3.5 MB");
    expect(formatBytes(1024 * 1024 * 42)).toBe("42 MB");
  });
});
