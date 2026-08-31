#!/usr/bin/env node
//
// 빌드 전에 Rust 빌드 캐시(src-tauri/target)의 오래된 잔존물을 치운다.
//
// cargo 는 한 번 만든 산출물을 스스로 지우지 않는다. 브랜치를 옮기고 의존성을
// 올릴 때마다 debug/deps 와 debug/incremental 에 옛 해시의 파일이 계속 쌓여서,
// 몇 달 방치하면 수십 GB 가 된다. 이 스크립트는 빌드 직전에 두 가지를 한다.
//
//   1. debug/incremental 의 오래된 컴파일 세션 디렉터리 삭제 (순수 캐시라 안전)
//   2. cargo-sweep 이 깔려 있으면 오래된 산출물 정리
//
// 지금 브랜치를 빌드하는 데 쓰는 최신 캐시는 남으므로 재빌드는 여전히 증분이다.
//
// 환경변수:
//   AGENT_OFFICE_SKIP_SWEEP=1   이번 빌드에서는 청소를 건너뛴다
//   AGENT_OFFICE_SWEEP_DAYS=N   며칠 지난 것을 잔존물로 볼지 (기본 3)
//
// npm 의 pretauri 훅에 걸려 있어서 `npm run tauri dev` / `npm run tauri build`
// 어느 쪽으로 빌드해도 먼저 돈다.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CRATE = path.join(ROOT, 'src-tauri');
const TARGET = process.env.CARGO_TARGET_DIR
  ? path.resolve(process.env.CARGO_TARGET_DIR)
  : path.join(CRATE, 'target');

// 이 프로젝트는 debug 빌드 한 번에 수 GB 를 뱉어서 하루만 굴려도 14GB 가 쌓인다.
// 7일씩 들고 있으면 target 이 40GB 를 넘으므로 3일로 짧게 잡았다.
const DAYS = Number(process.env.AGENT_OFFICE_SWEEP_DAYS ?? 3);

if (process.env.AGENT_OFFICE_SKIP_SWEEP === '1') process.exit(0);
if (!fs.existsSync(TARGET)) process.exit(0);
if (!Number.isFinite(DAYS) || DAYS < 1) {
  console.error(`sweep: AGENT_OFFICE_SWEEP_DAYS 값이 이상하다: ${process.env.AGENT_OFFICE_SWEEP_DAYS}`);
  process.exit(0);
}

const cutoff = Date.now() - DAYS * 24 * 60 * 60 * 1000;

// --- 1. 오래된 증분 컴파일 세션 ---------------------------------------------
//
// incremental/ 아래는 `s-<시각>-<해시>` 꼴의 세션 디렉터리 모음이다. 지금 빌드가
// 쓰는 것만 mtime 이 갱신되고 나머지는 그대로 남아 계속 불어난다.

let freed = 0;

for (const profile of ['debug', 'release']) {
  const dir = path.join(TARGET, profile, 'incremental');
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    continue; // 프로파일을 아직 빌드한 적이 없다
  }
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const full = path.join(dir, entry.name);
    let mtime;
    try {
      mtime = fs.statSync(full).mtimeMs;
    } catch {
      continue;
    }
    if (mtime >= cutoff) continue;
    try {
      fs.rmSync(full, { recursive: true, force: true });
      freed += 1;
    } catch (err) {
      console.error(`sweep: ${full} 을(를) 못 지웠다 -- ${err.message}`);
    }
  }
}

if (freed > 0) {
  console.log(`==> sweep: ${DAYS}일 넘은 증분 컴파일 세션 ${freed}개 삭제`);
}

// --- 2. cargo-sweep ----------------------------------------------------------
//
// deps/ 의 옛 산출물은 파일 하나하나가 어느 빌드 것인지 cargo 만 알아서, 직접
// 지우면 안 된다. cargo-sweep 이 그 판단을 대신 해 준다. 없으면 안내만 하고
// 빌드는 그대로 진행한다 -- 청소 때문에 빌드가 막히면 안 된다.

const probe = spawnSync('cargo', ['sweep', '--version'], { cwd: CRATE, stdio: 'ignore' });

if (probe.error || probe.status !== 0) {
  console.log('==> sweep: cargo-sweep 이 없다. deps/ 의 오래된 빌드 산출물이 계속 쌓인다.');
  console.log('    설치: cargo install cargo-sweep');
} else {
  const sweep = spawnSync('cargo', ['sweep', '--time', String(DAYS)], {
    cwd: CRATE,
    stdio: 'inherit',
  });
  if (sweep.error || sweep.status !== 0) {
    console.error('==> sweep: cargo-sweep 이 실패했다. 빌드는 그대로 진행한다.');
  }
}
