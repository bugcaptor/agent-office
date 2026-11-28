"use strict";
// 웹 상태판 "파일" 탭: 머신 간 파일 보내기(`/api/files*`).
// - 보내기: 받는 곳(머신·캐릭터)을 고르고, 이 기기(폰·PC)의 파일을 4MB씩 올리거나(이어 올리기 재시도),
//   머신의 받은 파일·캐릭터 작업 폴더를 훑어 골라(호스트는 붙은 머신도) 보낸다. 캐릭터에게는 메모·세션 알림.
// - 기록: 보낸·받은 전송의 진행률·취소·이어 보내기·지우기, 받은 파일 내려받기(브라우저가 바로 받는다).
(() => {
const D = window.Dash;
const { L, el, toast, api, confirmBtn, fmtTime, view } = D;

const T = {
  ko: {
    receiveOn: (name, dir) => `${name}: 파일 받기 켜짐 · ${dir}`, receiveOff: (name) => `${name}: 파일 받기 꺼짐(앱 설정 › 머신 연동에서 켭니다)`,
    send: "보내기", to: "받는 곳", choose: "머신이나 캐릭터를 고르세요", thisMachine: (n) => `${n} (이 머신)`, machineItself: (n) => `${n} 받은 파일 폴더`,
    offline: "연결 끊김", receiveOffShort: "받기 꺼짐",
    fromDevice: "이 기기에서 올리기", fromMachine: "머신에서 고르기",
    pick: "파일 고르기", noPicked: "보낼 파일을 고르세요.", remove: "빼기",
    source: "머신", roots: "고를 수 있는 곳", up: "↑ 위로", selected: (n) => `${n}개 고름`, emptyDir: "비어 있습니다.", truncated: "항목이 많아 앞 2000개만 보입니다.",
    note: "메모", notePh: "무엇에 쓰는 파일인지(받는 캐릭터 세션에 전합니다)", notify: (n) => `${n} 세션에 저장 위치 알리기`,
    uploading: (name, pct) => `올리는 중 ${name} ${pct}%`, sending: "보내는 중…", sent: "보냈습니다",
    history: "보낸·받은 기록", empty: "아직 주고받은 파일이 없습니다.",
    fromWho: (w) => `${w}에게서`, toWho: (w) => `${w}에게`, relay: (a, b) => `${a} → ${b} (중계)`, files: (n) => `파일 ${n}개`,
    state: { preparing: "준비 중(목록·해시)", sending: (p) => `보내는 중 ${p}%`, done: "완료", failed: "멈춤", cancelled: "취소됨" },
    cancel: "취소", resume: "이어 보내기", forget: "지우기", download: "내려받기", more: (n) => `외 ${n}개`, savedAt: "저장 위치",
    uploadFailed: "올리지 못했습니다",
  },
  en: {
    receiveOn: (name, dir) => `${name}: receiving files on · ${dir}`, receiveOff: (name) => `${name}: receiving files off (turn it on in the app under Settings › Machines)`,
    send: "Send", to: "To", choose: "Choose a machine or character", thisMachine: (n) => `${n} (this machine)`, machineItself: (n) => `${n} received files folder`,
    offline: "offline", receiveOffShort: "receiving off",
    fromDevice: "Upload from this device", fromMachine: "Pick on a machine",
    pick: "Choose files", noPicked: "Choose files to send.", remove: "Remove",
    source: "Machine", roots: "Places", up: "↑ Up", selected: (n) => `${n} selected`, emptyDir: "Empty.", truncated: "Too many items; showing the first 2000.",
    note: "Note", notePh: "What the files are for (passed to the receiving character's session)", notify: (n) => `Tell ${n}'s session where they were saved`,
    uploading: (name, pct) => `Uploading ${name} ${pct}%`, sending: "Sending…", sent: "Sent",
    history: "Sent and received", empty: "No files sent or received yet.",
    fromWho: (w) => `from ${w}`, toWho: (w) => `to ${w}`, relay: (a, b) => `${a} → ${b} (relay)`, files: (n) => `${n} file(s)`,
    state: { preparing: "Preparing (list and hashes)", sending: (p) => `Sending ${p}%`, done: "Done", failed: "Stopped", cancelled: "Cancelled" },
    cancel: "Cancel", resume: "Resume", forget: "Remove", download: "Download", more: (n) => `+${n} more`, savedAt: "Saved at",
    uploadFailed: "Upload failed",
  },
}[L];

const ERR = {
  ko: {
    "file-target-offline": "받는 머신이 끊겨 있습니다", "file-target-receive-off": "받는 머신이 파일 받기를 꺼 두었습니다", "file-target-unknown": "받는 곳을 찾을 수 없습니다",
    "file-no-paths": "보낼 파일을 고르세요", "file-receive-off": "받는 쪽이 파일 받기를 껐습니다", "file-no-space": "디스크 공간이 모자랍니다", "link-offline": "호스트에 연결되어 있지 않습니다",
    "link-device-offline": "그 머신과 연결이 끊겼습니다", "link-request-timeout": "그 머신이 제때 답하지 않았습니다", "file-source-changed": "보내는 동안 원본이 바뀌었습니다",
    "file-source-missing": "원본이 없어졌습니다", "file-hash-mismatch": "받은 파일이 원본과 다릅니다", "file-incomplete": "다 받지 못했습니다", "file-interrupted": "앱이 꺼져 멈췄습니다",
    "file-not-resumable": "이어 보낼 수 없습니다", "file-busy": "이미 진행 중입니다", "file-path-not-allowed": "받은 파일·캐릭터 작업 폴더 밖입니다", "file-write-failed": "쓰지 못했습니다",
    "file-read-failed": "읽지 못했습니다", "unknown-agent": "그 머신에 그 캐릭터가 없습니다", "files-unavailable": "파일 보내기를 쓸 수 없습니다",
  },
  en: {
    "file-target-offline": "The receiving machine is offline", "file-target-receive-off": "The receiving machine has receiving files off", "file-target-unknown": "Destination not found",
    "file-no-paths": "Choose files to send", "file-receive-off": "The receiver turned off receiving files", "file-no-space": "Not enough disk space", "link-offline": "Not connected to the host",
    "link-device-offline": "Lost the connection to that machine", "link-request-timeout": "That machine did not answer in time", "file-source-changed": "A source changed while sending",
    "file-source-missing": "A source file is gone", "file-hash-mismatch": "A received file does not match", "file-incomplete": "Not fully received", "file-interrupted": "Stopped because the app quit",
    "file-not-resumable": "Cannot be resumed", "file-busy": "Already in progress", "file-path-not-allowed": "Outside the received files and character work folders", "file-write-failed": "Could not write",
    "file-read-failed": "Could not read", "unknown-agent": "That character is not on that machine", "files-unavailable": "File transfer is not available",
  },
}[L];

function errOf(code) {
  if (!code) return "";
  const i = code.indexOf(":");
  const key = i < 0 ? code.trim() : code.slice(0, i).trim();
  const detail = i < 0 ? "" : code.slice(i + 1).trim();
  return (ERR[key] || key) + (detail ? `: ${detail}` : "");
}
function bytes(n) {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let v = n, i = 0;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return i === 0 ? `${n} B` : `${v.toFixed(1)} ${u[i]}`;
}
function label(p) { return p.agentName ? `${p.agentName}@${p.machineName}` : p.machineName; }
function keyOf(t) { return `${t.machineId}|${t.agentId || ""}`; }
function pct(t) { return t.state === "done" ? 100 : t.totalBytes > 0 ? Math.min(100, Math.floor(t.doneBytes * 100 / t.totalBytes)) : 0; }

let data = null, dataJson = "";
const st = { target: "", mode: "upload", picked: [], note: "", notify: false, busy: false, progress: "",
  browse: { machineId: null, path: null, view: null, selected: new Set(), error: "" } };

async function refresh() {
  const d = await api("/files");
  const json = JSON.stringify(d);
  if (json === dataJson) return;
  const first = !data;
  const targetsChanged = !data || JSON.stringify(data.targets) !== JSON.stringify(d.targets);
  data = d; dataJson = json;
  if (view.tab !== "files") return;
  const main = document.getElementById("main");
  const typing = document.activeElement && ["TEXTAREA", "SELECT", "INPUT"].includes(document.activeElement.tagName);
  if (first || (targetsChanged && !typing && !st.busy)) render(main);
  else {
    const hist = document.getElementById("fhist");
    if (hist) hist.replaceWith(renderHistory()); else render(main);
  }
}

function render(main) {
  if (!data) { main.replaceChildren(el("p", { class: "muted", text: "…" })); refresh().catch((e) => toast(e.message)); return; }
  main.replaceChildren(renderStatus(), renderSend(), renderHistory());
}

function renderStatus() {
  const s = data.status;
  return el("p", { class: "muted files-status", text: s.config.receiveEnabled ? T.receiveOn(s.machineName, s.receiveDir) : T.receiveOff(s.machineName) });
}

function chosenTarget() { return (data.targets || []).find((t) => keyOf(t) === st.target); }
function unavailable(t) { return !t.online ? T.offline : !t.receive ? T.receiveOffShort : null; }

function renderSend() {
  const groups = new Map();
  for (const t of data.targets || []) {
    const g = groups.get(t.machineId) || { name: t.machineName, local: t.local, items: [] };
    g.items.push(t); groups.set(t.machineId, g);
  }
  const select = el("select", { class: "files-target", onchange: (e) => { st.target = e.target.value; const t = chosenTarget(); st.notify = !!(t && t.agentId); render(document.getElementById("main")); } },
    el("option", { value: "", text: T.choose }),
    ...[...groups.values()].map((g) => el("optgroup", { label: g.local ? T.thisMachine(g.name) : g.name },
      ...g.items.map((t) => {
        const why = unavailable(t);
        const name = t.agentName ? `${t.agentName} · ${t.machineName}` : T.machineItself(t.machineName);
        return el("option", { value: keyOf(t), disabled: !!why, selected: keyOf(t) === st.target, text: why ? `${name} (${why})` : name });
      }))));
  const tabs = el("div", { class: "files-modes" },
    ...[["upload", T.fromDevice], ["browse", T.fromMachine]].map(([m, text]) =>
      el("button", { type: "button", class: st.mode === m ? "b active" : "b", text, onclick: () => { st.mode = m; render(document.getElementById("main")); } })));
  const target = chosenTarget();
  const agent = target && target.agentId;
  const ready = target && !unavailable(target) && !st.busy && (st.mode === "upload" ? st.picked.length > 0 : st.browse.selected.size > 0);
  return el("section", { class: "card files-send" },
    el("h3", { text: T.send }),
    el("label", { class: "files-field" }, el("span", { text: T.to }), select),
    tabs,
    st.mode === "upload" ? renderUpload() : renderBrowse(),
    agent && el("label", { class: "files-field" }, el("span", { text: T.note }),
      el("textarea", { rows: 2, placeholder: T.notePh, oninput: (e) => { st.note = e.target.value; } }, st.note)),
    agent && el("label", { class: "files-check" },
      el("input", { type: "checkbox", checked: st.notify || !!st.note.trim(), onchange: (e) => { st.notify = e.target.checked; } }),
      el("span", { text: T.notify(target.agentName) })),
    el("p", { class: "muted", id: "fprog", text: st.progress }),
    el("div", { class: "row" }, el("button", { class: "b primary", type: "button", disabled: !ready, text: st.busy ? T.sending : T.send, onclick: send })));
}

function renderUpload() {
  const input = el("input", { type: "file", multiple: true, hidden: true, onchange: (e) => {
    for (const f of e.target.files) if (!st.picked.some((p) => p.name === f.name && p.size === f.size)) st.picked.push(f);
    render(document.getElementById("main"));
  } });
  return el("div", { class: "files-picked" },
    st.picked.length ? el("ul", null, ...st.picked.map((f, i) => el("li", null,
      el("span", { text: f.name }), el("small", { class: "muted", text: bytes(f.size) }),
      el("button", { class: "b", type: "button", text: "×", title: T.remove, onclick: () => { st.picked.splice(i, 1); render(document.getElementById("main")); } }))))
      : el("p", { class: "muted", text: T.noPicked }),
    input,
    el("button", { class: "b", type: "button", text: T.pick, onclick: () => input.click() }));
}

async function loadBrowse(machineId, path) {
  const b = st.browse;
  b.machineId = machineId; b.path = path || null; b.error = "";
  try {
    const q = new URLSearchParams();
    if (machineId) q.set("machineId", machineId);
    if (path) q.set("path", path);
    b.view = await api(`/files/browse?${q}`);
  } catch (e) { b.error = e.message; b.view = null; }
  render(document.getElementById("main"));
}

function renderBrowse() {
  const b = st.browse;
  const sources = data.sources || [];
  if (!b.machineId && sources.length) { b.machineId = sources[0].machineId; loadBrowse(b.machineId, null); }
  const pickSource = sources.length > 1 && el("label", { class: "files-field" }, el("span", { text: T.source }),
    el("select", { onchange: (e) => { b.selected.clear(); loadBrowse(e.target.value, null); } },
      ...sources.map((s) => el("option", { value: s.machineId, disabled: !s.online, selected: s.machineId === b.machineId,
        text: s.local ? T.thisMachine(s.name) : s.online ? s.name : `${s.name} (${T.offline})` }))));
  const v = b.view;
  const roots = v && el("div", { class: "files-roots" }, el("small", { class: "muted", text: T.roots }),
    ...(v.roots || []).filter((r) => r.exists).map((r) => el("button", { class: v.path && v.path.startsWith(r.path) ? "b active" : "b", type: "button", text: r.name,
      onclick: () => loadBrowse(b.machineId, r.path) })));
  const list = v && v.path && el("div", { class: "files-browse" },
    el("div", { class: "row" },
      v.parent && el("button", { class: "b", type: "button", text: T.up, onclick: () => loadBrowse(b.machineId, v.parent) }),
      el("code", { class: "files-path", text: v.path })),
    v.entries && v.entries.length ? el("ul", null, ...v.entries.map((e) => el("li", null,
      el("input", { type: "checkbox", checked: b.selected.has(e.path), onchange: (ev) => { ev.target.checked ? b.selected.add(e.path) : b.selected.delete(e.path); render(document.getElementById("main")); } }),
      e.dir ? el("button", { class: "linkish", type: "button", text: `📁 ${e.name}`, onclick: () => loadBrowse(b.machineId, e.path) })
        : el("span", { text: e.name }),
      !e.dir && el("small", { class: "muted", text: bytes(e.size) }))))
      : el("p", { class: "muted", text: T.emptyDir }),
    v.truncated && el("p", { class: "muted", text: T.truncated }));
  return el("div", { class: "files-browse-wrap" }, pickSource, b.error && el("p", { class: "err", text: b.error }), roots, list,
    b.selected.size > 0 && el("p", { class: "muted", text: T.selected(b.selected.size) }));
}

/** 파일 하나를 4MB씩 올린다(청크마다 세 번까지 다시 시도). */
async function upload(file, onProgress) {
  const { id } = await api("/files/uploads", { method: "POST", body: { name: file.name, size: file.size } });
  const CH = 4 * 1024 * 1024;
  let offset = 0;
  while (offset < file.size) {
    const end = Math.min(file.size, offset + CH);
    let done = false;
    for (let attempt = 0; attempt < 3 && !done; attempt++) {
      try {
        const res = await fetch(`/api/files/uploads/${encodeURIComponent(id)}?offset=${offset}`, {
          method: "PUT", headers: { Authorization: "Bearer " + D.token() }, body: file.slice(offset, end), cache: "no-store",
        });
        const j = await res.json();
        if (j.ok) { offset = j.data.received; done = true; }
        else if (attempt === 2) throw new Error(errOf(j.error));
      } catch (e) {
        if (attempt === 2) throw e;
        await new Promise((r) => setTimeout(r, 800 * (attempt + 1)));
      }
    }
    onProgress(file.size ? Math.floor(offset * 100 / file.size) : 100);
  }
  return id;
}

async function send() {
  const target = chosenTarget();
  if (!target) return;
  st.busy = true; render(document.getElementById("main"));
  try {
    const body = { machineId: target.machineId, agentId: target.agentId || null, note: st.note.trim() || null, notify: !!(st.notify || st.note.trim()) };
    if (st.mode === "upload") {
      const ids = [];
      for (const f of st.picked) {
        ids.push(await upload(f, (p) => { st.progress = T.uploading(f.name, p); const n = document.getElementById("fprog"); if (n) n.textContent = st.progress; }));
      }
      body.uploads = ids;
    } else {
      body.sourceMachineId = st.browse.machineId;
      body.paths = [...st.browse.selected];
    }
    await api("/files/send", { method: "POST", body });
    toast(T.sent);
    st.picked = []; st.note = ""; st.browse.selected.clear();
  } catch (e) {
    toast(e.code ? errOf(e.code) : `${T.uploadFailed}: ${e.message}`);
  } finally {
    st.busy = false; st.progress = "";
    dataJson = "";
    await refresh().catch(() => {});
    render(document.getElementById("main"));
  }
}

function direction(t) {
  const me = data.status.machineId;
  const from = t.from.machineId === me, to = t.to.machineId === me;
  return from && to ? "local" : from ? "sent" : to ? "received" : "relay";
}

function renderHistory() {
  const list = [...(data.transfers || [])];
  return el("section", { class: "card files-history", id: "fhist" },
    el("h3", { text: T.history }),
    list.length ? el("ul", { class: "files-list" }, ...list.map(row)) : el("p", { class: "muted", text: T.empty }));
}

function row(t) {
  const dir = direction(t);
  const icon = { sent: "↑", received: "↓", local: "↔", relay: "⇄" }[dir];
  const who = dir === "received" ? T.fromWho(label(t.from)) : dir === "relay" ? T.relay(label(t.from), label(t.to)) : T.toWho(label(t.to));
  const active = t.state === "preparing" || t.state === "sending";
  const p = pct(t);
  const stateText = t.state === "sending" ? T.state.sending(p) : T.state[t.state];
  const downloads = t.state === "done" ? t.files.slice(0, 20).map((f, i) =>
    el("a", { class: "b", href: `/api/files/t/${encodeURIComponent(t.id)}/file/${i}?t=${D.token()}`, download: f.path.split("/").pop(), text: `⬇ ${f.path}` })) : [];
  return el("li", { class: `files-row files-${t.state}` },
    el("div", { class: "files-line" }, el("b", { text: `${icon} ${t.items.join(", ")}` })),
    el("small", { class: "muted", text: `${who} · ${T.files(t.files.length)} · ${bytes(t.totalBytes)} · ${fmtTime(t.createdAt)}` }),
    t.state === "sending" && el("div", { class: "files-bar" }, el("span", { style: `width:${p}%` })),
    el("small", { class: t.state === "failed" ? "err" : "muted", text: stateText + (t.state === "failed" && t.error ? ` — ${errOf(t.error)}` : "") }),
    t.note && el("small", { class: "muted", text: `“${t.note}”` }),
    t.saved && t.saved.length > 0 && el("small", { class: "muted", text: `${T.savedAt}: ${t.saved.join(", ")}` }),
    downloads.length > 0 && el("div", { class: "files-downloads" }, ...downloads, t.files.length > 20 && el("small", { class: "muted", text: T.more(t.files.length - 20) })),
    el("div", { class: "row" },
      active && confirmBtn(T.cancel, () => op(t.id, "cancel"), "b"),
      t.state === "failed" && dir !== "received" && el("button", { class: "b", type: "button", text: T.resume, onclick: () => op(t.id, "resume") }),
      !active && el("button", { class: "b", type: "button", text: T.forget, onclick: () => op(t.id, "forget") })));
}

async function op(id, what) {
  try { await api(`/files/t/${encodeURIComponent(id)}/${what}`, { method: "POST", body: {} }); dataJson = ""; await refresh(); }
  catch (e) { toast(e.code ? errOf(e.code) : e.message); }
}

D.files = { render, refresh };
})();
