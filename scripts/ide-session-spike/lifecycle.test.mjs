import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, writeFile, appendFile, rm } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
import path from 'node:path';
import os from 'node:os';

async function until(predicate) {
  const deadline = Date.now() + 8000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error('test-timeout');
    await delay(20);
  }
}

test('CLI watcher attaches, relays only appended metadata and detaches on SIGTERM', { timeout: 12000 }, async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'ao-ide-lifecycle-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const file = path.join(dir, 'session.jsonl');
  const meta = { type: 'session_meta', payload: { id: 'native', cwd: dir, source: 'vscode', originator: 'codex_vscode', thread_source: 'user' } };
  const event = (type, turn_id) => ({ type: 'event_msg', payload: { type, turn_id, last_agent_message: 'PRIVATE' } });
  const line = r => `${JSON.stringify(r)}\n`;
  await writeFile(file, line(meta) + line(event('task_complete', 'historical')));
  const requests = [];
  const server = createServer(async (req, res) => {
    const chunks = []; for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString());
    requests.push({ url: req.url, body });
    assert.equal(req.headers['x-agent-office-token'], 'test-secret');
    res.setHeader('content-type', 'application/json');
    const data = req.url.endsWith('/attach') ? { sessionId: 'observed' }
      : req.url.endsWith('/detach') ? { detached: true } : { accepted: true };
    res.end(JSON.stringify({ ok: true, data }));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => { server.closeAllConnections(); server.close(); });
  await writeFile(path.join(dir, 'control-port'), String(server.address().port));
  await writeFile(path.join(dir, 'control-token'), 'test-secret');
  const child = spawn(process.execPath, ['scripts/ide-session-spike.mjs', 'watch', '--provider', 'codex', '--file', file, '--agent', 'character', '--app-data', dir], { stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(() => { if (child.exitCode === null) child.kill('SIGKILL'); });
  let output = ''; let errors = '';
  child.stdout.on('data', d => { output += d; }); child.stderr.on('data', d => { errors += d; });
  const exited = once(child, 'exit');
  await until(() => output.includes('"status":"connected"'));
  assert.equal(requests.filter(r => r.body.kind === 'stop').length, 0);
  await appendFile(file, line(event('task_started', 'new')) + line(event('task_complete', 'new')) + line(event('task_complete', 'new')));
  await until(() => requests.some(r => r.body.kind === 'stop'));
  child.kill('SIGTERM');
  const [code, signal] = await exited;
  assert.equal(code, 0); assert.equal(signal, null); assert.equal(errors, '');
  assert.equal(requests[0].url, '/v1/observed/attach');
  assert.equal(requests.at(-1).url, '/v1/observed/detach');
  assert.deepEqual(requests.filter(r => ['prompt', 'stop'].includes(r.body.kind)).map(r => r.body.kind), ['prompt', 'stop']);
  assert.ok(requests.some(r => r.body.kind === 'heartbeat'));
  assert.equal(JSON.stringify(requests).includes('PRIVATE'), false);
  assert.equal(output.includes('PRIVATE'), false);
  assert.equal(output.includes('test-secret'), false);
});
