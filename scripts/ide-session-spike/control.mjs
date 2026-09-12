import { readFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';

export function appDataDir(explicit, env = process.env, platform = process.platform, home = os.homedir()) {
  if (explicit || env.AGENT_OFFICE_APP_DATA) return path.resolve(explicit || env.AGENT_OFFICE_APP_DATA);
  if (platform === 'darwin') return path.join(home, 'Library', 'Application Support', 'com.bugcaptor.agent-office');
  if (platform === 'win32') {
    if (!env.APPDATA) throw new Error('no-app-data');
    return path.join(env.APPDATA, 'com.bugcaptor.agent-office');
  }
  return path.join(env.XDG_DATA_HOME || path.join(home, '.local/share'), 'com.bugcaptor.agent-office');
}

export class ControlClient {
  constructor(directory, fetchFn = fetch) { this.directory = directory; this.fetchFn = fetchFn; }
  async request(command, body = {}) {
    let portText, token;
    try { portText = (await readFile(path.join(this.directory, 'control-port'), 'utf8')).trim(); }
    catch { throw new Error('control-not-running'); }
    const port = Number(portText);
    if (!/^\d+$/.test(portText) || !Number.isInteger(port) || port < 1 || port > 65535) throw new Error('invalid-control-port');
    try { token = (await readFile(path.join(this.directory, 'control-token'), 'utf8')).trim(); }
    catch { throw new Error('control-not-approved'); }
    if (!token) throw new Error('control-not-approved');
    let response;
    try {
      response = await this.fetchFn(`http://127.0.0.1:${port}/v1/${command}`, {
        method: 'POST', headers: { 'content-type': 'application/json', 'x-agent-office-token': token },
        body: JSON.stringify(body), signal: AbortSignal.timeout(3000), redirect: 'error',
      });
    } catch { throw new Error('control-unreachable'); }
    if (response.status === 401) throw new Error('control-not-approved');
    if (response.status === 404) throw new Error('observed-api-unavailable');
    let envelope;
    try { envelope = await response.json(); } catch { throw new Error('invalid-control-response'); }
    if (!response.ok || envelope?.ok !== true) {
      // Do not reflect arbitrary server text or transport exceptions (may contain secrets).
      const code = typeof envelope?.error === 'string' ? envelope.error.split(':')[0] : '';
      throw new Error(/^observed-[a-z-]+$/.test(code) ? code : 'control-request-failed');
    }
    return envelope.data;
  }
}

// Sequence retries are safe: the server ignores repeated sequence numbers.
export async function sendEvent(client, connection, sequence, kind) {
  const body = { ...connection, sequence, kind };
  for (let attempt = 0; ; attempt++) {
    try { return await client.request('observed/event', body); }
    catch (e) {
      if (e.message !== 'control-unreachable' || attempt === 1) throw e;
    }
  }
}
