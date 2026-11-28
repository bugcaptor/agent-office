// scripts/dash-files-shot.mjs
//
// 웹 상태판 "파일" 탭을 가짜 API로 띄워 폰 크기로 찍고 흐름을 확인한다. 앱·Tauri 없이 화면만 본다.
//   node scripts/dash-files-shot.mjs [출력 폴더(기본 /tmp/dash-files-shot)]
// 찍는 것: 기록(보내는 중·받음·멈춤), 이 기기에서 올려 캐릭터에게 보내기(4MB 청크 PUT 뒤 send), 머신에서 고르기(받은 파일 훑기).
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const outDir = resolve(process.argv[2] || "/tmp/dash-files-shot");
const files = {
  "/": ["src-tauri/web/dash.html", "text/html; charset=utf-8"],
  "/dash.css": ["src-tauri/web/dash.css", "text/css; charset=utf-8"],
  "/dash.js": ["src-tauri/web/dash.js", "text/javascript; charset=utf-8"],
  "/tasks.js": ["src-tauri/web/tasks.js", "text/javascript; charset=utf-8"],
  "/shell.js": ["src-tauri/web/shell.js", "text/javascript; charset=utf-8"],
  "/files.js": ["src-tauri/web/files.js", "text/javascript; charset=utf-8"],
};
const assets = Object.fromEntries(await Promise.all(Object.entries(files).map(async ([k, [f, type]]) => [k, [await readFile(resolve(f), "utf8"), type]])));
const now = Date.now();
const HOST = { machineId: "m-host", machineName: "집 맥미니" };
const PC = { machineId: "m-pc", machineName: "studio-pc" };
const entry = (path, size) => ({ path, size, sha256: "0".repeat(64), mtime: 0 });
const overview = {
  status: { config: { receiveEnabled: true }, receiveDir: "/Users/me/Downloads/Agent Office", machineId: "m-host", machineName: "집 맥미니", linked: true },
  targets: [
    { ...HOST, local: true, online: true, receive: true },
    { ...HOST, agentId: "a", agentName: "에이다", local: true, online: true, receive: true },
    { ...PC, deviceId: "d-1", local: false, online: true, receive: true },
    { ...PC, agentId: "r1", agentName: "보리", deviceId: "d-1", local: false, online: true, receive: true },
    { machineId: "m-old", machineName: "old-laptop", deviceId: "d-2", local: false, online: false, receive: false },
  ],
  sources: [{ machineId: "m-host", name: "집 맥미니", local: true, online: true }, { machineId: "m-pc", name: "studio-pc", deviceId: "d-1", local: false, online: true }],
  transfers: [
    { id: "f-1", createdAt: now - 60000, updatedAt: now, from: HOST, to: { ...PC, agentId: "r1", agentName: "보리" }, items: ["dataset"],
      files: [entry("dataset/a.parquet", 900e6), entry("dataset/b.parquet", 600e6)], totalBytes: 1.5e9, doneBytes: 0.63e9, state: "sending", notify: true, note: "이 분할로 학습", via: "app" },
    { id: "f-2", createdAt: now - 3600000, updatedAt: now, from: { ...PC, agentId: "r1", agentName: "보리" }, to: { ...HOST, agentId: "a", agentName: "에이다" }, items: ["report.pdf"],
      files: [entry("report.pdf", 2.4e6)], totalBytes: 2.4e6, doneBytes: 2.4e6, state: "done", notify: true, saved: ["/Users/me/Downloads/Agent Office/에이다/report.pdf"], via: "agent" },
    { id: "f-3", createdAt: now - 7200000, updatedAt: now, from: HOST, to: PC, items: ["build.zip"], files: [entry("build.zip", 3e8)], totalBytes: 3e8, doneBytes: 1.2e8,
      state: "failed", error: "link-device-offline", notify: false, via: "web" },
  ],
};
const browse = {
  roots: [{ name: "받은 파일", path: "/Users/me/Downloads/Agent Office", exists: true }, { name: "에이다", path: "/Users/me/work/site", exists: true }],
  path: "/Users/me/Downloads/Agent Office", parent: null, truncated: false,
  entries: [{ name: "에이다", path: "/Users/me/Downloads/Agent Office/에이다", dir: true, size: 0, mtime: 0 }, { name: "notes.md", path: "/Users/me/Downloads/Agent Office/notes.md", dir: false, size: 1200, mtime: 0 }],
};
const posts = [];
const puts = [];
const send = (res, data) => { res.setHeader("content-type", "application/json"); res.end(JSON.stringify({ ok: true, data })); };
const server = createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  if (assets[url.pathname]) { res.setHeader("content-type", assets[url.pathname][1]); return res.end(assets[url.pathname][0]); }
  if (url.pathname === "/api/state") return send(res, { app: "0.0.0", host: "집 맥미니", rev: "r1", characters: [], officeAgent: { session: null }, folders: [], home: "/Users/me", outputAt: {}, now: Date.now() });
  if (url.pathname === "/api/usage") return send(res, {});
  if (url.pathname === "/api/tasks") return send(res, { rev: "t1", open: [], done: [] });
  if (url.pathname === "/api/files") return send(res, overview);
  if (url.pathname === "/api/files/browse") return send(res, browse);
  let body = Buffer.alloc(0);
  for await (const chunk of req) body = Buffer.concat([body, chunk]);
  if (req.method === "PUT" && url.pathname.startsWith("/api/files/uploads/")) {
    puts.push({ path: url.pathname, offset: Number(url.searchParams.get("offset")), len: body.length });
    return send(res, { received: Number(url.searchParams.get("offset")) + body.length });
  }
  if (req.method === "POST") {
    const parsed = body.length ? JSON.parse(body.toString()) : null;
    posts.push({ path: url.pathname, body: parsed });
    if (url.pathname === "/api/files/uploads") return send(res, { id: "u-1" });
    if (url.pathname === "/api/files/send") return send(res, { id: "f-new", items: ["photo.jpg"] });
    return send(res, null);
  }
  res.statusCode = 404; res.end();
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/#t=abc`;

const { chromium } = await import("playwright-core");
const browser = await chromium.launch({ channel: "chrome", headless: true });
const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, locale: "ko-KR" });
const p = await ctx.newPage();
const errors = [];
p.on("pageerror", (e) => errors.push(e.message));
await mkdir(outDir, { recursive: true });
await p.goto(base);
await p.getByRole("button", { name: "파일", exact: true }).click();
await p.locator("#fhist .files-row").first().waitFor();
assert.equal(await p.locator("#fhist .files-row").count(), 3);
assert.match(await p.locator("#fhist").textContent(), /그 머신과 연결이 끊겼습니다/);
assert.equal(await p.locator('#fhist a[href*="/api/files/t/f-2/file/0?t=abc"]').count(), 1, "받은 파일 내려받기 링크");
await p.screenshot({ path: `${outDir}/files-history.png`, fullPage: true });

// 이 기기에서 올려 다른 머신 캐릭터에게.
await p.locator("select.files-target").selectOption("m-pc|r1");
const big = resolve(outDir, "photo.jpg");
await writeFile(big, Buffer.alloc(5 * 1024 * 1024 + 10, 1));
await p.locator('.files-picked input[type="file"]').setInputFiles(big);
await p.locator(".files-send textarea").fill("표지에 쓰세요");
await p.screenshot({ path: `${outDir}/files-upload.png`, fullPage: true });
await p.getByRole("button", { name: "보내기", exact: true }).click();
await p.locator("#toast.show").filter({ hasText: "보냈습니다" }).waitFor();
assert.deepEqual(puts.map((x) => [x.offset, x.len]), [[0, 4 * 1024 * 1024], [4 * 1024 * 1024, 1024 * 1024 + 10]]);
const sent = posts.find((x) => x.path === "/api/files/send").body;
assert.deepEqual(sent, { machineId: "m-pc", agentId: "r1", note: "표지에 쓰세요", notify: true, uploads: ["u-1"] });

// 머신에서 고르기.
await p.getByRole("button", { name: "머신에서 고르기" }).click();
await p.locator(".files-roots").waitFor();
await p.getByRole("button", { name: "받은 파일" }).click();
await p.locator(".files-browse li").first().waitFor();
await p.locator(".files-browse li").filter({ hasText: "notes.md" }).locator("input").check();
await p.screenshot({ path: `${outDir}/files-browse.png`, fullPage: true });
await p.getByRole("button", { name: "보내기", exact: true }).click();
await p.locator("#toast.show").filter({ hasText: "보냈습니다" }).waitFor();
const sent2 = posts.filter((x) => x.path === "/api/files/send").at(-1).body;
assert.equal(sent2.sourceMachineId, "m-host");
assert.deepEqual(sent2.paths, ["/Users/me/Downloads/Agent Office/notes.md"]);
assert.deepEqual(errors, []);
console.log("ok", outDir);
await browser.close();
server.close();
