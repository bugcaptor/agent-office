// Read-only, deliberately version-sensitive transcript adapters for the spike.
// Never return prompt, assistant text, tool arguments, credentials, or file contents.
import { open, readdir, stat, realpath } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';

const MAX_LINE = 2 * 1024 * 1024;
const CHUNK = 64 * 1024;
const MAX_TICK = 4 * 1024 * 1024;

export function metadata(provider, records) {
  if (provider === 'codex') {
    const p = records.find(r => r?.type === 'session_meta')?.payload;
    if (!p || typeof (p.id ?? p.session_id) !== 'string' || typeof p.cwd !== 'string') return null;
    const vscode = p.source === 'vscode' && p.originator === 'codex_vscode' && p.thread_source === 'user';
    const subagent = Boolean(p.parent_thread_id || p.source?.subagent || p.thread_source === 'subagent' || p.thread_source?.subagent);
    return {
      provider, sourceSessionId: p.id ?? p.session_id, cwd: p.cwd,
      source: subagent ? 'subagent' : vscode ? 'vscode' : 'unknown',
      originator: typeof p.originator === 'string' ? p.originator : null,
    };
  }
  if (provider === 'claude') {
    const p = records.find(r => typeof r?.sessionId === 'string' && typeof r.cwd === 'string');
    if (!p) return null;
    return { provider, sourceSessionId: p.sessionId, cwd: p.cwd,
      source: p.isSidechain ? 'subagent' : p.entrypoint === 'claude-vscode' ? 'vscode' : 'unknown',
      originator: typeof p.entrypoint === 'string' ? p.entrypoint : null };
  }
  throw new Error('invalid-provider');
}

// Parse only complete records. An oversized/partial record is never an event.
export async function inspectTranscript(file, provider) {
  const canonical = await realpath(file);
  const handle = await open(canonical, 'r');
  try {
    const s = await handle.stat();
    if (!s.isFile()) throw new Error('not-regular-file');
    const buffer = Buffer.alloc(Math.min(s.size, MAX_LINE));
    const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0);
    const text = buffer.subarray(0, bytesRead).toString('utf8');
    const records = text.slice(0, text.lastIndexOf('\n') + 1).split('\n').flatMap(line => {
      try { return [JSON.parse(line)]; } catch { return []; }
    });
    const meta = metadata(provider, records);
    if (!meta || !path.isAbsolute(meta.cwd)) throw new Error('unsupported-transcript');
    const result = { ...meta, file: canonical, updatedAt: new Date(s.mtimeMs).toISOString(), state: 'unknown' };
    // Keep the checked file identity internally; do not include it in JSON output.
    Object.defineProperty(result, 'baseline', { value: { identity: `${s.dev}:${s.ino}`, size: s.size } });
    return result;
  } finally { await handle.close(); }
}

export function eventFromRecord(provider, r, sessionId) {
  if (!r || typeof r !== 'object') return null;
  if (provider === 'codex') {
    if (r.type === 'event_msg') {
      if (r.payload?.type === 'task_started') return { kind: 'prompt', key: r.payload.turn_id };
      if (r.payload?.type === 'task_complete') return { kind: 'stop', key: r.payload.turn_id };
    }
    if (r.type === 'response_item' && ['function_call', 'custom_tool_call'].includes(r.payload?.type)) {
      return { kind: 'tool', key: r.payload.call_id };
    }
    return null;
  }
  if (provider !== 'claude' || r.sessionId !== sessionId || r.isSidechain || r.isMeta) return null;
  if (r.type === 'user') {
    const content = r.message?.content;
    if (r.promptId && r.message?.role === 'user' && (typeof content === 'string' || (Array.isArray(content) && content.some(c => c?.type === 'text') && !content.some(c => c?.type === 'tool_result')))) {
      return { kind: 'prompt', key: r.promptId };
    }
  }
  if (r.type === 'assistant') {
    if (r.message?.stop_reason === 'end_turn') return { kind: 'stop', key: r.message.id ?? r.uuid };
    if (Array.isArray(r.message?.content) && r.message.content.some(c => c?.type === 'tool_use')) return { kind: 'tool', key: r.message.id ?? r.uuid };
  }
  return null;
}

export class EventFilter {
  constructor(provider, sessionId) { this.provider = provider; this.sessionId = sessionId; this.seen = new Set(); }
  take(record) {
    const e = eventFromRecord(this.provider, record, this.sessionId);
    if (!e) return null;
    if (e.key) {
      const key = `${e.kind}:${e.key}`;
      if (this.seen.has(key)) return null;
      this.seen.add(key);
      if (this.seen.size > 4096) this.seen.delete(this.seen.values().next().value);
    }
    return { kind: e.kind };
  }
}

export class TranscriptTail {
  static async fromNow(file, baseline) {
    const handle = await open(file, 'r');
    try {
      const s = await handle.stat();
      if (!s.isFile()) throw new Error('not-regular-file');
      if (baseline && (`${s.dev}:${s.ino}` !== baseline.identity || s.size < baseline.size)) throw new Error('transcript-replaced');
      const last = Buffer.alloc(1);
      if (s.size) await handle.read(last, 0, 1, s.size - 1);
      return new TranscriptTail(file, handle, s, s.size > 0 && last[0] !== 10);
    } catch (e) { await handle.close(); throw e; }
  }
  constructor(file, handle, s, discard) {
    this.file = file; this.handle = handle; this.identity = `${s.dev}:${s.ino}`;
    this.offset = s.size; this.pending = Buffer.alloc(0); this.discard = discard;
  }
  async read() {
    const s = await stat(this.file);
    if (`${s.dev}:${s.ino}` !== this.identity || s.size < this.offset) throw new Error('transcript-replaced');
    const end = Math.min(s.size, this.offset + MAX_TICK);
    const records = [];
    while (this.offset < end) {
      const buffer = Buffer.alloc(Math.min(CHUNK, end - this.offset));
      const { bytesRead } = await this.handle.read(buffer, 0, buffer.length, this.offset);
      if (!bytesRead) break;
      this.offset += bytesRead;
      const data = Buffer.concat([this.pending, buffer.subarray(0, bytesRead)]);
      this.pending = Buffer.alloc(0);
      let start = 0;
      for (let i = data.indexOf(10); i !== -1; i = data.indexOf(10, start)) {
        if (!this.discard && i - start <= MAX_LINE) {
          try { records.push(JSON.parse(data.subarray(start, i).toString('utf8'))); } catch { /* unfinished/corrupt data is not an event */ }
        }
        this.discard = false; start = i + 1;
      }
      if (data.length - start > MAX_LINE) this.discard = true;
      if (!this.discard) this.pending = data.subarray(start);
    }
    return records;
  }
  async close() { await this.handle.close(); }
}

export async function discover({ provider, cwd, limit = 20, allSources = false, home = os.homedir(), env = process.env } = {}) {
  const roots = [
    ['codex', path.join(env.CODEX_HOME || path.join(home, '.codex'), 'sessions'), 3],
    ['claude', path.join(env.CLAUDE_CONFIG_DIR || path.join(home, '.claude'), 'projects'), 1],
  ];
  const found = [];
  for (const [kind, root, depth] of roots) {
    if (provider && provider !== kind) continue;
    let budget = 2000;
    const candidates = [];
    async function visit(dir, remaining) {
      if (budget <= 0) return;
      let entries;
      try { entries = await readdir(dir, { withFileTypes: true }); } catch { return; }
      // Codex date folders sort newest first. Symlinks and subagent folders are not followed.
      entries.sort((a, b) => b.name.localeCompare(a.name));
      for (const e of entries) {
        if (--budget < 0) break;
        const file = path.join(dir, e.name);
        if (e.isDirectory() && remaining > 0) await visit(file, remaining - 1);
        else if (e.isFile() && e.name.endsWith('.jsonl')) {
          try { candidates.push({ file, mtime: (await stat(file)).mtimeMs }); } catch { /* concurrent removal */ }
        }
      }
    }
    await visit(root, depth);
    candidates.sort((a, b) => b.mtime - a.mtime);
    for (const { file } of candidates.slice(0, 200)) {
      try {
        const item = await inspectTranscript(file, kind);
        if (item.source !== 'subagent' && (allSources || item.source === 'vscode') && (!cwd || path.resolve(item.cwd) === path.resolve(cwd))) found.push(item);
      } catch { /* incompatible version/metadata: skip, never guess from filename */ }
    }
  }
  return found.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)).slice(0, limit);
}
