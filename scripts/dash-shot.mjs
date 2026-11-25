// scripts/dash-shot.mjs
//
// 웹 상태판(src-tauri/web/dash.html)을 가짜 API로 띄워 폰 크기로 찍는다. 앱·Tauri 없이 화면만 본다.
//   node scripts/dash-shot.mjs [출력 폴더(기본 /tmp/dash-shot)]
// 찍는 것: 캐릭터 목록(머리에 컴퓨터 이름, 맨 위 일반 에이전트), 캐릭터 화면(보낸 업무·대화 로그 최신순·업무 보내기),
// CLI 실행 중 재시작 경고, 끝난 업무 완료, 일반 에이전트 새 지시·이어서 보내기,
// 대화 로그 속 이미지(썸네일·확대), 권한 요청·선택지 카드(AskUserQuestion 답·Bash 허용).
import { createServer } from "node:http";
import { readFile, mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const outDir = resolve(process.argv[2] || "/tmp/dash-shot");
const page = await readFile(resolve("src-tauri/web/dash.html"), "utf8");
const now = Date.now();
const characters = [
  {
    agentId: "a", name: "에이다", role: "개발", cwd: "/Volumes/ssd4tb/dev/very/long/path/to/agent-office",
    clockedOut: false, defaultCli: "claude", session: { sessionId: "s1", state: "running" },
    turn: { phase: "working", at: now }, waiting: [], cli: "claude", atPrompt: false,
    asks: [{ id: "ask-1", toolName: "AskUserQuestion", canAlways: false, sidechain: false, at: now, toolInput: { questions: [
      { question: "어떤 방식으로 이미지를 보여 줄까요?", header: "표시", multiSelect: false, options: [
        { label: "썸네일", description: "로그 안에 작게 보이고 누르면 확대" }, { label: "링크만", description: "누르면 새 창" }] },
      { question: "어디에 보일까요?", header: "위치", multiSelect: true, options: [{ label: "웹" }, { label: "앱" }] },
    ] } }],
    label: { goal: "웹 상태판 대화 로그 정리", cwd: "/Volumes/ssd4tb/dev/very/long/path/to/agent-office", branch: "main" },
  },
  {
    agentId: "b", name: "보리", role: "리뷰", cwd: "/Users/me/work/backend-service",
    clockedOut: false, session: { sessionId: "s2", state: "running" }, turn: { phase: "idle", at: now },
    waiting: [], cli: null, atPrompt: true, label: { cwd: "/Users/me/work/backend-service" },
    asks: [{ id: "ask-2", toolName: "Bash", canAlways: true, sidechain: false, at: now,
      toolInput: { command: "npm run build && rm -rf dist/tmp", description: "빌드하고 임시 폴더 정리" } }],
  },
];
const items = [
  { seq: 1, role: "user", kind: "text", text: "대화 로그를 최신순으로 보여 줘" },
  { seq: 2, role: "assistant", kind: "text", text: "dash.html을 살펴보겠습니다." },
  { seq: 3, role: "assistant", kind: "tool_use", toolName: "Read", text: "src-tauri/web/dash.html" },
  { seq: 4, role: "user", kind: "tool_result", text: "1\t<!doctype html>\n2\t<html>" },
  { seq: 5, role: "user", kind: "text", text: "사람이 입력한 게 안 보이는 거 같다." },
  { seq: 6, role: "assistant", kind: "tool_use", toolName: "Grep", text: "queued_command" },
  { seq: 7, role: "assistant", kind: "tool_use", toolName: "Bash", text: "git log --oneline -3" },
  { seq: 8, role: "user", kind: "tool_result", text: "claude.rs:120: queued_command" },
  { seq: 9, role: "user", kind: "tool_result", text: "fatal: not a git repository", isError: true },
  { seq: 10, role: "assistant", kind: "text", text: "원인을 찾았습니다." },
  { seq: 11, role: "assistant", kind: "tool_use", toolName: "Read", text: "/tmp/shot.png" },
  { seq: 12, role: "user", kind: "tool_result", text: "(이미지)" },
  { seq: 13, role: "user", kind: "image", text: "", image: { id: "img1", mediaType: "image/png", bytes: 1000 } },
  { seq: 14, role: "user", kind: "text", text: "이 화면 좀 봐 줘" },
  { seq: 15, role: "user", kind: "image", text: "", image: { id: "img2", mediaType: "image/png", bytes: 1000 } },
  { seq: 16, role: "user", kind: "image", text: "", image: { id: "gone", mediaType: "image/png", bytes: 1000 } },
];
const png = await readFile(resolve("scripts/icon-source-small.png"));
const tasks = {
  open: [
    { id: "w1", title: "로그 정렬 고치기", body: "", status: "review", assigneeAgentId: "a", plan: false, updatedAt: now - 60000 },
    { id: "w2", title: "", body: "토큰 갱신 버튼 문구 바꾸기\n자세히", status: "in_progress", assigneeAgentId: "a", plan: false, updatedAt: now },
    { id: "w3", title: "남의 업무", body: "", status: "review", assigneeAgentId: "b", plan: false, updatedAt: now },
  ],
  done: [],
};
const posts = [];
let rev = 1;
let officeAgent = { session: null, turn: null, cli: null, atPrompt: null, cwd: null, opening: false, error: null };
const folders = ["/Users/me", "/Volumes/ssd4tb/dev/very/long/path/to/agent-office", "/Users/me/work/backend-service"];
const server = createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  const send = (data) => { res.setHeader("content-type", "application/json"); res.end(JSON.stringify({ ok: true, data })); };
  if (url.pathname === "/") { res.setHeader("content-type", "text/html; charset=utf-8"); return res.end(page); }
  if (url.pathname === "/api/state") return send({ app: "0.0.0", host: "회사 맥북 프로", rev: `r${rev}`, characters, officeAgent, folders, home: "/Users/me" });
  if (url.pathname.startsWith("/api/log/")) return send({ epoch: 1, last: 16, attached: true, items: url.searchParams.get("after") ? [] : items });
  if (url.pathname.startsWith("/api/image/")) {
    if (url.pathname.endsWith("/gone")) { res.statusCode = 404; return res.end(); }
    res.setHeader("content-type", "image/png"); return res.end(png);
  }
  if (url.pathname === "/api/tasks" && req.method === "GET") return send(tasks);
  if (req.method === "POST") {
    let body = ""; for await (const chunk of req) body += chunk;
    posts.push({ path: url.pathname, body: JSON.parse(body || "{}") });
    const done = url.pathname.match(/^\/api\/tasks\/([^/]+)\/done$/);
    if (done) tasks.open = tasks.open.filter((t) => t.id !== decodeURIComponent(done[1]));
    const ask = url.pathname.match(/^\/api\/asks\/([^/]+)$/);
    if (ask) { for (const c of characters) c.asks = (c.asks || []).filter((a) => a.id !== decodeURIComponent(ask[1])); rev++; }
    if (url.pathname === "/api/office-agent") {
      officeAgent = { ...officeAgent, session: { sessionId: "o1", state: "running" }, turn: { phase: "idle", at: now }, cli: "claude", atPrompt: false, cwd: posts.at(-1).body.cwd };
      rev++;
    }
    return send(null);
  }
  res.statusCode = 404; res.end();
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/#t=abc`;

const { chromium } = await import("playwright-core");
const browser = await chromium.launch({ channel: "chrome", headless: true });
const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, locale: "ko-KR" });
const p = await ctx.newPage();
p.on("pageerror", (e) => console.error("pageerror:", e.message));
await mkdir(outDir, { recursive: true });
await p.goto(base);
await p.waitForSelector(".card");
await p.screenshot({ path: `${outDir}/list.png` });
await p.locator(".card.clickable").nth(1).click();
await p.waitForSelector("#log .msg");
await p.waitForSelector("#sent .card");
await p.waitForSelector("#asks .card.ask");
await p.waitForFunction(() => [...document.querySelectorAll("#log img.shot")].every((i) => i.complete));
await p.screenshot({ path: `${outDir}/agent-claude.png`, fullPage: true });
// 선택지: 하나는 고르지 않고 보내면 막히고, 둘 다 답하면 보낸다.
await p.locator("#asks button.opt", { hasText: "썸네일" }).click();
await p.getByRole("button", { name: "답 보내기" }).click();
await p.waitForSelector("text=모든 질문에 답을");
await p.locator("#asks button.opt", { hasText: "웹" }).click();
await p.locator("#asks button.opt", { hasText: "앱" }).click();
await p.screenshot({ path: `${outDir}/agent-ask.png` });
await p.getByRole("button", { name: "답 보내기" }).click();
await p.waitForFunction(() => !document.querySelector("#asks .card"));
// 이미지 확대
await p.locator("#log img.shot").first().click();
await p.waitForSelector(".lb img");
await p.screenshot({ path: `${outDir}/agent-lightbox.png` });
await p.locator(".lb").click();
await p.locator("#sent button", { hasText: "완료" }).first().click();
await p.waitForFunction(() => document.querySelectorAll("#sent .row").length === 1);
await p.screenshot({ path: `${outDir}/agent-done.png` });
await p.fill("#taskform input", "로그 정렬 고치기");
await p.getByRole("button", { name: "재시작하고 보내기" }).click();
await p.getByRole("button", { name: "한 번 더 눌러 확인" }).click();
await p.waitForTimeout(300);
await p.locator(".back").click();
await p.locator(".card.clickable").nth(2).click();
await p.waitForSelector("#taskform");
await p.waitForSelector("#asks .card.ask");
await p.screenshot({ path: `${outDir}/agent-shell.png`, fullPage: true });
await p.fill("#asks input", "임시 폴더는 지우지 마");
await p.getByRole("button", { name: "거부" }).click();
await p.waitForFunction(() => !document.querySelector("#asks .card"));
await p.locator(".back").click();
await p.locator(".card.office").click();
await p.waitForSelector("#taskform textarea");
await p.screenshot({ path: `${outDir}/office-new.png`, fullPage: true });
await p.fill("#taskform textarea", "업무 보드에서 리뷰 대기 업무를 정리해 줘");
await p.locator("#taskform select").first().selectOption({ index: 1 });
await p.getByRole("button", { name: "보내기", exact: true }).click();
await p.waitForSelector("text=이어서 보내기");
await p.screenshot({ path: `${outDir}/office-follow.png`, fullPage: true });
console.log(JSON.stringify(posts));
await browser.close();
server.close();
