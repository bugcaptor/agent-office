// scripts/dash-shot.mjs
//
// 웹 상태판(src-tauri/web/dash.html)을 가짜 API로 띄워 폰 크기로 찍는다. 앱·Tauri 없이 화면만 본다.
//   node scripts/dash-shot.mjs [출력 폴더(기본 /tmp/dash-shot)]
// 찍는 것: 캐릭터 목록(머리에 컴퓨터 이름, 맨 위 일반 에이전트), 캐릭터 화면(보낸 업무·대화 로그 최신순·업무 보내기),
// CLI 실행 중 재시작 경고, 셸 프롬프트에서 고른 CLI 띄우기, 끝난 업무 완료, 일반 에이전트 새 지시·이어서 보내기,
// 대화 로그 속 이미지(썸네일·확대), 권한 요청·선택지 카드(AskUserQuestion 답·Bash 허용),
// 텍스트·이미지 클립보드 복사와 이미지·파일 다운로드, 하단 사용량 바, 생각 중·멈춤 상태 줄,
// 직접 입력과 보낼 대기열(쌓기·빼기), 로그 위 이 세션 추정 사용량, 화면 테마 토글(브라우저에 저장).
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const outDir = resolve(process.argv[2] || "/tmp/dash-shot");
const page = await readFile(resolve("src-tauri/web/dash.html"), "utf8");
const now = Date.now();
const characters = [
  {
    agentId: "a", name: "에이다", role: "개발", cwd: "/Volumes/ssd4tb/dev/very/long/path/to/agent-office", portrait: 1,
    clockedOut: false, defaultCli: "claude", session: { sessionId: "s1", state: "running" },
    turn: { phase: "working", at: now, since: now - 75000 }, waiting: [], cli: "claude", atPrompt: false, queue: { items: [] },
    asks: [{ id: "ask-1", toolName: "AskUserQuestion", canAlways: false, sidechain: false, at: now, toolInput: { questions: [
      { question: "어떤 방식으로 이미지를 보여 줄까요?", header: "표시", multiSelect: false, options: [
        { label: "썸네일", description: "로그 안에 작게 보이고 누르면 확대" }, { label: "링크만", description: "누르면 새 창" }] },
      { question: "어디에 보일까요?", header: "위치", multiSelect: true, options: [{ label: "웹" }, { label: "앱" }] },
    ] } }],
    label: { goal: "웹 상태판 대화 로그 정리", cwd: "/Volumes/ssd4tb/dev/very/long/path/to/agent-office", branch: "main",
      usage: { sessionId: "s1", tokens: "11.7K CH(1.7M)", cost: "~$3.45", turns: 12 } },
  },
  {
    agentId: "b", name: "보리", role: "리뷰", cwd: "/Users/me/work/backend-service",
    clockedOut: false, session: { sessionId: "s2", state: "running" }, turn: { phase: "idle", at: now - 1000 },
    waiting: [], cli: null, atPrompt: true, label: { cwd: "/Users/me/work/backend-service" },
    asks: [{ id: "ask-2", toolName: "Bash", canAlways: true, sidechain: false, at: now,
      toolInput: { command: "npm run build && rm -rf dist/tmp", description: "빌드하고 임시 폴더 정리" } }],
  },
  // 목록 정렬: 답을 기다림 → 작업 중 → 나머지는 최근 턴 관측 순, 관측이 없으면 맨 뒤.
  { agentId: "c", name: "체리", role: "문서", cwd: "/Users/me/docs", clockedOut: true, session: null, turn: null, waiting: [], asks: [] },
  { agentId: "d", name: "도토리", role: "테스트", cwd: "/Users/me/work/backend-service", clockedOut: false,
    session: { sessionId: "s4", state: "running" }, turn: { phase: "idle", at: now - 60000 }, waiting: [], asks: [], cli: "claude", atPrompt: false },
  { agentId: "e", name: "이끼", role: "빌드", cwd: "/Users/me/work/backend-service", clockedOut: false,
    session: { sessionId: "s5", state: "running" }, turn: { phase: "working", at: now - 300000, since: now - 300000 }, waiting: [], asks: [], cli: "codex", atPrompt: false },
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
  if (url.pathname === "/api/state") {
    return send({ app: "0.0.0", host: "회사 맥북 프로", rev: `r${rev}`, characters, officeAgent, folders, home: "/Users/me", outputAt: { a: Date.now() - aOutputAgo }, now: Date.now() });
  }
  if (url.pathname === "/api/usage") return send(usage);
  if (url.pathname.startsWith("/api/log/")) return send({ epoch: 1, last: items.at(-1).seq, attached: true, items: url.searchParams.get("after") ? [] : items });
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
  if (url.pathname === "/api/tasks" && req.method === "GET") return send(tasks);
  if (req.method === "POST") {
    let body = ""; for await (const chunk of req) body += chunk;
    posts.push({ path: url.pathname, body: JSON.parse(body || "{}") });
    const done = url.pathname.match(/^\/api\/tasks\/([^/]+)\/done$/);
    if (done) tasks.open = tasks.open.filter((t) => t.id !== decodeURIComponent(done[1]));
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
await p.locator(".card.clickable").filter({ hasText: "에이다" }).click();
await p.waitForSelector("#log .msg");
await p.waitForSelector("#sent .card");
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
console.log(JSON.stringify(posts));
await browser.close();
server.close();
