// Integration acceptance against the built native binary. All sessions and
// credentials live under a temporary directory; no installed app is touched.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import assert from 'node:assert/strict';

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const dir = await mkdtemp(join(tmpdir(), 'ao-remote-'));
const binary = resolve(process.argv[2] ?? 'src-tauri/target/debug/agent-office');
const listener = createServer();
await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
const port = listener.address().port;
await new Promise(resolve => listener.close(resolve));
let processHandle;
let log = '';
let clients = [];
let token;
async function start() {
  log = '';
  processHandle = spawn(binary, ['serve', '--data-dir', dir, '--bind', '127.0.0.1', '--port', String(port)], { stdio: ['ignore', 'ignore', 'pipe'] });
  processHandle.stderr.on('data', chunk => { log += chunk.toString(); });
  for (let i = 0; i < 200; i++) {
    if (processHandle.exitCode !== null) throw new Error(`Server exited: ${log}`);
    if (log.includes('listening')) { token = (await readFile(join(dir, 'serve-token'), 'utf8')).trim(); return; }
    await sleep(50);
  }
  throw new Error(`Server start timed out: ${log}`);
}
async function stop() {
  clients.forEach(c => c.socket.close()); clients = [];
  const child = processHandle;
  child.kill('SIGTERM');
  await Promise.race([new Promise(resolve => child.once('exit', resolve)), sleep(10000).then(() => {if(child.exitCode===null)child.kill('SIGKILL');})]);
}
async function connect(secret = token) {
  const socket = new WebSocket(`ws://127.0.0.1:${port}/webremote/v1/ws`, [`agent-office.token.${secret}`]);
  const messages = []; const pending = new Map(); let id = 0;
  socket.addEventListener('message', event => {
    const m = JSON.parse(event.data); messages.push(m);
    if (m.type === 'rpcResult' && pending.has(m.id)) { const resolve = pending.get(m.id); pending.delete(m.id); resolve(m); }
  });
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, {once:true}); socket.addEventListener('error', () => reject(new Error('connection rejected')), {once:true});
  });
  const client = { socket, messages, send: value => socket.send(JSON.stringify(value)), async rpc(cmd, args={}) {
    const requestId = ++id;
    const response = new Promise(resolve => pending.set(requestId, resolve));
    socket.send(JSON.stringify({type:'rpc', id:requestId, cmd, args}));
    const reply = await Promise.race([response, sleep(10000).then(() => {throw new Error(`RPC timeout: ${cmd}`);})]);
    if (!reply.ok) throw new Error(`${cmd}: ${JSON.stringify(reply.error)}`);
    return reply.data;
  }};
  clients.push(client); return client;
}
async function until(predicate, description, timeout=15000) {
  const deadline = Date.now()+timeout;
  while(Date.now()<deadline) { if(predicate()) return; await sleep(20); }
  throw new Error(`Timed out: ${description}`);
}
function outputs(client) {return client.messages.filter(m=>m.type==='output');}
try {
  await start();
  assert.equal((await stat(join(dir,'serve-token'))).mode & 0o777, 0o600);
  await assert.rejects(connect('incorrect-token'));
  const a = await connect();
  const before = await a.rpc('office.snapshot');
  const profile = { id:'remote-check', name:'Remote check', role:'test', seed:'remote-check', createdAt:Date.now(), deskIndex:0, cwd:dir, archetype:'human' };
  const saved = await a.rpc('office.saveState', {state:{version:1,agents:[profile]},revision:before.revision});
  await assert.rejects(a.rpc('office.saveState', {state:{version:1,agents:[]},revision:before.revision}), /conflict/);
  const pngBase64 = Buffer.from([0x89,0x50,0x4e,0x47,0x0d,0x0a,0x1a,0x0a]).toString('base64');
  for (const kind of ['portrait', 'sprite', 'minimi']) {
    const args = {agentId:profile.id,kind};
    assert.equal(await a.rpc('office.media.load', args), null);
    await a.rpc('office.media.save', {...args,pngBase64});
    assert.equal(await a.rpc('office.media.load', args), pngBase64);
    await a.rpc('office.media.delete', args);
    assert.equal(await a.rpc('office.media.load', args), null);
  }
  await assert.rejects(a.rpc('office.media.save', {agentId:'../secret',kind:'portrait',pngBase64}), /forbidden/);
  const session = await a.rpc('session.start',{agentId:profile.id});
  assert.ok(session.sessionId);
  a.send({type:'attach',agentId:profile.id});
  await until(()=>a.messages.some(m=>m.type==='restore'),'initial restore');
  a.send({type:'input',agentId:profile.id,data:`python3 -c "import sys;sys.stdout.write('REMOTE_DATA_'*120000+'REMOTE_END\\n')"\r`});
  await until(()=>outputs(a).reduce((n,m)=>n+m.bytes,0)>1_300_000,'large real PTY output',30000);
  await until(()=>outputs(a).some(m=>m.data.includes('REMOTE_END')),'output end marker');
  await a.rpc('session.resize',{agentId:profile.id,cols:100,rows:35});
  const last = outputs(a).at(-1);
  const offset = last.offset+last.bytes;
  a.socket.close();
  const controller = await connect();
  controller.send({type:'input',agentId:profile.id,data:"printf 'OFFLINE_MARKER\\n'\r"});
  await sleep(250);
  const b = await connect();
  b.send({type:'attach',agentId:profile.id,lastOffset:offset,lastSessionId:session.sessionId});
  await until(()=>outputs(b).some(m=>m.data.includes('OFFLINE_MARKER')),'offline delta');
  assert.equal(b.messages.find(m=>m.type==='restore').snapshot,null);
  assert.equal(b.messages.find(m=>m.type==='restore').baseOffset,offset);
  const c = await connect();
  c.send({type:'attach',agentId:profile.id});
  await until(()=>outputs(c).some(m=>m.data.includes('OFFLINE_MARKER')),'full replay',30000);
  const history = outputs(c).map(m=>m.data).join('');
  assert.ok(Buffer.byteLength(history)>1_300_000);
  assert.ok(history.includes('REMOTE_DATA_REMOTE_DATA_'));
  assert.ok(c.messages.some(m=>m.type==='resized' && m.cols===100 && m.rows===35));
  let expected=0;
  for(const m of outputs(c)){assert.equal(m.offset,expected);expected+=m.bytes;}
  const stale = await connect();
  stale.send({type:'attach',agentId:profile.id,lastOffset:10,lastSessionId:'previous-session'});
  await until(()=>stale.messages.some(m=>m.type==='restore'),'stale session full restore');
  assert.equal(stale.messages.find(m=>m.type==='restore').baseOffset,0);
  assert.equal(stale.messages.find(m=>m.type==='restore').snapshot,'');
  stale.socket.close();
  const stableToken = token;
  await stop(); await start(); assert.equal(token,stableToken);
  const d = await connect(); const adopted = await d.rpc('office.snapshot');
  assert.ok(adopted.sessions.some(s=>s.sessionId===session.sessionId),'server restart adopts same process session');
  await d.rpc('session.dispose',{agentId:profile.id});
  console.log('PASS: token auth/mode, profile conflict, PTY IO, >1MiB full replay, offline delta, resize replay, server restart/token/session continuity');
} catch(error) {
  console.error(error); console.error(log); process.exitCode=1;
} finally {
  if (processHandle && processHandle.exitCode === null) {
    // A failed assertion must not leave a durable test shell behind.
    try {
      const cleanup = await connect();
      await cleanup.rpc('session.dispose', {agentId:'remote-check'});
    } catch {}
    await stop();
  }
  await rm(dir,{recursive:true,force:true});
}
