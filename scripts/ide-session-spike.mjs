#!/usr/bin/env node
// Experimental read-only bridge. JSON output is metadata only, never transcript text.
import { readFile } from 'node:fs/promises';
import { parseArgs } from 'node:util';
import { randomUUID } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { pathToFileURL } from 'node:url';
import { discover, inspectTranscript, TranscriptTail, EventFilter } from './ide-session-spike/transcripts.mjs';
import { appDataDir, ControlClient, sendEvent } from './ide-session-spike/control.mjs';

const emit = data => process.stdout.write(`${JSON.stringify(data)}\n`);
export async function main(args = process.argv.slice(2)) {
  const { values: v, positionals } = parseArgs({ args, allowPositionals: true, options: {
    provider: { type: 'string' }, file: { type: 'string' }, agent: { type: 'string' },
    cwd: { type: 'string' }, limit: { type: 'string', default: '20' }, 'app-data': { type: 'string' },
    'allow-unknown-source': { type: 'boolean' }, 'dry-run': { type: 'boolean' },
    'all-sources': { type: 'boolean' },
    lang: { type: 'string', default: 'ko' }, help: { type: 'boolean' },
  } });
  const command = positionals[0] || 'help';
  if (v.help || command === 'help') {
    const lang = ['ko', 'en', 'ja', 'fr', 'zh-Hans', 'zh-Hant'].includes(v.lang) ? v.lang : 'en';
    const messages = JSON.parse(await readFile(new URL(`../src/shared/i18n/locales/${lang}/common.json`, import.meta.url), 'utf8'));
    emit({ description: messages.ideSpikeDescription, commands: [
      'list [--provider codex|claude] [--cwd PATH] [--limit 20] [--all-sources]',
      'list --provider codex|claude --file PATH',
      'agents [--app-data PATH]',
      'watch --provider codex|claude --file PATH --agent ID [--app-data PATH]',
      'watch --provider codex|claude --file PATH --dry-run',
    ], docs: 'docs/ide-session-spike.md' });
    return;
  }
  if (positionals.length > 1 || (v.provider && !['codex', 'claude'].includes(v.provider))) throw new Error('invalid-arguments');
  if (command === 'list') {
    const limit = Number(v.limit);
    if (!Number.isInteger(limit) || limit < 1 || limit > 200) throw new Error('invalid-limit');
    if (v.file && !v.provider) throw new Error('invalid-arguments');
    emit({ sessions: v.file ? [await inspectTranscript(v.file, v.provider)] : await discover({ provider: v.provider, cwd: v.cwd, limit, allSources: v['all-sources'] }),
      boundedScan: true, runningProcessVerified: false });
    return;
  }
  if (command === 'agents') {
    emit({ agents: await new ControlClient(appDataDir(v['app-data'])).request('list') });
    return;
  }
  if (command !== 'watch' || !v.provider || !v.file || (!v.agent && !v['dry-run'])) throw new Error('invalid-arguments');
  const meta = await inspectTranscript(v.file, v.provider);
  if (meta.source === 'subagent') throw new Error('subagent-not-supported');
  if (meta.source !== 'vscode' && !v['allow-unknown-source']) throw new Error('source-not-vscode');
  const tail = await TranscriptTail.fromNow(meta.file, meta.baseline);
  const filter = new EventFilter(meta.provider, meta.sourceSessionId);
  const client = v['dry-run'] ? null : new ControlClient(appDataDir(v['app-data']));
  let connection;
  let stopping = false;
  const stop = () => { stopping = true; };
  process.on('SIGINT', stop); process.on('SIGTERM', stop);
  try {
    if (client) {
      const ownerId = randomUUID();
      const attached = await client.request('observed/attach', {
        agentId: v.agent, provider: meta.provider, sourceSessionId: meta.sourceSessionId,
        cwd: meta.cwd, ownerId, pid: process.pid,
      });
      if (typeof attached?.sessionId !== 'string') throw new Error('invalid-control-response');
      connection = { agentId: v.agent, sessionId: attached.sessionId, ownerId };
    }
    emit({ status: client ? 'connected' : 'dry-run', ...meta, agentId: v.agent ?? null,
      attentionSupported: false, historyReplayed: false, runningProcessVerified: false });
    let sequence = 0;
    let heartbeatAt = 0;
    while (!stopping) {
      const records = await tail.read();
      for (const record of records) {
        const event = filter.take(record);
        if (!event) continue;
        sequence++;
        if (client) await sendEvent(client, connection, sequence, event.kind);
        emit({ status: 'event', provider: meta.provider, sourceSessionId: meta.sourceSessionId, sequence, ...event });
      }
      if (client && Date.now() - heartbeatAt >= 5000) {
        await sendEvent(client, connection, ++sequence, 'heartbeat');
        heartbeatAt = Date.now();
      }
      if (!stopping) await delay(500);
    }
  } finally {
    process.off('SIGINT', stop); process.off('SIGTERM', stop);
    await tail.close();
    if (client && connection) {
      try { await client.request('observed/detach', connection); }
      catch { emit({ status: 'detach-pending', reason: 'connector-lease-will-expire' }); }
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(e => {
    const code = typeof e.message === 'string' && /^[a-z]+(?:-[a-z]+)+$/.test(e.message) ? e.message : 'spike-failed';
    process.stderr.write(`${JSON.stringify({ error: code })}\n`);
    process.exitCode = 1;
  });
}
