"use strict";
// 셸 캐릭터 화면: 말로 시키면 LLM이 명령 하나를 제안하고, 사람이 [실행]을 눌러야 셸에 들어간다.
// 출력은 터미널 화면이 아니라 명령마다 블록으로 보인다(최신이 위). dash.js의 window.Dash에 붙는다.
(() => {
const D = window.Dash;
if (!D) return;
const { el, toast, api, copyText, L } = D;

const T = {
  ko: {
    ask: "말로 시키기", askPh: "하고 싶은 일을 쓰세요 (예: 이 폴더에서 큰 파일 10개). !로 시작하면 그 명령을 그대로 실행합니다.",
    make: "명령 만들기", making: (s) => `명령 만드는 중… ${s}초`, runNow: "바로 실행",
    help: "명령은 제안만 하고 [실행]을 눌러야 셸에 들어갑니다. 같은 셸 세션이 이어지므로 cd·export·가상환경이 다음 명령에도 남습니다.",
    noLlm: "쓸 수 있는 LLM이 없습니다. 앱 설정 › 셸 캐릭터에서 로컬 LLM 주소를 넣거나 클라우드 폴백을 켜세요. !로 시작하는 명령은 지금도 됩니다.",
    proposal: "제안한 명령", run: "실행", edit: "수정", again: "다시", cancel: "취소", source: (s) => `답한 곳: ${s}`,
    risky: "⚠ 되돌리기 어려운 명령일 수 있습니다. 확인하고 실행하세요.", interactive: "전체 화면·대화형 명령입니다. 화면은 기록되지 않고, 멈추면 아래 Ctrl-C로 끊으세요.",
    multiline: "여러 줄 명령이라 그대로 실행할 수 없습니다. 한 줄로 고쳐 주세요.",
    ran: "셸에 넣었습니다",
    prompt: (cwd) => `프롬프트 대기 · ${cwd}`, running: (cmd, t) => `실행 중 · ${cmd} · ${t}`, unknown: "셸 표식을 기다리는 중입니다. 계속 이 상태면 이 셸은 명령 블록을 지원하지 않습니다(zsh·bash·PowerShell만).",
    off: "터미널이 꺼져 있습니다. 위에서 터미널 재시작을 누르세요.", ctrlc: "중단 (Ctrl-C)",
    blocks: "명령 기록", noBlocks: "아직 실행한 명령이 없습니다.",
    exitOk: "성공", exitBad: (c) => `실패 ${c}`, exitUnknown: "끝남", runningTag: "실행 중",
    fromWeb: "웹에서 실행", fromTerm: "터미널에서 입력",
    output: (n) => `출력 ${n}줄`, dropped: (n) => `앞 ${n}줄은 버렸습니다`, fullscreen: "전체 화면 앱을 썼습니다(화면은 기록하지 않음)",
    explaining: "설명 중…", explainFailed: (e) => `설명 실패: ${e}`, explainOff: "설명할 LLM이 없습니다",
    copyCmd: "명령 복사", copyOut: "출력 복사", reuse: "입력칸에 넣기",
    err: {
      "shell-llm-none": "쓸 수 있는 LLM이 없습니다(앱 설정 › 셸 캐릭터)", "shell-llm-failed": "LLM 호출에 실패했습니다",
      "shell-running": "명령이 실행 중입니다. 끝나거나 Ctrl-C로 끊은 뒤 실행하세요", "shell-multiline": "여러 줄 명령은 실행할 수 없습니다",
      "shell-not-enabled": "셸 캐릭터가 아닙니다", "not-ready": "앱이 아직 준비되지 않았습니다",
    },
  },
  en: {
    ask: "Ask in words", askPh: "Say what you want (e.g. the 10 largest files here). Start with ! to run that command as is.",
    make: "Make command", making: (s) => `Making a command… ${s}s`, runNow: "Run now",
    help: "Commands are only proposed; they go into the shell when you press Run. It is the same shell session, so cd, export and virtualenvs carry over.",
    noLlm: "No LLM is available. Add a local LLM address or turn on cloud fallback in the app: Settings › Shell characters. Commands starting with ! still work.",
    proposal: "Proposed command", run: "Run", edit: "Edit", again: "Retry", cancel: "Cancel", source: (s) => `Answered by: ${s}`,
    risky: "⚠ This may be hard to undo. Check before running.", interactive: "Full-screen or interactive command. Its screen isn't recorded; stop it with Ctrl-C below if it hangs.",
    multiline: "Multi-line commands can't run as is. Make it one line.",
    ran: "Sent to the shell",
    prompt: (cwd) => `At the prompt · ${cwd}`, running: (cmd, t) => `Running · ${cmd} · ${t}`, unknown: "Waiting for shell marks. If this stays, this shell doesn't support command blocks (zsh, bash and PowerShell only).",
    off: "The terminal is off. Use Restart terminal above.", ctrlc: "Stop (Ctrl-C)",
    blocks: "Commands", noBlocks: "No commands yet.",
    exitOk: "OK", exitBad: (c) => `Failed ${c}`, exitUnknown: "Ended", runningTag: "Running",
    fromWeb: "Run from the web", fromTerm: "Typed in the terminal",
    output: (n) => `${n} lines of output`, dropped: (n) => `${n} earlier lines dropped`, fullscreen: "Used a full-screen app (screen not recorded)",
    explaining: "Explaining…", explainFailed: (e) => `Explanation failed: ${e}`, explainOff: "No LLM to explain",
    copyCmd: "Copy command", copyOut: "Copy output", reuse: "Put in the input",
    err: {
      "shell-llm-none": "No LLM is available (Settings › Shell characters)", "shell-llm-failed": "The LLM call failed",
      "shell-running": "A command is running. Wait or stop it with Ctrl-C first", "shell-multiline": "Multi-line commands can't run",
      "shell-not-enabled": "Not a shell character", "not-ready": "The app isn't ready yet",
    },
  },
}[L];

/** `shell-llm-failed: 이유` 같은 코드는 앞부분만 번역하고 이유를 덧붙인다. */
function errOf(e) {
  const code = (e && (e.code || e.message)) || String(e);
  const [head, ...rest] = String(code).split(": ");
  const base = T.err[head] || D.errText(head);
  return rest.length ? `${base} — ${rest.join(": ")}` : base;
}

const states = new Map();
function stateOf(agentId) {
  let s = states.get(agentId);
  if (!s) { s = { data: null, rev: null, text: "", busy: false, busySince: 0, proposal: null, editing: false, open: new Set() }; states.set(agentId, s); }
  return s;
}

function fmtDur(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  return s >= 60 ? `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}` : `${s}s`;
}

// ── 입력·제안 ──
function renderAsk(c) {
  const s = stateOf(c.agentId);
  const direct = s.text.trim().startsWith("!");
  const ta = el("textarea", { id: "shtext", rows: 2, placeholder: T.askPh, oninput: (e) => { s.text = e.target.value; refreshAskButtons(c); },
    onkeydown: (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); submit(c); } } }, s.text);
  const data = s.data;
  return el("div", { class: "card", id: "shask" },
    el("h3", { text: T.ask }),
    ta,
    renderAskButtons(c, direct),
    data && data.llm === false && el("div", { class: "note warn-note", text: T.noLlm }),
    el("div", { class: "note", text: T.help }));
}
function renderAskButtons(c, direct) {
  const s = stateOf(c.agentId);
  const label = s.busy ? T.making(Math.floor((Date.now() - s.busySince) / 1000)) : direct ? T.runNow : T.make;
  return el("div", { class: "btns", id: "shaskbtn", "data-key": JSON.stringify([label, s.busy]) },
    el("button", { class: "b primary", disabled: s.busy, text: label, onclick: () => submit(c) }));
}
function refreshAskButtons(c) {
  const box = document.getElementById("shaskbtn");
  if (!box) return;
  const next = renderAskButtons(c, stateOf(c.agentId).text.trim().startsWith("!"));
  if (next.dataset.key !== box.dataset.key) box.replaceWith(next);
}
async function submit(c) {
  const s = stateOf(c.agentId);
  if (s.busy) return;
  const text = s.text.trim();
  if (!text) { toast(D.errText("empty")); return; }
  if (text.startsWith("!")) {
    const cmd = text.slice(1).trim();
    if (await run(c, cmd)) { s.text = ""; const ta = document.getElementById("shtext"); if (ta) ta.value = ""; refreshAskButtons(c); }
    return;
  }
  await suggest(c, text);
}
async function suggest(c, text) {
  const s = stateOf(c.agentId);
  s.busy = true; s.busySince = Date.now(); s.proposal = null; s.editing = false;
  refreshAskButtons(c); refreshProposal(c);
  const timer = setInterval(() => refreshAskButtons(c), 1000);
  try {
    s.proposal = { ...(await api(`/shell/${encodeURIComponent(c.agentId)}/suggest`, { method: "POST", body: { text } })), request: text };
  } catch (e) { toast(errOf(e)); }
  finally { clearInterval(timer); s.busy = false; refreshAskButtons(c); refreshProposal(c); }
}
async function run(c, command) {
  if (!command) { toast(D.errText("empty")); return false; }
  try {
    await api(`/shell/${encodeURIComponent(c.agentId)}/run`, { method: "POST", body: { command } });
    toast(T.ran);
    stateOf(c.agentId).rev = null;
    setTimeout(D.refreshNow, 300);
    return true;
  } catch (e) { toast(errOf(e)); return false; }
}

function renderProposal(c) {
  const s = stateOf(c.agentId);
  const p = s.proposal;
  const box = el("div", { id: "shprop" });
  if (!p) return box;
  const cmd = el("textarea", { class: "sh-cmd", rows: Math.min(6, Math.max(1, p.command.split("\n").length)), readonly: !s.editing, spellcheck: "false", autocapitalize: "off",
    oninput: (e) => { p.command = e.target.value; p.multiline = p.command.includes("\n"); } }, p.command);
  const card = el("div", { class: "card sh-prop" + (p.risky ? " risky" : "") },
    el("h3", { text: T.proposal }),
    cmd,
    p.explanation && el("div", { class: "sh-expl", text: p.explanation }),
    p.risky && el("div", { class: "note warn-note", text: T.risky }),
    p.interactive && el("div", { class: "note", text: T.interactive }),
    p.multiline && el("div", { class: "note warn-note", text: T.multiline }),
    el("div", { class: "btns" },
      el("button", { class: "b primary", text: T.run, onclick: async () => {
        if (!(await run(c, p.command.trim()))) return;
        s.proposal = null; refreshProposal(c);
        if (s.text.trim() === p.request) { s.text = ""; const ta = document.getElementById("shtext"); if (ta) ta.value = ""; refreshAskButtons(c); }
      } }),
      el("button", { class: "b", text: T.edit, disabled: s.editing, onclick: () => { s.editing = true; refreshProposal(c); const t = document.querySelector("#shprop .sh-cmd"); if (t) { t.focus(); t.setSelectionRange(t.value.length, t.value.length); } } }),
      el("button", { class: "b", text: T.again, onclick: () => suggest(c, p.request) }),
      el("button", { class: "b", text: T.cancel, onclick: () => { s.proposal = null; refreshProposal(c); } })),
    p.source && el("div", { class: "note", text: T.source(p.source) }));
  box.append(card);
  return box;
}
function refreshProposal(c) {
  const box = document.getElementById("shprop");
  if (box) box.replaceWith(renderProposal(c));
}

// ── 지금 상태 ──
function renderStatus(c) {
  const s = stateOf(c.agentId);
  const d = s.data;
  const box = el("div", { id: "shstatus", class: "card sh-status" });
  const running = c.session && c.session.state === "running";
  if (!running) { box.append(el("div", { class: "note", text: T.off })); return box; }
  if (!d || d.phase === "unknown") { box.append(el("div", { class: "note", text: T.unknown })); return box; }
  if (d.phase === "running") {
    const cur = (d.blocks || []).at(-1);
    const since = cur ? cur.startedAt : Date.now();
    box.classList.add("busy");
    box.append(el("div", { class: "row" },
      el("span", { class: "live-dots", "aria-hidden": "true" }, el("i"), el("i"), el("i")),
      el("div", { class: "grow sh-now", "data-since": String(since), text: T.running(cur ? cur.command : "", fmtDur(D.nowMs() - since)) }),
      el("button", { class: "b danger", text: T.ctrlc, onclick: () => D.act(`/agents/${encodeURIComponent(c.agentId)}/ctrl-c`) })));
    const tail = [...(cur && cur.output ? cur.output.split("\n").slice(-12) : []), ...(d.live || [])].slice(-15).join("\n");
    if (tail.trim()) box.append(el("pre", { class: "sh-tail", text: tail }));
    if (cur && cur.fullscreen) box.append(el("div", { class: "note", text: T.fullscreen }));
    return box;
  }
  box.append(el("div", { class: "sub", title: d.cwd || "", text: T.prompt(D.tildePath ? D.tildePath(d.cwd || "") : (d.cwd || "")) }));
  return box;
}
setInterval(() => {
  const now = document.querySelector("#shstatus .sh-now");
  const c = D.view.agent && D.charOf(D.view.agent);
  const d = c && stateOf(c.agentId).data;
  const cur = d && (d.blocks || []).at(-1);
  if (now && cur) now.textContent = T.running(cur.command, fmtDur(D.nowMs() - Number(now.dataset.since)));
}, 1000);

// ── 명령 기록 ──
function exitChip(b) {
  if (!b.endedAt) return el("span", { class: "chip working", text: T.runningTag });
  if (b.exit === 0) return el("span", { class: "chip idle", text: T.exitOk });
  if (b.exit != null) return el("span", { class: "chip bad", text: T.exitBad(b.exit) });
  return el("span", { class: "chip off", text: T.exitUnknown });
}
function renderExplanation(b) {
  const x = b.explanation;
  if (!x) return null;
  if (x.state === "pending") return el("div", { class: "sh-expl muted", text: "💬 " + T.explaining });
  if (x.state === "done") return el("div", { class: "sh-expl" }, "💬 ", x.text, x.source ? el("span", { class: "sh-src", text: " · " + x.source }) : null);
  if (x.state === "failed") return el("div", { class: "sh-expl muted", text: "💬 " + T.explainFailed(errOf({ code: x.error })) });
  return null;
}
function renderBlock(c, b) {
  const s = stateOf(c.agentId);
  const lines = b.output ? b.output.split("\n").length : 0;
  const key = String(b.id);
  const out = b.output ? el("details", { class: "tool sh-out", open: lines <= 12 || s.open.has(key), ontoggle: (e) => { if (e.target.open) s.open.add(key); else s.open.delete(key); } },
    el("summary", { text: T.output(b.lines) }),
    b.dropped ? el("div", { class: "note", text: T.dropped(b.dropped) }) : null,
    el("pre", { text: b.output })) : null;
  return el("div", { class: "card sh-block", "data-id": key },
    el("div", { class: "row sh-head" },
      el("code", { class: "grow sh-line", title: b.cwd || "", text: "$ " + b.command }),
      el("span", { class: "sh-origin", title: b.origin === "web" ? T.fromWeb : T.fromTerm, text: b.origin === "web" ? "🌐" : "⌨" }),
      exitChip(b)),
    el("div", { class: "sub", text: [D.fmtTime(b.startedAt), b.endedAt ? fmtDur(b.endedAt - b.startedAt) : null, b.cwd ? (D.tildePath ? D.tildePath(b.cwd) : b.cwd) : null].filter(Boolean).join(" · ") }),
    b.fullscreen ? el("div", { class: "note", text: T.fullscreen }) : null,
    out,
    renderExplanation(b),
    el("div", { class: "content-actions" },
      el("button", { class: "b", type: "button", text: T.copyCmd, onclick: () => copyText(b.command) }),
      b.output && el("button", { class: "b", type: "button", text: T.copyOut, onclick: () => copyText(b.output) }),
      el("button", { class: "b", type: "button", text: T.reuse, onclick: () => {
        s.text = "!" + b.command;
        const ta = document.getElementById("shtext");
        if (ta) { ta.value = s.text; ta.focus(); window.scrollTo(0, 0); }
        refreshAskButtons(c);
      } })));
}
function renderBlocks(c) {
  const d = stateOf(c.agentId).data;
  const blocks = (d && d.blocks) || [];
  const box = el("div", { id: "shblocks" }, el("h2", { text: T.blocks }));
  // 진행 중인 블록은 위 상태 칸에 보이므로 기록에는 끝난 것만 싣는다.
  const done = blocks.filter((b) => b.endedAt).reverse();
  if (!done.length) box.append(el("div", { class: "empty", text: T.noBlocks }));
  for (const b of done) box.append(renderBlock(c, b));
  return box;
}

// ── dash.js가 부르는 곳 ──
function render(c) {
  return el("div", { id: "shell" }, renderAsk(c), renderProposal(c), renderStatus(c), renderBlocks(c));
}
/** 상태를 받아 오고, 바뀌었으면 상태 칸과 기록만 다시 그린다(입력칸은 건드리지 않는다). */
async function refresh(agentId) {
  const s = stateOf(agentId);
  const q = s.rev != null ? `?rev=${s.rev}` : "";
  const d = await api(`/shell/${encodeURIComponent(agentId)}${q}`);
  if (d.same) return false;
  const llmChanged = !s.data || s.data.llm !== d.llm;
  s.data = d; s.rev = d.rev;
  const c = D.charOf(agentId);
  if (!c || D.view.agent !== agentId || !document.getElementById("shell")) return true;
  if (llmChanged) document.getElementById("shask").replaceWith(renderAsk(c));
  update(c);
  return true;
}
function update(c) {
  const status = document.getElementById("shstatus");
  if (status) status.replaceWith(renderStatus(c));
  const blocks = document.getElementById("shblocks");
  if (blocks) blocks.replaceWith(renderBlocks(c));
}
D.shell = { render, refresh, update };
})();
