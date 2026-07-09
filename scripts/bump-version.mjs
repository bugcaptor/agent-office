#!/usr/bin/env node
// 버전 올리기 스크립트
//
// 사용법:
//   node scripts/bump-version.mjs [major|minor|patch]   (기본값: patch)
//   node scripts/bump-version.mjs <x.y.z>                (특정 버전 지정)
//
// 자리수를 올리면 아래 자리수는 0으로 리셋된다.
//   1.4.2 + major -> 2.0.0
//   1.4.2 + minor -> 1.5.0
//   1.4.2 + patch -> 1.4.3
//
// 버전이 들어있는 5개 파일을 모두 갱신한다:
//   package.json, package-lock.json,
//   src-tauri/Cargo.toml, src-tauri/Cargo.lock, src-tauri/tauri.conf.json

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

function computeNext(current, kind) {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(current);
  if (!m) {
    throw new Error(`현재 버전 형식을 해석할 수 없습니다: "${current}"`);
  }
  let [major, minor, patch] = m.slice(1).map(Number);
  switch (kind) {
    case "major":
      major += 1;
      minor = 0;
      patch = 0;
      break;
    case "minor":
      minor += 1;
      patch = 0;
      break;
    case "patch":
      patch += 1;
      break;
    default:
      throw new Error(`알 수 없는 인자: "${kind}" (major|minor|patch 또는 x.y.z)`);
  }
  return `${major}.${minor}.${patch}`;
}

// JSON 파일: 지정한 키들의 값을 새 버전으로 바꾼다. (들여쓰기 2칸 유지)
function updateJson(relPath, apply) {
  const path = join(ROOT, relPath);
  const json = JSON.parse(readFileSync(path, "utf8"));
  apply(json);
  writeFileSync(path, JSON.stringify(json, null, 2) + "\n");
}

// TOML/Lock 파일: agent-office 패키지의 version 라인만 바꾼다.
function updateCargo(relPath, nextVersion) {
  const path = join(ROOT, relPath);
  const text = readFileSync(path, "utf8");
  // name = "agent-office" 바로 다음 version = "..." 라인만 교체
  const re = /(name = "agent-office"\r?\nversion = )"[^"]*"/;
  if (!re.test(text)) {
    throw new Error(`${relPath} 에서 agent-office 버전 라인을 찾지 못했습니다.`);
  }
  writeFileSync(path, text.replace(re, `$1"${nextVersion}"`));
}

function main() {
  const arg = process.argv[2] ?? "patch";

  const pkgPath = join(ROOT, "package.json");
  const current = JSON.parse(readFileSync(pkgPath, "utf8")).version;

  const explicit = /^\d+\.\d+\.\d+$/.test(arg);
  const next = explicit ? arg : computeNext(current, arg);

  updateJson("package.json", (j) => {
    j.version = next;
  });
  updateJson("package-lock.json", (j) => {
    j.version = next;
    if (j.packages && j.packages[""]) j.packages[""].version = next;
  });
  updateJson("src-tauri/tauri.conf.json", (j) => {
    j.version = next;
  });
  updateCargo("src-tauri/Cargo.toml", next);
  updateCargo("src-tauri/Cargo.lock", next);

  console.log(`버전을 올렸습니다: ${current} -> ${next}`);
  console.log("갱신된 파일: package.json, package-lock.json, src-tauri/Cargo.toml, src-tauri/Cargo.lock, src-tauri/tauri.conf.json");
}

main();
