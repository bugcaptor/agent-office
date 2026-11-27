// scripts/dash-shot.mjs
//
// 웹 상태판(src-tauri/web/dash.html)을 가짜 API로 띄워 폰 크기로 찍는다. 앱·Tauri 없이 화면만 본다.
//   node scripts/dash-shot.mjs [출력 폴더(기본 /tmp/dash-shot)]
// 찍는 것: 캐릭터 목록(머리에 컴퓨터 이름, 맨 위 일반 에이전트), 캐릭터 화면(보낸 업무·대화 로그 최신순·업무 보내기),
// CLI 실행 중 재시작 경고, 셸 프롬프트에서 고른 CLI 띄우기, 끝난 업무 완료, 일반 에이전트 새 지시·이어서 보내기,
// 대화 로그 속 이미지(썸네일·확대), 권한 요청·선택지 카드(AskUserQuestion 답·Bash 허용),
// 텍스트·이미지 클립보드 복사와 이미지·파일 다운로드, 하단 사용량 바, 생각 중·멈춤 상태 줄,
// 직접 입력과 보낼 대기열(쌓기·빼기), 로그 위 이 세션 추정 사용량, 화면 테마 토글(브라우저에 저장),
// 목록·업무 탭 진행 중 카드의 활성 캐릭터 로그 미리보기(접기·세션이 바뀌면 다시 펼침), 스플릿뷰(칸 격자·완료로 내리기·사이드 패널),
// 목록·상세·스플릿 칸의 진행 중 업무 표시와 플랜 진행도, 머신 연동 호스트의 머신별 구역(끊긴 머신 흐림)과
// 다른 머신 캐릭터 화면(보기만 — 입력·조작 단추 없음, 권한 요청은 그 머신에서 답하라는 카드).
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const outDir = resolve(process.argv[2] || "/tmp/dash-shot");
const page = await readFile(resolve("src-tauri/web/dash.html"), "utf8");
const dashCss = await readFile(resolve("src-tauri/web/dash.css"), "utf8");
const dashJs = await readFile(resolve("src-tauri/web/dash.js"), "utf8");
const tasksJs = await readFile(resolve("src-tauri/web/tasks.js"), "utf8");
const now = Date.now();
let machines = [];
const characters = [
  {
    agentId: "a", name: "에이다", role: "개발", cwd: "/Volumes/ssd4tb/dev/very/long/path/to/agent-office", portrait: 1,
    clockedOut: false, defaultCli: "claude", session: { sessionId: "s1", state: "running" },
    turn: { phase: "working", at: now, since: now - 75000 }, cli: "claude", atPrompt: false, queue: { items: [] },
    asks: [{ id: "ask-1", toolName: "AskUserQuestion", canAlways: false, sidechain: false, at: now, toolInput: { questions: [
      { question: "어떤 방식으로 이미지를 보여 줄까요?", header: "표시", multiSelect: false, options: [
        { label: "썸네일", description: "로그 안에 작게 보이고 누르면 확대" }, { label: "링크만", description: "누르면 새 창" }] },
      { question: "어디에 보일까요?", header: "위치", multiSelect: true, options: [{ label: "웹" }, { label: "앱" }] },
    ] } }],
    work: { id: "w2", title: "토큰 갱신 버튼 문구 바꾸기", plan: null },
    label: { goal: "웹 상태판 대화 로그 정리", cwd: "/Volumes/ssd4tb/dev/very/long/path/to/agent-office", branch: "main",
      usage: { sessionId: "s1", tokens: "11.7K CH(1.7M)", cost: "~$3.45", turns: 12 } },
  },
  {
    agentId: "b", name: "보리", role: "리뷰", cwd: "/Users/me/work/backend-service",
    clockedOut: false, session: { sessionId: "s2", state: "running" }, turn: { phase: "idle", at: now - 1000 },
    cli: null, atPrompt: true, label: { cwd: "/Users/me/work/backend-service" },
    asks: [{ id: "ask-2", toolName: "Bash", canAlways: true, sidechain: false, at: now,
      toolInput: { command: "npm run build && rm -rf dist/tmp", description: "빌드하고 임시 폴더 정리" } }],
  },
  // 목록 정렬: 답을 기다림 → 작업 중 → 나머지는 최근 턴 관측 순, 관측이 없으면 맨 뒤.
  { agentId: "c", name: "체리", role: "문서", cwd: "/Users/me/docs", clockedOut: true, session: null, turn: null, asks: [] },
  { agentId: "d", name: "도토리", role: "테스트", cwd: "/Users/me/work/backend-service", clockedOut: false,
    session: { sessionId: "s4", state: "running" }, turn: { phase: "idle", at: now - 60000 }, asks: [], cli: "claude", atPrompt: false },
  { agentId: "e", name: "이끼", role: "빌드", cwd: "/Users/me/work/backend-service", clockedOut: false,
    session: { sessionId: "s5", state: "running" }, turn: { phase: "working", at: now - 300000, since: now - 300000 }, asks: [], cli: "codex", atPrompt: false,
    work: { id: "p2", title: "작업보드 UI 구현", plan: { id: "proot", title: "웹 작업보드 출시 플랜", done: 2, total: 5, step: 3, autoRun: "running" } } },
];
const code = '  printf "완료\\n"\n';
const fileName = "실행 결과.txt";
const fileBytes = Buffer.from("줄 1\n줄 2\n");
const message = `아래 명령을 실행하세요.\n\n\`\`\`sh\n${code}\`\`\`\n\n[결과](실행%20결과.txt)`;
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
  // 에이전트 말은 서버가 Markdown을 HTML로 그려 `html`에 싣는다(`webdash/markdown.rs`). 여기서는 그 결과를 흉내 낸다.
  { seq: 10, role: "assistant", kind: "text", text: "## 원인\n\n**사람 입력**이 `queued_command`로 들어와 빠졌습니다.\n\n- 전사 읽기\n- 화면 그리기\n\n| 파일 | 고칠 곳 |\n|---|---|\n| claude.rs | 120행 |",
    html: "<h2>원인</h2>\n<p><strong>사람 입력</strong>이 <code>queued_command</code>로 들어와 빠졌습니다.</p>\n<ul>\n<li>전사 읽기</li>\n<li>화면 그리기</li>\n</ul>\n<table><thead><tr><th>파일</th><th>고칠 곳</th></tr></thead><tbody>\n<tr><td>claude.rs</td><td>120행</td></tr>\n</tbody></table>\n" },
  { seq: 11, role: "assistant", kind: "tool_use", toolName: "Read", text: "/tmp/shot.png" },
  { seq: 12, role: "user", kind: "tool_result", text: "(이미지)" },
  { seq: 13, role: "user", kind: "image", text: "", image: { id: "img1", mediaType: "image/png", bytes: 1000, source: "/tmp/shot.png" } },
  { seq: 14, role: "user", kind: "text", text: "이 화면 좀 봐 줘" },
  { seq: 15, role: "user", kind: "image", text: "", image: { id: "img2", mediaType: "image/png", bytes: 1000 } },
  { seq: 16, role: "user", kind: "image", text: "", image: { id: "gone", mediaType: "image/png", bytes: 1000 } },
  { seq: 17, role: "assistant", kind: "text", text: message, files: [{ name: fileName }],
    html: `<p>아래 명령을 실행하세요.</p>\n<pre><code>${code.replace(/&/g, "&amp;").replace(/</g, "&lt;")}</code></pre>\n<p>결과</p>\n` },
];
const png = await readFile(resolve("scripts/icon-source-small.png"));
let taskSeq = 10;
const tasks = {
  open: [
    { id: "w1", title: "로그 정렬 고치기", body: "대화 로그를 최신순으로 정렬한다.", status: "review", order: 0, assigneeAgentId: "a", plan: false, dependsOn: [], updatedAt: now - 60000 },
    { id: "w2", title: "", body: "토큰 갱신 버튼 문구 바꾸기\n자세히", status: "in_progress", order: 1, assigneeAgentId: "a", plan: false, dependsOn: [], updatedAt: now },
    { id: "w3", title: "남의 업무", body: "", status: "review", order: 0, assigneeAgentId: "b", plan: false, dependsOn: [], updatedAt: now },
    { id: "p1", title: "API 계약 정리", body: "라우트 스키마 확인", status: "done", order: 0, assigneeAgentId: "b", plan: false, dependsOn: [], updatedAt: now - 120000 },
    { id: "p2", title: "작업보드 UI 구현", body: "모바일·PC 반응형 보드 구현", status: "todo", order: 1, assigneeAgentId: "b", plan: false, dependsOn: ["p1"], updatedAt: now - 30000 },
    { id: "proot", title: "웹 작업보드 출시 플랜", body: "목표: 웹에서도 업무 관리와 자동 진행 제어", status: "todo", order: 2, assigneeAgentId: "b", plan: true, dependsOn: ["p2"],
      autoRun: { state: "paused", judgeSessionId: "j1", judgeStarted: true, currentId: "p2", followUps: 1, pauseReason: "judge-stop", pauseDetail: "완료된 업무 기본 표시 건수를 20건으로 둘까요?", costUsd: 0.42, startedAt: now - 300000, usageGuard: true,
        log: [{ at: now - 300000, kind: "start" }, { at: now - 240000, kind: "send", taskId: "p1" }, { at: now - 120000, kind: "next", taskId: "p1", text: "라우트 정의 완료" }, { at: now - 60000, kind: "pause", text: "judge-stop" }] },
      updatedAt: now - 10000 },
    { id: "sc1", title: "아침 저장소 점검", body: "git 상태 확인", status: "todo", order: 3, assigneeAgentId: "d", plan: false, dependsOn: [],
      schedule: { rule: { kind: "daily", time: "09:00" }, enabled: true, nextAt: now + 7200000, lastRunAt: now - 86400000 }, updatedAt: now - 50000 },
  ],
  done: [
    { id: "d1", title: "이전 완료 업무 1", body: "완료됨", status: "done", order: 0, assigneeAgentId: "a", plan: false, dependsOn: [], updatedAt: now - 200000 },
    { id: "d2", title: "이전 완료 업무 2", body: "완료됨", status: "done", order: 1, assigneeAgentId: "a", plan: false, dependsOn: [], updatedAt: now - 400000 },
  ],
};
let tasksRev = 1;
const posts = [];
const logReqs = [];
let rev = 1;
let aOutputAgo = 500;
const usage = {
  claude: { planLabel: "Max", windows: [
    { kind: "session", usedPercent: 32, resetsAtMs: now + 3600e3 }, { kind: "weekly", usedPercent: 78, resetsAtMs: now + 3 * 86400e3 },
    { kind: "weekly_model", label: "Opus", usedPercent: 12 }] },
  codex: { windows: [{ kind: "session", usedPercent: 5 }, { kind: "weekly", usedPercent: 96 }] },
};
let officeAgent = { session: null, turn: null, cli: null, atPrompt: null, cwd: null, opening: false, error: null };
const folders = ["/Users/me", "/Volumes/ssd4tb/dev/very/long/path/to/agent-office", "/Users/me/work/backend-service"];
const server = createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  const send = (data) => { res.setHeader("content-type", "application/json"); res.end(JSON.stringify({ ok: true, data })); };
  if (url.pathname === "/") { res.setHeader("content-type", "text/html; charset=utf-8"); return res.end(page); }
  if (url.pathname === "/dash.css") { res.setHeader("content-type", "text/css; charset=utf-8"); return res.end(dashCss); }
  if (url.pathname === "/dash.js") { res.setHeader("content-type", "text/javascript; charset=utf-8"); return res.end(dashJs); }
  if (url.pathname === "/tasks.js") { res.setHeader("content-type", "text/javascript; charset=utf-8"); return res.end(tasksJs); }
  if (url.pathname === "/api/state") {
    return send({ app: "0.0.0", host: "회사 맥북 프로", rev: `r${rev}`, characters, officeAgent, machines, folders, home: "/Users/me", outputAt: { a: Date.now() - aOutputAgo }, now: Date.now() });
  }
  if (url.pathname === "/api/usage") return send(usage);
  if (url.pathname.startsWith("/api/models/")) {
    const cli = url.pathname.split("/").pop();
    const list = cli === "claude" ? ["claude-sonnet-4-6", "claude-opus-4-6"] : cli === "codex" ? ["gpt-5.4", "gpt-5.4-mini"] : [];
    return send(list);
  }
  if (url.pathname.startsWith("/api/log/")) {
    logReqs.push(url.pathname + url.search);
    const tail = Number(url.searchParams.get("tail")) || items.length;
    return send({ epoch: 1, last: items.at(-1).seq, attached: true, items: url.searchParams.get("after") ? [] : items.slice(-tail) });
  }
  if (url.pathname.startsWith("/api/portrait/")) { res.setHeader("content-type", "image/png"); return res.end(png); }
  if (url.pathname.startsWith("/api/image/")) {
    if (url.searchParams.get("t") !== "abc" && req.headers.authorization !== "Bearer abc") {
      res.statusCode = 401; return res.end();
    }
    if (url.pathname.endsWith("/gone")) { res.statusCode = 404; return res.end(); }
    res.setHeader("content-type", "image/png"); return res.end(png);
  }
  if (url.pathname === "/api/file/a/1/17/0" && req.headers.authorization === "Bearer abc") {
    res.setHeader("content-type", "application/octet-stream"); return res.end(fileBytes);
  }
  if (url.pathname === "/api/tasks" && req.method === "GET") {
    const allDone = url.searchParams.get("allDone") === "1" || url.searchParams.get("allDone") === "true";
    const done = allDone ? tasks.done : tasks.done.slice(0, 1);
    return send({ open: tasks.open, done, doneTotal: tasks.done.length, allDone, observerEnabled: true, sessionCostEnabled: true, rev: `t${tasksRev}-${allDone}` });
  }
  if (req.method === "POST") {
    let body = ""; for await (const chunk of req) body += chunk;
    const parsed = JSON.parse(body || "{}");
    posts.push({ path: url.pathname, body: parsed });
    if (url.pathname === "/api/tasks") {
      const id = `new-${++taskSeq}`;
      const item = {
        id, title: parsed.title || "", body: parsed.body || "", status: parsed.status || "todo", order: tasks.open.length,
        assigneeAgentId: parsed.assigneeAgentId || undefined, agent: parsed.agent || (parsed.cli ? { cli: parsed.cli } : undefined),
        dependsOn: parsed.dependsOn || [], plan: !!parsed.plan, maxFollowUps: parsed.maxFollowUps ?? undefined,
        permissionWaitMinutes: parsed.permissionWaitMinutes ?? undefined, updatedAt: Date.now(),
      };
      if (item.status === "done") tasks.done.unshift(item); else tasks.open.push(item);
      tasksRev++;
      return send({ itemId: id });
    }
    const editM = url.pathname.match(/^\/api\/tasks\/([^/]+)\/edit$/);
    if (editM) {
      const id = decodeURIComponent(editM[1]);
      const item = [...tasks.open, ...tasks.done].find((t) => t.id === id);
      if (item) {
        Object.assign(item, parsed, { updatedAt: Date.now() });
        if (parsed.assigneeAgentId === null) delete item.assigneeAgentId;
        if (parsed.agent === null) delete item.agent;
        if (parsed.dependsOn === null) item.dependsOn = [];
        tasksRev++;
      }
      return send(null);
    }
    const delM = url.pathname.match(/^\/api\/tasks\/([^/]+)\/delete$/);
    if (delM) {
      const id = decodeURIComponent(delM[1]);
      tasks.open = tasks.open.filter((t) => t.id !== id);
      tasks.done = tasks.done.filter((t) => t.id !== id);
      tasksRev++;
      return send(null);
    }
    const arM = url.pathname.match(/^\/api\/tasks\/([^/]+)\/autorun$/);
    if (arM) {
      const id = decodeURIComponent(arM[1]);
      const item = tasks.open.find((t) => t.id === id);
      if (item) {
        if (parsed.action === "resume" && item.autoRun) { item.autoRun.state = "running"; delete item.autoRun.pauseReason; delete item.autoRun.pauseDetail; }
        else if (parsed.action === "pause" && item.autoRun) { item.autoRun.state = "paused"; item.autoRun.pauseReason = "user"; }
        else if (parsed.action === "end") delete item.autoRun;
        else if (parsed.action === "start" || !parsed.action) item.autoRun = { state: "running", followUps: 0, costUsd: 0, log: [{ at: Date.now(), kind: "start" }], startedAt: Date.now(), usageGuard: parsed.usageGuard !== false };
        item.updatedAt = Date.now();
        tasksRev++;
      }
      return send(null);
    }
    const scM = url.pathname.match(/^\/api\/tasks\/([^/]+)\/schedule$/);
    if (scM) {
      const id = decodeURIComponent(scM[1]);
      const item = tasks.open.find((t) => t.id === id);
      if (item) {
        if (parsed.action === "remove") delete item.schedule;
        else if (parsed.action === "disable" && item.schedule) { item.schedule.enabled = false; delete item.schedule.nextAt; }
        else if (parsed.action === "enable" && item.schedule) { item.schedule.enabled = true; item.schedule.nextAt = Date.now() + 3600000; }
        else if (parsed.action === "set" && parsed.rule) { item.schedule = { rule: parsed.rule, enabled: true, nextAt: Date.now() + 3600000 }; }
        item.updatedAt = Date.now();
        tasksRev++;
      }
      return send(null);
    }
    const done = url.pathname.match(/^\/api\/tasks\/([^/]+)\/done$/);
    if (done) {
      const id = decodeURIComponent(done[1]);
      const found = tasks.open.find((t) => t.id === id);
      if (found) { tasks.open = tasks.open.filter((t) => t.id !== id); found.status = "done"; found.updatedAt = Date.now(); tasks.done.unshift(found); tasksRev++; }
    }
    const ask = url.pathname.match(/^\/api\/asks\/([^/]+)$/);
    if (ask) { for (const c of characters) c.asks = (c.asks || []).filter((a) => a.id !== decodeURIComponent(ask[1])); rev++; }
    const a = characters[0];
    if (url.pathname === "/api/agents/a/message") {
      a.queue.items.push({ id: `q${a.queue.items.length + 1}`, text: posts.at(-1).body.text, at: Date.now() });
      rev++;
      return send({ outcome: "queued" });
    }
    if (url.pathname === "/api/agents/b/message") return send({ outcome: "launched" });
    if (url.pathname === "/api/agents/a/queue-cancel") { a.queue.items = a.queue.items.filter((m) => m.id !== posts.at(-1).body.id); rev++; }
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
const browser = await chromium.launch({ channel: "chrome", headless: true,
  args: ["--host-resolver-rules=MAP dashboard.test 127.0.0.1", "--no-proxy-server"] });
const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, locale: "ko-KR" });
await ctx.grantPermissions(["clipboard-read", "clipboard-write"], { origin: new URL(base).origin });
const p = await ctx.newPage();
p.on("pageerror", (e) => console.error("pageerror:", e.message));
await mkdir(outDir, { recursive: true });
await p.goto(base);
await p.waitForSelector(".card");
await p.screenshot({ path: `${outDir}/list.png` });
assert.deepEqual(await p.locator(".card.clickable .name").allTextContents(), ["일반 에이전트", "에이다", "보리", "이끼", "도토리", "체리"]);
// 진행 중인 업무: 단독 업무는 제목만, 플랜 단계는 플랜 제목·단계·완료 수와 칸 막대, 자동 진행 상태.
const workOf = (name) => p.locator(".card.clickable").filter({ hasText: name }).locator(".work");
assert.equal(await workOf("에이다").locator(".work-t").textContent(), "토큰 갱신 버튼 문구 바꾸기");
assert.equal(await workOf("에이다").locator(".work-p").count(), 0);
assert.equal(await workOf("이끼").locator(".work-pt").textContent(), "웹 작업보드 출시 플랜 · 단계 3/5");
assert.equal(await workOf("이끼").locator(".work-p > b").textContent(), "2/5 완료");
assert.equal(await workOf("이끼").locator(".work-m > i.on").count(), 2);
assert.equal(await workOf("이끼").locator(".work-m > i.cur").count(), 1);
assert.equal(await workOf("이끼").locator(".work-auto.running").textContent(), "자동 진행");
assert.equal(await workOf("보리").count(), 0);
await p.locator(".card.clickable").filter({ hasText: "에이다" }).click();
await p.waitForSelector("#log .msg");
await p.waitForSelector("#sent .card");
await p.waitForSelector("#head .work .work-t");
await p.waitForSelector("#asks .card.ask");
await p.waitForFunction(() => [...document.querySelectorAll("#log img.shot")].every((i) => i.complete));
await p.screenshot({ path: `${outDir}/agent-claude.png`, fullPage: true });
// 하단 사용량 바, 답을 기다리는 동안의 상태 줄(멈춤과 구분).
await p.locator("#usagebar").filter({ hasText: "Claude" }).filter({ hasText: "96%" }).waitFor();
await p.locator("#live.human").waitFor();
const copied = () => p.evaluate(() => navigator.clipboard.readText());
const newest = p.locator("#log .msg.assistant").first();
await newest.getByRole("button", { name: "텍스트 복사" }).click();
assert.equal(await copied(), code);
await newest.getByRole("button", { name: "메시지 복사" }).click();
assert.equal(await copied(), message);
const tool = p.locator("#log details.tool").filter({ hasText: "git log --oneline -3" }).first();
await tool.locator("summary").click();
await tool.getByRole("button", { name: "텍스트 복사" }).click();
assert.equal(await copied(), "git log --oneline -3");
const result = p.locator("#log details.tool.err").filter({ hasText: "fatal: not a git repository" });
await result.locator("summary").click();
await result.getByRole("button", { name: "텍스트 복사" }).click();
assert.equal(await copied(), "fatal: not a git repository");
async function verifyDownload(button, name, bytes) {
  const [download] = await Promise.all([button.page().waitForEvent("download"), button.click()]);
  assert.equal(download.suggestedFilename(), name);
  assert.deepEqual(await readFile(await download.path()), bytes);
}
await verifyDownload(newest.getByRole("button", { name: `다운로드 · ${fileName}` }), fileName, fileBytes);
// 선택지: 하나는 고르지 않고 보내면 막히고, 둘 다 답하면 보낸다.
await p.locator("#asks button.opt", { hasText: "썸네일" }).click();
await p.getByRole("button", { name: "답 보내기" }).click();
await p.waitForSelector("text=모든 질문에 답을");
await p.locator("#asks button.opt", { hasText: "웹" }).click();
await p.locator("#asks button.opt", { hasText: "앱" }).click();
await p.screenshot({ path: `${outDir}/agent-ask.png` });
await p.getByRole("button", { name: "답 보내기" }).click();
await p.waitForFunction(() => !document.querySelector("#asks .card"));
// 답하고 나면 생각 중 상태 줄(점이 움직임).
await p.locator("#live.thinking .live-dots").waitFor();
assert.match(await p.locator("#live").textContent(), /생각 중… 1분 1\d초/);
assert.equal(await p.locator("#cost").textContent(), "이 세션 추정 사용량 · 토큰 11.7K CH(1.7M) · ~$3.45 · 12턴");
await p.screenshot({ path: `${outDir}/agent-cost.png` });
// 직접 입력: 작업 중이라 대기열에 쌓이고, 뺄 수 있다.
await p.fill("#msgtext", "다음엔 테스트도 돌려 줘");
await p.getByRole("button", { name: "대기열에 넣기" }).click();
await p.locator("#queue .q-item").filter({ hasText: "다음엔 테스트도 돌려 줘" }).waitFor();
assert.equal(await p.locator("#msgtext").inputValue(), "");
await p.locator("#msgform").screenshot({ path: `${outDir}/agent-queue.png` });
await p.locator("#queue").getByRole("button", { name: "빼기" }).click();
await p.waitForFunction(() => !document.querySelector("#queue .q-item"));
// 출력이 끊기면 멈춤으로 바뀐다(칩·상태 줄).
aOutputAgo = 45000;
await p.locator("#live.stalled").waitFor();
await p.locator("#head .chip.stalled").waitFor();
await p.screenshot({ path: `${outDir}/agent-stalled.png` });
aOutputAgo = 500;
await p.locator("#live.thinking").waitFor();
// 이미지 확대
await p.locator("#log img.shot").first().click();
await p.waitForSelector(".lb img");
await p.screenshot({ path: `${outDir}/agent-lightbox.png` });
const lightbox = p.locator(".lb");
await lightbox.getByRole("button", { name: "이미지 복사" }).click();
await p.waitForFunction(async () => (await navigator.clipboard.read()).some((item) => item.types.includes("image/png")));
await verifyDownload(lightbox.getByRole("button", { name: "이미지 다운로드" }), "image-img2.png", png);
await p.locator(".lb").click();
await p.locator("#sent button", { hasText: "완료" }).first().click();
await p.waitForFunction(() => document.querySelectorAll("#sent .row").length === 1);
await p.screenshot({ path: `${outDir}/agent-done.png` });
await p.locator("#taskform summary").click();
await p.fill("#taskform input", "로그 정렬 고치기");
await p.getByRole("button", { name: "재시작하고 보내기" }).click();
await p.getByRole("button", { name: "한 번 더 눌러 확인" }).click();
await p.waitForTimeout(300);
await p.locator(".back").click();
await p.locator(".card.clickable").filter({ hasText: "보리" }).click();
await p.waitForSelector("#taskform");
await p.waitForSelector("#asks .card.ask");
await p.screenshot({ path: `${outDir}/agent-shell.png`, fullPage: true });
// 셸 프롬프트 터미널: 고른 CLI로 띄운다(Claude·Codex 밖의 CLI도).
await p.selectOption("#head select.launch-cli", "opencode");
await p.getByRole("button", { name: "CLI 실행" }).click();
await p.waitForTimeout(200);
assert.deepEqual(posts.at(-1), { path: "/api/agents/b/launch", body: { cli: "opencode" } });
await p.locator("#head").screenshot({ path: `${outDir}/agent-launch.png` });
// 셸 프롬프트 터미널의 직접 입력은 업무 보내기처럼 CLI를 띄워 첫 프롬프트로 넣는다(OpenCode는 기본 CLI로).
await p.fill("#msgtext", "README 요약해 줘");
await p.getByRole("button", { name: "Claude 띄워 보내기" }).click();
await p.locator("#toast.show").filter({ hasText: "CLI를 띄워 보냈습니다" }).waitFor();
assert.deepEqual(posts.at(-1), { path: "/api/agents/b/message", body: { text: "README 요약해 줘", cli: "claude" } });
await p.selectOption("#head select.launch-cli", "codex");
await p.getByRole("button", { name: "Codex 띄워 보내기" }).waitFor();
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
// Tailnet·LAN에서 쓰는 평문 HTTP에서는 브라우저의 선택 영역 복사로 동작한다.
const http = await ctx.newPage();
await http.goto(base.replace("127.0.0.1", "dashboard.test"));
assert.equal(await http.evaluate(() => window.isSecureContext), false);
await http.locator(".card.clickable").filter({ hasText: "에이다" }).click();
await http.locator("#log .msg.assistant").first().getByRole("button", { name: "텍스트 복사" }).click();
await http.locator("#toast.show").filter({ hasText: "클립보드에 복사했습니다" }).waitFor();
await p.bringToFront();
assert.equal(await copied(), code);
const thumbnail = http.locator("#log .imgwrap").first();
await thumbnail.getByRole("button", { name: "이미지 복사" }).click();
await http.locator("#toast.show").filter({ hasText: "이미지를 길게 누르거나" }).waitFor();
await verifyDownload(thumbnail.getByRole("button", { name: "이미지 다운로드" }), "image-img2.png", png);
await verifyDownload(http.getByRole("button", { name: `다운로드 · ${fileName}` }), fileName, fileBytes);
// 화면 테마: 자동 → 밝게 → 어둡게. 고른 것은 브라우저에 남아 다시 열어도 그대로다.
await p.bringToFront();
const theme = () => p.evaluate(() => [document.documentElement.dataset.theme || "auto", getComputedStyle(document.body).backgroundColor]);
assert.equal((await theme())[0], "auto");
await p.locator("#theme").click();
assert.deepEqual(await theme(), ["light", "rgb(244, 242, 238)"]);
await p.locator("#theme").click();
assert.deepEqual(await theme(), ["dark", "rgb(20, 20, 18)"]);
await p.reload();
await p.locator("#theme", { hasText: "어둡게" }).waitFor();
assert.deepEqual(await theme(), ["dark", "rgb(20, 20, 18)"]);
await p.screenshot({ path: `${outDir}/theme-dark.png` });
await p.locator("#theme").click();
assert.equal((await theme())[0], "auto");

// 목록 미리보기: 활성(작업 중·답을 기다림) 캐릭터만, 최근 항목 5줄(최신 블록이 위, 도구 결과·이미지는 그 블록 아래).
await p.goto(base.replace("#t=abc", ""));
const pvLines = (id) => p.locator(`.pv-slot[data-agent="${id}"] .pv-l`);
await pvLines("a").first().waitFor();
assert.deepEqual(await pvLines("a").allTextContents(),
  ["💬 아래 명령을 실행하세요.", "👤 이 화면 좀 봐 줘", "🖼 이미지", "🖼 이미지", "🔧 Read /tmp/shot.png"]);
await pvLines("e").first().waitFor();
assert.equal(await p.locator('.pv-slot[data-agent="b"] .pv').count(), 0);
assert.ok(logReqs.some((r) => r === "/api/log/a?tail=30"), logReqs.join(" "));
await p.screenshot({ path: `${outDir}/list-preview.png`, fullPage: true });
// 접으면 상태 한 줄. 새로고침해도 같은 세션이면 접힌 채, 세션이 바뀌면 다시 펼친다.
await p.locator('.pv-slot[data-agent="a"] .pv-fold').click();
await p.locator('.pv-slot[data-agent="a"] button.pv.folded', { hasText: "▸ 작업 중" }).waitFor();
assert.equal(await p.locator(".card.clickable .name").first().textContent(), "일반 에이전트");
await p.reload();
await p.locator('.pv-slot[data-agent="a"] button.pv.folded').waitFor();
characters[0].session = { sessionId: "s1b", state: "running" }; rev++;
await pvLines("a").first().waitFor();
// 스플릿뷰: 활성 캐릭터 칸 격자, 나머지는 서랍(좁은 화면).
await p.getByRole("switch", { name: "스플릿뷰" }).click();
const paneIds = () => p.locator("#panes > .pane").evaluateAll((ps) => ps.map((x) => x.dataset.agent));
await p.locator('#panes .pane[data-agent="e"] .log .msg').first().waitFor();
// 보리는 앞에서 답을 기다리다 끝났다 — 완료로 내리기 전까지 칸이 남는다.
assert.deepEqual((await paneIds()).sort(), ["a", "b", "e"]);
await p.locator('#panes .pane[data-agent="b"] .chip.idle').waitFor();
// 칸 머리에도 진행 중인 업무와 진행도(좁은 칸이라 플랜 제목은 빼고 막대·완료 수만).
assert.equal(await p.locator('#panes .pane[data-agent="e"] .pane-head .work-p > b').textContent(), "2/5 완료");
assert.equal(await p.locator('#panes .pane[data-agent="a"] .pane-head .work-t').textContent(), "토큰 갱신 버튼 문구 바꾸기");
await p.screenshot({ path: `${outDir}/split-phone.png` });
await p.getByRole("button", { name: /다른 캐릭터 3/ }).click();
await p.locator("#split.side-open #side .card", { hasText: "도토리" }).waitFor();
await p.screenshot({ path: `${outDir}/split-drawer.png` });
await p.locator("#split .backdrop").click({ position: { x: 370, y: 400 } });
await p.waitForFunction(() => !document.querySelector("#split.side-open"));
await p.setViewportSize({ width: 1440, height: 900 });
await p.waitForSelector("#split.side-open #side");
await p.screenshot({ path: `${outDir}/split-wide.png` });
// 턴이 끝나도 칸은 남고, 완료로 내리면 사이드 패널로 간다. 다시 작업하면 칸이 돌아온다.
characters[4].turn = { phase: "idle", at: Date.now() }; rev++;
await p.locator('#panes .pane[data-agent="e"] .chip.idle').waitFor();
await p.locator('#panes .pane[data-agent="e"]').getByRole("button", { name: "완료" }).click();
await p.waitForFunction(() => !document.querySelector('#panes .pane[data-agent="e"]'));
await p.locator("#side .card", { hasText: "이끼" }).waitFor();
characters[4].turn = { phase: "working", at: Date.now(), since: Date.now() }; rev++;
await p.locator('#panes .pane[data-agent="e"]').waitFor();
// 칸 머리를 누르면 상세, 돌아가면 스플릿뷰. 켜 둔 것은 기기에 남는다.
await p.locator('#panes .pane[data-agent="a"] .pane-head .name').click();
await p.waitForSelector("#log .msg");
await p.locator(".back").click();
await p.locator("#panes .pane").first().waitFor();
await p.reload();
await p.locator('#panes .pane[data-agent="e"]').waitFor();
await p.getByRole("switch", { name: "스플릿뷰" }).click();
await pvLines("a").first().waitFor();

// ── 작업보드(업무 탭): 모바일 1열 + PC 마스터-디테일 + 플랜/자동진행/예약/편집/양방향 이동 ──
await p.setViewportSize({ width: 390, height: 844 });
await p.locator("#nav button", { hasText: "업무" }).click();
await p.locator("#tb-list .tb-group").first().waitFor();
// 진행 중 업무 카드에도 담당 캐릭터(작업 중)의 로그 미리보기가 붙는다. 다른 상태 업무에는 없다.
await p.locator('.tb-card[data-id="w2"] .pv-slot .pv-l').first().waitFor();
assert.equal(await p.locator('.tb-card[data-id="w2"] .pv-l').first().textContent(), "💬 아래 명령을 실행하세요.");
assert.equal(await p.locator('.tb-card[data-id="w3"] .pv-slot').count(), 0);
await p.screenshot({ path: `${outDir}/tasks-mobile.png`, fullPage: true });
// 완료된 업무 '더 보기'로 전체 확장
await p.locator(".tb-more-done").click();
await p.waitForFunction(() => !document.querySelector(".tb-more-done"));
// 플랜 루트 열기 → 자동 진행 일시정지 사유·로그 확인 → 계속 누르기
await p.locator('.tb-card[data-id="proot"] .tb-card-btn').click();
await p.locator("#tb-autorun .tb-ar-paused").filter({ hasText: "20건" }).waitFor();
await p.screenshot({ path: `${outDir}/tasks-autorun-mobile.png`, fullPage: true });
await p.locator("#tb-autorun").getByRole("button", { name: "계속" }).click();
await p.locator("#tb-autorun").getByRole("button", { name: "일시정지" }).waitFor();
// PC 넓은 화면 마스터-디테일
await p.setViewportSize({ width: 1440, height: 900 });
await p.locator(".tb-layout #tb-list").waitFor();
await p.locator(".tb-layout #tb-detail").waitFor();
await p.screenshot({ path: `${outDir}/tasks-wide.png` });
// 예약 카드 열어 끄기 → 다시 켜기
await p.locator('.tb-card[data-id="sc1"] .tb-card-btn').click();
await p.locator("#tb-sched").filter({ hasText: "매일 09:00" }).waitFor();
await p.locator("#tb-sched").getByRole("button", { name: "끄기" }).click();
await p.locator("#tb-sched").getByRole("button", { name: "켜기" }).waitFor();
// 새 업무 생성 (CLI/모델/추론 강도 포함)
await p.locator("#tb-list").getByRole("button", { name: "+ 새 업무" }).click();
await p.locator('#tb-detail input[type="text"]').first().fill("웹에서 만든 새 태스크");
await p.locator("#tb-detail textarea").fill("웹 작업보드 생성 검증");
await p.locator("#tb-detail select").first().selectOption("a");
await p.locator("#tb-detail select").nth(2).selectOption("claude");
await p.locator('#tb-detail input[list^="tb-models-"]').fill("claude-sonnet-4-6");
await p.locator("#tb-save-btn").click();
await p.locator(".tb-card", { hasText: "웹에서 만든 새 태스크" }).waitFor();
// 수정 중 폴링이 와도 작성 중인 초안이 덮어써지지 않는지 확인
await p.locator('#tb-detail input[type="text"]').first().fill("수정 중인 제목");
tasksRev++;
await p.waitForTimeout(400);
assert.equal(await p.locator('#tb-detail input[type="text"]').first().inputValue(), "수정 중인 제목");
await p.locator("#tb-save-btn").click();
await p.locator(".tb-card", { hasText: "수정 중인 제목" }).waitFor();
// 양방향 이동: 업무 상세 → 담당 캐릭터 대화 로그 → 보낸 업무 클릭 → 업무 상세
await p.locator("#tb-head-chips").getByRole("button", { name: /대화 로그/ }).click();
await p.locator("#sent .sent-row").first().waitFor();
await p.locator("#sent .sent-row").first().click();
await p.locator('#tb-detail[data-id="w2"]').waitFor();

// 머신 연동 호스트: 붙은 머신(studio-pc)과 끊긴 머신(laptop)의 구역
machines = [{ id: "d-1a2b3c4d", name: "studio-pc", online: true, count: 1 }, { id: "d-5e6f7a8b", name: "laptop", online: false, count: 1 }];
characters.push(
  { agentId: "d-1a2b3c4d:r1", name: "Ada", role: "", cwd: "/work/api", clockedOut: false, portrait: null, remote: true,
    machine: { id: "d-1a2b3c4d", name: "studio-pc", online: true },
    session: { sessionId: "rs1", state: "running" }, turn: { phase: "needs-human", at: Date.now() - 5000 },
    asks: [{ id: "k1", toolName: "Bash", canAlways: true, sidechain: false, at: Date.now() - 5000, toolInput: { command: "cargo test --lib", description: "Run tests" } }],
    queue: null, label: { goal: "API 테스트 고치기", cwd: "/work/api", branch: "main" } },
  { agentId: "d-5e6f7a8b:r2", name: "Cleo", role: "", cwd: "/work/web", clockedOut: false, portrait: null, remote: true,
    machine: { id: "d-5e6f7a8b", name: "laptop", online: false }, session: null, turn: null, asks: [], queue: null },
);
rev++;
await p.getByRole("button", { name: "캐릭터" }).click();
await p.locator(".machine-h", { hasText: "studio-pc" }).waitFor();
assert.equal(await p.locator(".machine-h").count(), 3, "이 머신 + 붙은 머신 + 끊긴 머신");
assert.equal(await p.locator(".card.offline", { hasText: "Cleo" }).count(), 1);
// 머신은 구역 머리에만 — 카드 아랫줄 그 자리는 폴더 말단 이름(·브랜치)이다.
assert.equal(await p.locator(".card", { hasText: "Ada" }).locator(".sub").first().textContent(), "api ⎇ main");
assert.equal(await p.locator(".card", { hasText: "studio-pc" }).count(), 0, "카드에 머신 이름을 다시 싣지 않는다");
await p.screenshot({ path: `${outDir}/list-machines.png`, fullPage: true });
await p.locator(".card", { hasText: "Ada" }).filter({ hasText: "api" }).click();
await p.locator(".ask").waitFor();
// 다른 머신 캐릭터: 메시지·권한 응답·세션 조작을 그 머신에 넘긴다(호스트가 원격 id로 받는다).
assert.equal(await p.locator("#msgform").count(), 1, "붙은 머신 캐릭터에는 직접 입력이 있다");
assert.equal(await p.locator("#taskform").count(), 0, "업무 보내기는 아직 이 머신 캐릭터만");
assert.ok(await p.locator("#head .btns").count() > 0, "조작 단추가 있다");
assert.ok(logReqs.some((r) => r.startsWith("/api/log/d-1a2b3c4d%3Ar1")), "로그는 머신:캐릭터 id로 묻는다");
await p.screenshot({ path: `${outDir}/agent-remote.png`, fullPage: true });
await p.locator(".ask").getByRole("button", { name: "허용", exact: true }).click();
await p.waitForFunction(() => !document.querySelector(".ask"));
assert.deepEqual(posts.at(-1), { path: "/api/asks/k1", body: { action: "allow", agentId: "d-1a2b3c4d:r1" } }, "원격 요청 답에는 캐릭터 id를 싣는다");
await p.locator("#msgtext").fill("원격으로 보냄");
await p.locator("#msgact button").click();
await p.waitForFunction(() => document.querySelector("#msgtext") && !document.querySelector("#msgtext").value);
assert.deepEqual(posts.at(-1), { path: "/api/agents/d-1a2b3c4d%3Ar1/message", body: { text: "원격으로 보냄" } });
await p.locator("#head .btns button", { hasText: "작업 중단" }).click();
await p.waitForTimeout(200);
assert.equal(posts.at(-1).path, "/api/agents/d-1a2b3c4d%3Ar1/interrupt");
// 끊긴 머신 캐릭터는 보기만.
await p.locator("button.back").click();
await p.locator(".card", { hasText: "Cleo" }).click();
await p.locator("#head").waitFor();
assert.equal(await p.locator("#msgform").count(), 0, "끊긴 머신 캐릭터에는 입력이 없다");
assert.equal(await p.locator("#head .btns").count(), 0, "끊긴 머신 캐릭터에는 조작 단추가 없다");
await p.screenshot({ path: `${outDir}/agent-remote-offline.png`, fullPage: true });
// 업무 탭 담당 고르기에는 이 머신 캐릭터만
assert.ok(!(await p.evaluate(() => window.Dash.getChars().some((c) => c.remote))));

console.log(JSON.stringify(posts));
await browser.close();
server.close();
