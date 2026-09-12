import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { ControlClient, appDataDir, sendEvent } from './control.mjs';

async function fixture(t) {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'ao-ide-control-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await writeFile(path.join(dir, 'control-port'), '12345');
  await writeFile(path.join(dir, 'control-token'), 'secret-fixture');
  return dir;
}

test('uses local authenticated control server and carries only normalized event metadata', async t => {
  const dir = await fixture(t);
  const requests = [];
  const client = new ControlClient(dir, async (url, init) => {
    requests.push({ url, init });
    return { ok: true, status: 200, json: async () => ({ ok: true, data: { accepted: true } }) };
  });
  const connection = { agentId: 'a', ownerId: 'o', sessionId: 's' };
  await sendEvent(client, connection, 1, 'stop');
  assert.equal(requests[0].url, 'http://127.0.0.1:12345/v1/observed/event');
  assert.equal(requests[0].init.headers['x-agent-office-token'], 'secret-fixture');
  assert.equal(requests[0].init.redirect, 'error');
  assert.deepEqual(JSON.parse(requests[0].init.body), { ...connection, sequence: 1, kind: 'stop' });
});

test('transport retry preserves sequence for server idempotency and never exposes underlying errors', async t => {
  const dir = await fixture(t); const bodies = [];
  const client = new ControlClient(dir, async (_url, init) => {
    bodies.push(init.body);
    if (bodies.length === 1) throw new Error('secret-fixture');
    return { ok: true, status: 200, json: async () => ({ ok: true, data: { accepted: false } }) };
  });
  assert.deepEqual(await sendEvent(client, { sessionId: 's' }, 4, 'stop'), { accepted: false });
  assert.equal(bodies[0], bodies[1]);
  await assert.rejects(new ControlClient(dir, async () => { throw new Error('secret-fixture'); }).request('list'), /^Error: control-unreachable$/);
});

test('missing approval, malformed ports, revoked token, old app and secret-bearing server errors fail safely', async t => {
  const dir = await fixture(t);
  const response = (status, error) => new ControlClient(dir, async () => ({ status, ok: false, json: async () => ({ ok: false, error }) }));
  await assert.rejects(response(401).request('list'), /control-not-approved/);
  await assert.rejects(response(404).request('observed/attach'), /observed-api-unavailable/);
  await assert.rejects(response(200, 'secret-fixture').request('list'), /^Error: control-request-failed$/);
  await assert.rejects(response(200, 'observed-agent-busy').request('list'), /observed-agent-busy/);
  await writeFile(path.join(dir, 'control-port'), '12345garbage');
  await assert.rejects(response(200).request('list'), /invalid-control-port/);
  await writeFile(path.join(dir, 'control-port'), '12345');
  await rm(path.join(dir, 'control-token'));
  await assert.rejects(response(200).request('list'), /control-not-approved/);
});

test('app-data resolution follows existing CLI conventions', () => {
  assert.equal(appDataDir('/explicit', { AGENT_OFFICE_APP_DATA: '/env' }), '/explicit');
  assert.equal(appDataDir(undefined, { AGENT_OFFICE_APP_DATA: '/env' }), '/env');
  assert.equal(appDataDir(undefined, {}, 'darwin', '/home'), '/home/Library/Application Support/com.bugcaptor.agent-office');
  assert.equal(appDataDir(undefined, { XDG_DATA_HOME: '/data' }, 'linux', '/home'), '/data/com.bugcaptor.agent-office');
});
