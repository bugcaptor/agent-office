// vscode-ext/src/logFiles.ts
//
// 세션 로그 파일 열람. 정본은 src-tauri/src/session_log/store.rs 다.
//
//   <app_data>/session-logs/v1/<agentId>/<YYYYMMDD-HHMMSS>-<sid8>.log
//   <app_data>/session-logs/v1/study/…          ← 학습자료, 캐릭터가 아니다
//
// - stamp는 로컬 시간, sid8은 sessionId의 영숫자 앞 8자(없으면 "nosessid").
// - 파일명 역순 정렬 = 최신순(파일명이 시작 시각으로 시작하므로).
// - `.log`만 로그로 취급한다.
//
// vscode 모듈에 의존하지 않는다(fs만 쓰고 tmpdir로 테스트한다).

import * as fs from "node:fs/promises";
import * as path from "node:path";

/** 학습자료 디렉터리 이름 — agentId로 오해하면 안 된다. */
export const STUDY_DIR = "study";

/** 파일 앞부분에서 헤더를 찾는 크기(store.rs HEADER_PROBE_BYTES와 동일). */
const HEADER_PROBE_BYTES = 512;

/** `<app_data>/session-logs/v1`. */
export function sessionLogRoot(appDataDir: string): string {
  return path.join(appDataDir, "session-logs", "v1");
}

export interface ParsedLogName {
  /** `YYYYMMDD-HHMMSS` 원문. 파싱 실패 시 빈 문자열. */
  stamp: string;
  /** sessionId 앞 8자. 파싱 실패 시 빈 문자열. */
  sid8: string;
  /** stamp를 로컬 시간으로 해석한 epoch ms. 파싱 실패 시 undefined. */
  startedAt?: number;
  /** 목록 표시용 "MM/DD HH:MM". 파싱 실패 시 파일명 그대로. */
  label: string;
}

const NAME_RE = /^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})-([0-9A-Za-z]+)\.log$/;

/**
 * `<stamp>-<sid8>.log` 파싱. 규약을 벗어난 파일명도 목록에서 빠지지 않게
 * 실패를 던지지 않고 label만 파일명으로 되돌린다.
 */
export function parseLogFileName(fileName: string): ParsedLogName {
  const m = NAME_RE.exec(fileName);
  if (!m) return { stamp: "", sid8: "", label: fileName };
  const [, y, mo, d, h, mi, s, sid8] = m;
  // stamp는 로컬 시간이므로 로컬 컴포넌트로 만든다(store.rs format_stamp).
  const date = new Date(
    Number(y),
    Number(mo) - 1,
    Number(d),
    Number(h),
    Number(mi),
    Number(s),
  );
  const startedAt = Number.isNaN(date.getTime()) ? undefined : date.getTime();
  return {
    stamp: `${y}${mo}${d}-${h}${mi}${s}`,
    sid8,
    startedAt,
    label: `${mo}/${d} ${h}:${mi}`,
  };
}

export interface LogFileInfo extends ParsedLogName {
  agentId: string;
  fileName: string;
  path: string;
  bytes: number;
  modifiedAt: number;
}

/** 캐릭터 하나의 로그 디렉터리. */
export function agentLogDir(root: string, agentId: string): string {
  return path.join(root, agentId);
}

/**
 * 한 캐릭터의 로그 파일 목록(최신순). 디렉터리가 없거나 못 읽으면 빈 배열 —
 * 열람 경로가 에러로 막히지 않게 한다(store.rs list_logs와 같은 태도).
 */
export async function listLogFiles(root: string, agentId: string): Promise<LogFileInfo[]> {
  if (!isValidAgentId(agentId)) return [];
  const dir = agentLogDir(root, agentId);
  let names: string[];
  try {
    names = await fs.readdir(dir);
  } catch {
    return [];
  }
  const logs = names.filter((n) => n.endsWith(".log"));
  // 최신순: 파일명이 시작 시각으로 시작하므로 이름 역순이 곧 최신순이다.
  logs.sort((a, b) => (a < b ? 1 : a > b ? -1 : 0));

  const out: LogFileInfo[] = [];
  for (const fileName of logs) {
    const full = path.join(dir, fileName);
    let bytes = 0;
    let modifiedAt = 0;
    try {
      const st = await fs.stat(full);
      if (!st.isFile()) continue;
      bytes = st.size;
      modifiedAt = st.mtimeMs;
    } catch {
      continue;
    }
    out.push({ agentId, fileName, path: full, bytes, modifiedAt, ...parseLogFileName(fileName) });
  }
  return out;
}

/** agentId를 경로 요소로 쓰기 전 검증(store.rs valid_agent_id와 동일 규칙). */
export function isValidAgentId(agentId: string): boolean {
  return !(
    agentId.length === 0 ||
    agentId.includes("/") ||
    agentId.includes("\\") ||
    agentId.includes("..") ||
    agentId === STUDY_DIR
  );
}

/** 로그 루트 아래 캐릭터 디렉터리 이름들(study 제외). 앱 미연결 시 폴백 목록용. */
export async function listAgentDirs(root: string): Promise<string[]> {
  try {
    const entries = await fs.readdir(root, { withFileTypes: true });
    return entries
      .filter((e) => e.isDirectory() && isValidAgentId(e.name))
      .map((e) => e.name)
      .sort();
  } catch {
    return [];
  }
}

/** 로그 루트가 있는지(없으면 sessionLogEnabled OFF이거나 앱을 쓴 적 없음). */
export async function logRootExists(root: string): Promise<boolean> {
  try {
    return (await fs.stat(root)).isDirectory();
  } catch {
    return false;
  }
}

export interface LogHeader {
  agentId?: string;
  sessionId?: string;
  cwd?: string;
  started?: string;
}

/**
 * 파일 앞 512바이트만 읽어 헤더를 파싱한다(목록이 파일 전체를 읽지 않게).
 * 헤더가 없거나 깨졌으면 빈 값 — 툴팁 보강용이라 없어도 무해하다.
 */
export async function readLogHeader(filePath: string): Promise<LogHeader> {
  const header: LogHeader = {};
  let text: string;
  try {
    const fd = await fs.open(filePath, "r");
    try {
      const buf = Buffer.alloc(HEADER_PROBE_BYTES);
      const { bytesRead } = await fd.read(buf, 0, HEADER_PROBE_BYTES, 0);
      text = buf.subarray(0, bytesRead).toString("utf8");
    } finally {
      await fd.close();
    }
  } catch {
    return header;
  }
  for (const line of text.split("\n")) {
    if (!line.startsWith("# ")) {
      if (line.startsWith("#")) continue;
      break; // 헤더 블록 끝
    }
    const rest = line.slice(2);
    for (const [key, field] of [
      ["agentId: ", "agentId"],
      ["sessionId: ", "sessionId"],
      ["cwd: ", "cwd"],
      ["started: ", "started"],
    ] as const) {
      if (rest.startsWith(key)) {
        header[field] = rest.slice(key.length).trim();
      }
    }
  }
  return header;
}

/** 바이트 수를 사람이 읽는 크기로. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${Math.round(kb)} KB`;
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
}

/** 잘렸을 때 문서 첫 줄에 끼우는 안내(개행 없음). */
export function truncationNotice(omittedBytes: number): string {
  return `⋯ (앞 ${formatBytes(omittedBytes)} 생략 — "전체 불러오기"로 처음부터)`;
}

export interface TailSlice {
  /** 문서에 넣을 본문(잘렸으면 안내 첫 줄 포함). */
  text: string;
  truncated: boolean;
  /** 잘라 버린 앞부분 바이트 수. */
  omittedBytes: number;
  totalBytes: number;
}

/**
 * 파일 끝에서 `tailKb` KB를 읽는다. 잘린 경우 첫 줄 경계에 맞춰(깨진 줄과
 * 쪼개진 멀티바이트 문자를 버리고) 안내 한 줄을 앞에 붙인다.
 */
export async function readTailSlice(
  filePath: string,
  tailKb: number,
  notice: (omittedBytes: number) => string = truncationNotice,
): Promise<TailSlice> {
  const limit = Math.max(1, Math.floor(tailKb)) * 1024;
  const fd = await fs.open(filePath, "r");
  try {
    const { size } = await fd.stat();
    if (size <= limit) {
      const buf = Buffer.alloc(size);
      if (size > 0) await fd.read(buf, 0, size, 0);
      return { text: buf.toString("utf8"), truncated: false, omittedBytes: 0, totalBytes: size };
    }
    const start = size - limit;
    const buf = Buffer.alloc(limit);
    const { bytesRead } = await fd.read(buf, 0, limit, start);
    const chunk = buf.subarray(0, bytesRead);
    // 첫 줄 경계로 정렬. 개행이 아예 없으면(초장문 한 줄) 그대로 쓴다.
    const nl = chunk.indexOf(0x0a);
    const body = nl >= 0 ? chunk.subarray(nl + 1) : chunk;
    const omittedBytes = size - body.length;
    return {
      text: `${notice(omittedBytes)}\n${body.toString("utf8")}`,
      truncated: true,
      omittedBytes,
      totalBytes: size,
    };
  } finally {
    await fd.close();
  }
}

/**
 * "전체 불러오기"도 무한정 열 수는 없다 — 이보다 큰 파일을 에디터에 밀어 넣으면
 * 확장 호스트가 아니라 VSCode 창 자체가 멈춘다(V8 문자열 상한도 그 근처다).
 * 실제 세션 로그는 보통 KB~MB 단위라 이 상한에 닿지 않는다.
 */
export const FULL_MAX_BYTES = 128 * 1024 * 1024;

/** 전체 본문("전체 불러오기"). 상한을 넘으면 뒤에서 상한만큼만 준다. */
export async function readFullText(filePath: string): Promise<string> {
  const st = await fs.stat(filePath);
  if (st.size > FULL_MAX_BYTES) {
    const slice = await readTailSlice(
      filePath,
      Math.floor(FULL_MAX_BYTES / 1024),
      (omitted) =>
        `⋯ (파일이 ${formatBytes(st.size)}로 너무 커서 앞 ${formatBytes(omitted)}을 생략했습니다 — 전체는 "파일로 열기"나 외부 도구로 보세요)`,
    );
    return slice.text;
  }
  return fs.readFile(filePath, "utf8");
}
