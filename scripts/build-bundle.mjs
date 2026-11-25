#!/usr/bin/env node

import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const require = createRequire(import.meta.url);
const tauri = require.resolve('@tauri-apps/cli/tauri.js');
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], {
  cwd: path.join(root, 'src-tauri'),
  encoding: 'utf8',
  stdio: ['ignore', 'pipe', 'inherit'],
}));

if (!Object.hasOwn(metadata, 'build_directory')) {
  console.error('설치용 빌드에는 Cargo 1.91 이상이 필요합니다. rustup update로 업데이트하세요.');
  process.exit(1);
}

// Cargo는 중간 산출물만 이 디렉터리에 두고, Tauri는 기존 target에 번들을 생성한다.
mkdirSync(metadata.target_directory, { recursive: true });
const buildDir = mkdtempSync(path.join(metadata.target_directory, 'bundle-build-'));
try {
  const result = spawnSync(process.execPath, [tauri, 'build', ...process.argv.slice(2)], {
    cwd: root,
    env: { ...process.env, CARGO_BUILD_BUILD_DIR: buildDir },
    stdio: 'inherit',
  });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
} finally {
  try {
    rmSync(buildDir, { recursive: true, force: true, maxRetries: 3 });
  } catch (error) {
    console.warn(`빌드 중간 파일 정리 실패 (${buildDir}): ${error.message}`);
  }
}
