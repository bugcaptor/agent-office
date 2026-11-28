// scripts/dash-notify-shot.mjs
//
// 웹 상태판의 데스크톱 알림과 제목 대기 수를 가짜 API로 확인한다. 앱·Tauri 없이 화면만 본다.
//   node scripts/dash-notify-shot.mjs [출력 폴더(기본 /tmp/dash-notify-shot)]
// 확인하는 것: 🔔 단추로 켜기(권한 요청), 답을 기다리게 되면·턴이 끝나면 알림(첫 상태는 알리지 않음),
// 보고 있을 땐 알리지 않음, 제목 "(N) …"(답 대기 + 안 보는 동안 끝난 턴), 화면을 다시 보면 끝난 턴을 지움,
// 알림을 누르면 그 캐릭터 화면, 화면이 숨어 있어도 상태를 받아 알림.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const outDir = resolve(process.argv[2] || "/tmp/dash-notify-shot");
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
const char = (agentId, name, phase, since) => ({
  agentId, name, role: "개발", cwd: `/Users/me/${agentId}`, clockedOut: false, defaultCli: "claude",
  session: { sessionId: `s-${agentId}`, state: "running" }, turn: { phase, at: since, since }, cli: "claude", atPrompt: false,
  queue: { items: [] }, asks: [], label: { goal: `${name}의 목표` },
});
const characters = [char("a", "에이다", "working", now), char("b", "보리", "idle", now - 5000), char("c", "체리", "needs-human", now)];
let rev = 1;
const set = (id, phase, extra = {}) => {
  const c = characters.find((x) => x.agentId === id);
  const t = Date.now();
  c.turn = { phase, at: t, since: phase === c.turn.phase ? c.turn.since : t, ...extra };
  rev++;
};
const server = createServer((req, res) => {
  const url = new URL(req.url, "http://x");
  const send = (data) => { res.setHeader("content-type", "application/json"); res.end(JSON.stringify({ ok: true, data })); };
  const a = assets[url.pathname];
  if (a) { res.setHeader("content-type", a[1]); return res.end(a[0]); }
  if (url.pathname === "/api/state") return send({ app: "0.0.0", host: "회사 맥북", rev: `r${rev}`, characters, officeAgent: null, machines: [], folders: [], home: "/Users/me", outputAt: {}, now: Date.now() });
  if (url.pathname === "/api/usage") return send({});
  if (url.pathname === "/api/tasks") return send({ rev: 1, items: [] });
  if (url.pathname.startsWith("/api/log/")) return send(null);
  res.statusCode = 404; res.end();
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/#t=abc`;

const { chromium } = await import("playwright-core");
const browser = await chromium.launch({ channel: "chrome", headless: true, args: ["--no-proxy-server"] });
const ctx = await browser.newContext({ viewport: { width: 1100, height: 700 }, locale: "ko-KR" });
// 헤드리스에선 실제 알림·포커스를 볼 수 없어 Notification과 hasFocus·hidden을 바꿔 끼운다.
await ctx.addInitScript(() => {
  window.__sent = [];
  window.__look = true;
  window.__hidden = false;
  class FakeNotification {
    static permission = "default";
    static async requestPermission() { FakeNotification.permission = "granted"; return "granted"; }
    constructor(title, opts) { this.title = title; this.body = opts && opts.body; window.__sent.push(this); }
    close() {}
  }
  window.Notification = FakeNotification;
  document.hasFocus = () => window.__look;
  Object.defineProperty(document, "hidden", { get: () => window.__hidden });
});
const p = await ctx.newPage();
p.on("pageerror", (e) => { console.error("pageerror:", e.message); process.exitCode = 1; });
await mkdir(outDir, { recursive: true });
await p.goto(base);
await p.waitForSelector(".card");
const sent = () => p.evaluate(() => window.__sent.map((n) => `${n.title} | ${n.body}`));
const waitTitle = (t) => p.waitForFunction((t) => document.title === t, t, { timeout: 8000 });

// 첫 상태: 체리가 답을 기다림 → 제목 (1), 알림 없음(아직 꺼짐).
await waitTitle("(1) 회사 맥북 · Agent Office");
assert.equal(await p.locator("#notify").textContent(), "🔕 알림");
await p.locator("#notify").click();
assert.equal(await p.locator("#notify").textContent(), "🔔 알림");
assert.equal(await p.evaluate(() => localStorage.getItem("agent-office-web-dash-notify")), "1");
await p.locator(".bar").screenshot({ path: `${outDir}/bar.png` });
await p.setViewportSize({ width: 390, height: 844 });
await p.locator(".bar").screenshot({ path: `${outDir}/bar-phone.png` });
assert.ok(await p.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "폰 폭에서 가로로 넘치지 않음");
await p.setViewportSize({ width: 1100, height: 700 });

// 보고 있을 때 턴이 끝나면 알림도, 제목 수도 없음.
set("a", "idle");
await p.waitForFunction(() => document.querySelector('.card .chip.idle') && true);
await p.waitForTimeout(3000);
assert.deepEqual(await sent(), []);
assert.equal(await p.title(), "(1) 회사 맥북 · Agent Office");

// 안 보는 동안: 보리가 답을 기다림, 에이다는 새 턴을 하고 끝남(작업 중을 못 봐도 since로 잡음).
await p.evaluate(() => { window.__look = false; });
set("b", "needs-human", { message: "Bash 실행 허용?" });
set("a", "idle"); characters[0].turn.since = Date.now() + 1; rev++;
await waitTitle("(3) 회사 맥북 · Agent Office");
assert.deepEqual((await sent()).sort(), ["보리 · 답을 기다림 | Bash 실행 허용?", "에이다 · 턴 끝남 | 에이다의 목표"].sort());

// 화면을 숨겨도 상태를 받아 알림(숨은 동안엔 4틱에 한 번).
await p.evaluate(() => { window.__hidden = true; document.dispatchEvent(new Event("visibilitychange")); });
set("c", "working"); set("c", "idle");
await p.waitForFunction(() => window.__sent.some((n) => n.title === "체리 · 턴 끝남"), null, { timeout: 15000 });
assert.equal(await p.title(), "(3) 회사 맥북 · Agent Office", "보리 대기 + 에이다·체리 끝남");

// 다시 보면 끝난 턴은 지우고 답 대기만 남는다.
await p.evaluate(() => { window.__hidden = false; window.__look = true; window.dispatchEvent(new Event("focus")); });
await waitTitle("(1) 회사 맥북 · Agent Office");
await p.screenshot({ path: `${outDir}/list.png` });

// 알림을 누르면 그 캐릭터 화면으로.
await p.evaluate(() => window.__sent.find((n) => n.title.startsWith("보리")).onclick());
await p.waitForSelector("#head");
assert.match(await p.locator("#head").textContent(), /보리/);

// 끄면 더 알리지 않는다.
await p.locator("#notify").click();
await p.evaluate(() => { window.__look = false; });
const before = (await sent()).length;
set("b", "working"); set("b", "idle");
await p.waitForTimeout(3500);
assert.equal((await sent()).length, before);

await browser.close();
server.close();
console.log("ok", outDir);
