import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, appendFile, rm, rename, mkdir } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { metadata, inspectTranscript, TranscriptTail, EventFilter, discover } from './transcripts.mjs';

const codexMeta = { type: 'session_meta', payload: { id: 'native-1', cwd: '/repo', source: 'vscode', originator: 'codex_vscode', thread_source: 'user', instructions: 'PRIVATE' } };
const claudePrompt = { type: 'user', sessionId: 'native-2', cwd: '/repo', entrypoint: 'claude-vscode', promptId: 'prompt-1', message: { role: 'user', content: [{ type: 'text', text: 'PRIVATE' }] } };
const line = r => `${JSON.stringify(r)}\n`;
async function fixture(t) {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'ao-ide-spike-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return dir;
}

test('metadata identifies both installed extension formats without returning conversation contents', () => {
  assert.equal(metadata('codex', [codexMeta]).source, 'vscode');
  assert.equal(metadata('claude', [claudePrompt]).source, 'vscode');
  assert.equal(JSON.stringify(metadata('codex', [codexMeta])).includes('PRIVATE'), false);
  assert.equal(JSON.stringify(metadata('claude', [claudePrompt])).includes('PRIVATE'), false);
  assert.equal(metadata('codex', [{ ...codexMeta, payload: { ...codexMeta.payload, source: { subagent: {} }, thread_source: 'subagent' } }]).source, 'subagent');
  assert.equal(metadata('claude', [{ ...claudePrompt, isSidechain: true }]).source, 'subagent');
  assert.equal(metadata('claude', [{ ...claudePrompt, entrypoint: 'cli' }]).source, 'unknown');
  assert.equal(metadata('codex', [{ ...codexMeta, payload: { ...codexMeta.payload, thread_source: undefined } }]).source, 'unknown');
});

test('Codex emits explicit start/tool/complete, deduplicates per turn, ignores narration and arbitrary text', () => {
  const f = new EventFilter('codex', 'native-1');
  const start = { type: 'event_msg', payload: { type: 'task_started', turn_id: 't1' } };
  const stop = { type: 'event_msg', payload: { type: 'task_complete', turn_id: 't1', last_agent_message: 'PRIVATE' } };
  assert.deepEqual(f.take(start), { kind: 'prompt' });
  assert.equal(f.take(start), null);
  assert.deepEqual(f.take({ type: 'response_item', payload: { type: 'function_call', call_id: 'c1', arguments: 'PRIVATE' } }), { kind: 'tool' });
  assert.equal(f.take({ type: 'response_item', payload: { type: 'message', phase: 'final_answer' } }), null);
  assert.deepEqual(f.take(stop), { kind: 'stop' });
  assert.equal(f.take(stop), null);
  assert.equal(f.take({ type: 'event_msg', payload: { type: 'turn_aborted' } }), null);
});

test('Claude ignores tool results, sidechains, meta and other session IDs; repeated streamed end_turn is one completion', () => {
  const f = new EventFilter('claude', 'native-2');
  assert.deepEqual(f.take(claudePrompt), { kind: 'prompt' });
  assert.equal(f.take(claudePrompt), null);
  assert.equal(f.take({ ...claudePrompt, promptId: 'p2', message: { role: 'user', content: [{ type: 'tool_result' }] } }), null);
  assert.equal(f.take({ ...claudePrompt, promptId: 'p3', isMeta: true }), null);
  assert.equal(f.take({ ...claudePrompt, promptId: 'p4', isSidechain: true }), null);
  assert.equal(f.take({ ...claudePrompt, promptId: 'p5', sessionId: 'someone-else' }), null);
  const assistant = { type: 'assistant', sessionId: 'native-2', message: { id: 'm1', content: [{ type: 'tool_use', input: 'PRIVATE' }], stop_reason: 'tool_use' } };
  assert.deepEqual(f.take(assistant), { kind: 'tool' });
  assert.equal(f.take(assistant), null);
  const complete = { ...assistant, message: { id: 'm2', content: [{ type: 'text', text: 'PRIVATE' }], stop_reason: 'end_turn' } };
  assert.deepEqual(f.take(complete), { kind: 'stop' });
  assert.equal(f.take({ ...complete, uuid: 'another-stream-chunk' }), null);
  assert.equal(f.take({ type: 'system', sessionId: 'native-2', subtype: 'stop_hook_summary' }), null);
});

test('tail starts at EOF, buffers partial UTF-8 records, skips malformed lines', async t => {
  const dir = await fixture(t); const file = path.join(dir, 'session.jsonl');
  await writeFile(file, line(codexMeta) + line({ old: true }));
  const tail = await TranscriptTail.fromNow(file); t.after(() => tail.close());
  assert.deepEqual(await tail.read(), []);
  const bytes = Buffer.from(line({ value: '용' }));
  await appendFile(file, bytes.subarray(0, 12));
  assert.deepEqual(await tail.read(), []);
  await appendFile(file, bytes.subarray(12));
  await appendFile(file, 'invalid\n');
  assert.deepEqual(await tail.read(), [{ value: '용' }]);
  assert.deepEqual(await tail.read(), []);
});

test('tail never replays a record that had already started before connection', async t => {
  const dir = await fixture(t); const file = path.join(dir, 'session.jsonl');
  await writeFile(file, '{"old":');
  const tail = await TranscriptTail.fromNow(file); t.after(() => tail.close());
  await appendFile(file, 'true}\n' + line({ new: true }));
  assert.deepEqual(await tail.read(), [{ new: true }]);
});

test('tail fails closed on truncation and replacement instead of replaying history', async t => {
  const dir = await fixture(t); const file = path.join(dir, 'session.jsonl');
  await writeFile(file, line(codexMeta));
  const tail = await TranscriptTail.fromNow(file); t.after(() => tail.close());
  await writeFile(file, '');
  await assert.rejects(tail.read(), /transcript-replaced/);
  await writeFile(path.join(dir, 'replacement'), line(codexMeta));
  await rename(path.join(dir, 'replacement'), file);
  await assert.rejects(tail.read(), /transcript-replaced/);
});

test('replacement between metadata inspection and tail connection cannot attribute B events to A', async t => {
  const dir = await fixture(t); const file = path.join(dir, 'session.jsonl');
  await writeFile(file, line(codexMeta));
  const checked = await inspectTranscript(file, 'codex');
  assert.equal(JSON.stringify(checked).includes('baseline'), false);
  await writeFile(path.join(dir, 'other.jsonl'), line({ ...codexMeta, payload: { ...codexMeta.payload, id: 'someone-else' } }));
  await rename(path.join(dir, 'other.jsonl'), file);
  await assert.rejects(TranscriptTail.fromNow(file, checked.baseline), /transcript-replaced/);
});

test('oversized lines are dropped and subsequent complete records remain readable', async t => {
  const dir = await fixture(t); const file = path.join(dir, 'session.jsonl');
  await writeFile(file, '');
  const tail = await TranscriptTail.fromNow(file); t.after(() => tail.close());
  await appendFile(file, line({ value: 'x'.repeat(2 * 1024 * 1024 + 1) }) + line({ next: true }));
  assert.deepEqual(await tail.read(), [{ next: true }]);
});

test('discovery honors configured roots, filters cwd, excludes nested sidechains and reports no invented running status', async t => {
  const home = await fixture(t);
  const codexHome = path.join(home, 'codex'); const claudeHome = path.join(home, 'claude');
  const cd = path.join(codexHome, 'sessions/2026/09/12'); const cl = path.join(claudeHome, 'projects/project');
  await mkdir(cd, { recursive: true }); await mkdir(cl, { recursive: true });
  await writeFile(path.join(cd, 'rollout-1.jsonl'), line(codexMeta));
  await writeFile(path.join(cl, 'native-2.jsonl'), line(claudePrompt));
  await mkdir(path.join(cl, 'subagents')); await writeFile(path.join(cl, 'subagents/side.jsonl'), line(claudePrompt));
  const env = { CODEX_HOME: codexHome, CLAUDE_CONFIG_DIR: claudeHome };
  const result = await discover({ home, env, cwd: '/repo' });
  assert.equal(result.length, 2);
  assert.ok(result.every(r => r.source === 'vscode' && r.state === 'unknown'));
  assert.equal(JSON.stringify(result).includes('PRIVATE'), false);
  assert.deepEqual(await discover({ home, env, cwd: '/other' }), []);
  const inspected = await inspectTranscript(path.join(cl, 'native-2.jsonl'), 'claude');
  assert.equal(inspected.sourceSessionId, 'native-2');
});
