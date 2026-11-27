// scripts/dash-shell-shot.mjs
//
// 웹 상태판의 셸 캐릭터 화면(src-tauri/web/shell.js)을 가짜 API로 띄워 폰 크기로 찍는다.
//   node scripts/dash-shell-shot.mjs [출력 폴더(기본 /tmp/dash-shell-shot)]
// 찍고 확인하는 것: 말 → 명령 제안 카드(위험 표시·수정·실행), `!` 직접 실행, 실행 중 칸(출력 꼬리·Ctrl-C),
// 명령 기록(최신이 위, 종료 코드, 결과 설명, 긴 출력 접기), LLM이 없을 때 안내.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const outDir = resolve(process.argv[2] || "/tmp/dash-shell-shot");
const files = {
  "/": ["src-tauri/web/dash.html", "text/html; charset=utf-8"],
  "/dash.css": ["src-tauri/web/dash.css", "text/css; charset=utf-8"],
  "/dash.js": ["src-tauri/web/dash.js", "text/javascript; charset=utf-8"],
  "/tasks.js": ["src-tauri/web/tasks.js", "text/javascript; charset=utf-8"],
  "/shell.js": ["src-tauri/web/shell.js", "text/javascript; charset=utf-8"],
};
const assets = Object.fromEntries(await Promise.all(Object.entries(files).map(async ([k, [f, type]]) => [k, [await readFile(resolve(f), "utf8"), type]])));
const now = Date.now();
const characters = [
  { agentId: "s", name: "셸리", role: "운영", cwd: "/Users/me/work/site", clockedOut: false, shellAssist: true,
    session: { sessionId: "s1", state: "running" }, turn: null, asks: [], cli: null, atPrompt: true, queue: { items: [] } },
];
let shellRev = 3;
let shell = {
  sessionId: "s1", shell: "zsh", cwd: "/Users/me/work/site", phase: "prompt", rev: shellRev, llm: true, explain: true, live: [],
  blocks: [
    { id: 1, command: "git status", cwd: "/Users/me/work/site", startedAt: now - 120000, endedAt: now - 119500, exit: 0,
      output: "On branch main\nnothing to commit, working tree clean", lines: 2, dropped: 0, fullscreen: false, origin: "web",
      explanation: { state: "done", text: "main 브랜치이고 바뀐 파일이 없습니다.", source: "4090 PC" } },
    { id: 2, command: "npm test", cwd: "/Users/me/work/site", startedAt: now - 60000, endedAt: now - 45000, exit: 1,
      output: Array.from({ length: 30 }, (_, i) => `line ${i + 1}`).join("\n") + "\nFAIL src/app.test.ts\n  expected 2, got 3", lines: 32, dropped: 0, fullscreen: false, origin: "terminal",
      explanation: { state: "done", text: "테스트 하나가 실패했습니다. app.test.ts에서 2를 기대했지만 3이 나왔습니다.", source: "4090 PC" } },
  ],
};
const posts = [];
const send = (res, data) => { res.setHeader("content-type", "application/json"); res.end(JSON.stringify({ ok: true, data })); };
const fail = (res, error) => { res.setHeader("content-type", "application/json"); res.end(JSON.stringify({ ok: false, error })); };
const server = createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  if (assets[url.pathname]) { res.setHeader("content-type", assets[url.pathname][1]); return res.end(assets[url.pathname][0]); }
  if (url.pathname === "/api/state") return send(res, { app: "0.0.0", host: "집 맥미니", rev: "r1", characters, officeAgent: { session: null }, folders: [], home: "/Users/me", outputAt: {}, now: Date.now() });
  if (url.pathname === "/api/usage") return send(res, {});
  if (url.pathname === "/api/tasks") return send(res, { rev: "t1", open: [], done: [] });
  if (url.pathname === "/api/shell/s") {
    if (url.searchParams.get("rev") === String(shell.rev)) return send(res, { same: true, rev: shell.rev });
    return send(res, shell);
  }
  if (req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    const parsed = body ? JSON.parse(body) : null;
    posts.push({ path: url.pathname, body: parsed });
    if (url.pathname === "/api/shell/s/suggest") {
      if (parsed.text.includes("지워")) return send(res, { command: "rm -rf dist", explanation: "dist 폴더를 통째로 지웁니다.", interactive: false, risky: true, multiline: false, source: "4090 PC" });
      return send(res, { command: "du -sh * | sort -rh | head -10", explanation: "이 폴더에서 큰 항목 10개를 보여 줍니다.", interactive: false, risky: false, multiline: false, source: "4090 PC" });
    }
    if (url.pathname === "/api/shell/s/run") {
      if (shell.phase === "running") return fail(res, "shell-running");
      shell = { ...shell, phase: "running", rev: ++shellRev, live: ["12M\tnode_modules", "1.1M\tdist"],
        blocks: [...shell.blocks, { id: 3, command: parsed.command, cwd: shell.cwd, startedAt: Date.now(), endedAt: null, exit: null, output: "", lines: 0, dropped: 0, fullscreen: false, origin: "web", explanation: null }] };
      return send(res, null);
    }
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
await p.locator(".card.clickable").filter({ hasText: "셸리" }).waitFor();
assert.equal(await p.locator(".card.clickable").filter({ hasText: "셸리" }).locator(".chip.cli").textContent(), "셸 캐릭터");
await p.locator(".card.clickable").filter({ hasText: "셸리" }).click();
await p.locator("#shblocks .sh-block").first().waitFor();
// 셸 캐릭터 머리에는 CLI 띄우기·이어하기가 없다.
assert.equal(await p.locator("#head .launch").count(), 0);
// 기록은 최신이 위, 실패 코드와 설명이 보이고, 긴 출력은 접혀 있다.
assert.deepEqual(await p.locator("#shblocks .sh-line").allTextContents(), ["$ npm test", "$ git status"]);
assert.equal(await p.locator('#shblocks .sh-block[data-id="2"] .chip.bad').textContent(), "실패 1");
assert.equal(await p.locator('#shblocks .sh-block[data-id="2"] details.sh-out').getAttribute("open"), null);
assert.notEqual(await p.locator('#shblocks .sh-block[data-id="1"] details.sh-out').getAttribute("open"), null);
await p.screenshot({ path: `${outDir}/shell-idle.png`, fullPage: true });

// 위험한 제안은 경고와 함께, 수정 후 실행하면 고친 명령이 간다.
await p.locator("#shtext").fill("dist 지워");
await p.getByRole("button", { name: "명령 만들기" }).click();
await p.locator("#shprop .sh-prop.risky").waitFor();
await p.screenshot({ path: `${outDir}/shell-risky.png` });
await p.getByRole("button", { name: "취소" }).click();
assert.equal(await p.locator("#shprop .sh-prop").count(), 0);

await p.locator("#shtext").fill("큰 파일 보여 줘");
await p.getByRole("button", { name: "명령 만들기" }).click();
await p.locator("#shprop .sh-cmd").waitFor();
assert.equal(await p.locator("#shprop .sh-cmd").inputValue(), "du -sh * | sort -rh | head -10");
await p.getByRole("button", { name: "수정" }).click();
await p.locator("#shprop .sh-cmd").fill("du -sh * | sort -rh | head -5");
await p.screenshot({ path: `${outDir}/shell-proposal.png` });
await p.locator("#shprop").getByRole("button", { name: "실행" }).click();
await p.locator("#shstatus.busy").waitFor();
assert.deepEqual(posts.filter((x) => x.path === "/api/shell/s/run").map((x) => x.body.command), ["du -sh * | sort -rh | head -5"]);
assert.match(await p.locator("#shstatus .sh-tail").textContent(), /node_modules/);
await p.screenshot({ path: `${outDir}/shell-running.png` });

// 실행 중에 `!` 직접 실행은 거절 문구가 뜬다.
await p.locator("#shtext").fill("!ls");
await p.getByRole("button", { name: "바로 실행" }).click();
await p.locator("#toast.show").filter({ hasText: "명령이 실행 중입니다" }).waitFor();

// LLM이 없으면 안내가 보인다.
shell = { ...shell, llm: false, rev: ++shellRev };
await p.locator("#shask .warn-note").waitFor();
assert.deepEqual(errors, []);
console.log("ok", outDir);
await browser.close();
server.close();
