#!/usr/bin/env node

import { execFileSync, spawnSync } from 'node:child_process';
import { lstatSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const checkout = realpathSync(root);
const crate = path.join(root, 'src-tauri');
const require = createRequire(import.meta.url);
const tauri = require.resolve('@tauri-apps/cli/tauri.js');
const args = process.argv.slice(2);
const bundlesApp = !args.some(arg => ['-h', '--help', '-V', '--version', '--no-bundle'].includes(arg));
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], {
  cwd: crate,
  encoding: 'utf8',
  stdio: ['ignore', 'pipe', 'inherit'],
}));

if (!Object.hasOwn(metadata, 'build_directory')) {
  console.error('설치용 빌드에는 Cargo 1.91 이상이 필요합니다. rustup update로 업데이트하세요.');
  process.exit(1);
}

const profiles = ['debug', 'release'];
const cacheEntries = ['.fingerprint', 'build', 'deps', 'examples', 'incremental'];
const intermediateFile = /^(?:agent-office\.d|(?:lib)?agent_office_lib\.(?:a|d|dll(?:\.a|\.exp|\.lib)?|dylib|lib|rlib|so))$/;

function removeGenerated(entry) {
  try {
    rmSync(entry, { recursive: true, force: true, maxRetries: 3 });
  } catch (error) {
    console.warn(`빌드 중간 파일 정리 실패 (${entry}): ${error.message}`);
  }
}

function cleanCargoCaches(directory, targetTriples) {
  if (!lstatSync(directory, { throwIfNoEntry: false })?.isDirectory()) return;
  const relative = path.relative(checkout, realpathSync(directory));
  // CARGO_TARGET_DIR와 build.build-dir은 다른 프로젝트와 공유될 수 있다.
  if (!relative || path.isAbsolute(relative) || relative.split(path.sep)[0] === '..') return;

  const layouts = [directory];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && targetTriples.has(entry.name)) {
      layouts.push(path.join(directory, entry.name));
    }
  }

  for (const layout of layouts) {
    for (const profile of profiles) {
      const output = path.join(layout, profile);
      if (!lstatSync(output, { throwIfNoEntry: false })?.isDirectory()) continue;
      // 컴파일러가 만든 디렉터리만 정리한다. bundle과 실행 파일은 그대로 사용한다.
      const names = new Set(readdirSync(output));
      if (!names.has('.cargo-lock') && !names.has('.cargo-build-lock')) continue;
      for (const name of cacheEntries) {
        if (names.has(name)) removeGenerated(path.join(output, name));
      }
      for (const name of names) {
        if (intermediateFile.test(name)) removeGenerated(path.join(output, name));
      }
    }
  }
}

// Cargo는 중간 산출물만 이 디렉터리에 두고, Tauri는 기존 target에 번들을 생성한다.
mkdirSync(metadata.target_directory, { recursive: true });
const buildDir = mkdtempSync(path.join(metadata.target_directory, 'bundle-build-'));
try {
  const result = spawnSync(process.execPath, [tauri, 'build', ...args], {
    cwd: root,
    env: { ...process.env, CARGO_BUILD_BUILD_DIR: buildDir },
    stdio: 'inherit',
  });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
  if (result.status === 0 && bundlesApp) {
    const targetList = spawnSync('rustc', ['--print', 'target-list'], { cwd: crate, encoding: 'utf8' });
    const targetTriples = new Set(targetList.stdout?.trim().split(/\s+/));
    const cacheRoots = new Set([
      metadata.target_directory,
      metadata.build_directory,
      path.join(metadata.target_directory, 'llvm-cov-target'),
    ]);
    for (const directory of cacheRoots) {
      try {
        cleanCargoCaches(directory, targetTriples);
      } catch (error) {
        console.warn(`빌드 중간 파일 정리 실패 (${directory}): ${error.message}`);
      }
    }
  }
} finally {
  removeGenerated(buildDir);
}
