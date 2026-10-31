import type { WorkAgent, WorkItem, WorkStatus } from "@shared/types";
import { WORK_STATUS_ORDER } from "./workItemStore";
import { hasWorkContent } from "./workItemDraft";

/** 내보내기 파일 계약. */
export const WORK_ITEMS_FILE_FORMAT = "agent-office-work-items";
export const WORK_ITEMS_FILE_VERSION = 1;

/** 사람이 고치기 쉽게 본문은 줄 배열, 시각은 ISO 문자열로 쓴다. 나머지는 저장 필드 그대로. */
interface ExportedWorkItem {
  id: string;
  title: string;
  status: WorkStatus;
  assigneeAgentId?: string;
  /** 내보낸 때의 담당 캐릭터 이름. 다른 컴퓨터에서 없는 캐릭터를 바꿀 때 보여주는 참고 값이다. */
  assigneeName?: string;
  agent?: WorkAgent;
  dependsOn?: string[];
  plan?: boolean;
  body: string[];
  order: number;
  sessionIds: string[];
  lastError?: string;
  createdAt: string;
  updatedAt: string;
}

/** 가져올 때 쓰는 필드. id는 새로 만들고 파일 안 선행 작업 참조를 다시 잇는 데만 쓴다. */
export type ImportedWorkItem = Pick<WorkItem, "title" | "body" | "status" | "assigneeAgentId" | "agent" | "dependsOn" | "plan" | "createdAt"> & { sourceId?: string; assigneeName?: string };
export type WorkItemsFileError = "invalid" | "format" | "version" | "empty";
export type WorkItemsFileParse = { ok: true; items: ImportedWorkItem[]; skipped: number } | { ok: false; error: WorkItemsFileError };

const CLIS: readonly WorkAgent["cli"][] = ["claude", "codex", "agy", "kilo", "pi"];

/** 내보내기 담당 선택의 키. 미배정은 "", 지금 없는 캐릭터는 모두 MISSING_ASSIGNEE 하나로 묶는다. */
export const MISSING_ASSIGNEE = "?";
export function assigneeKey(item: Pick<WorkItem, "assigneeAgentId">, known: Readonly<Record<string, unknown>>): string {
  if (!item.assigneeAgentId) return "";
  return known[item.assigneeAgentId] ? item.assigneeAgentId : MISSING_ASSIGNEE;
}

/** 고른 상태의 업무만 상태 그룹 순서·그룹 안 순서대로 쓴다. 들여쓰기는 항목별로 지우거나 고치기 쉽게 둔다.
 * `names`(캐릭터 id → 이름)에 있는 담당은 이름도 함께 쓴다. */
export function serializeWorkItems(items: WorkItem[], statuses: readonly WorkStatus[], now = Date.now(), names: Readonly<Record<string, string>> = {}): { json: string; count: number } {
  const picked = WORK_STATUS_ORDER.filter((status) => statuses.includes(status))
    .flatMap((status) => items.filter((item) => item.status === status).sort((a, b) => a.order - b.order));
  const exported: ExportedWorkItem[] = picked.map((item) => ({
    id: item.id,
    title: item.title,
    status: item.status,
    assigneeAgentId: item.assigneeAgentId,
    assigneeName: item.assigneeAgentId ? names[item.assigneeAgentId] : undefined,
    agent: item.agent,
    dependsOn: item.dependsOn,
    plan: item.plan || undefined,
    body: item.body.split(/\r?\n/u),
    order: item.order,
    sessionIds: item.sessionIds,
    lastError: item.lastError,
    createdAt: new Date(item.createdAt).toISOString(),
    updatedAt: new Date(item.updatedAt).toISOString(),
  }));
  const file = { format: WORK_ITEMS_FILE_FORMAT, version: WORK_ITEMS_FILE_VERSION, exportedAt: new Date(now).toISOString(), items: exported };
  return { json: `${JSON.stringify(file, null, 2)}\n`, count: exported.length };
}

export function workItemsFileName(now = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `agent-office-work-items-${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}.json`;
}

const text = (value: unknown) => typeof value === "string" ? value : undefined;

function parseTime(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value !== "string") return undefined;
  const at = Date.parse(value);
  return Number.isNaN(at) ? undefined : at;
}

function parseAgent(value: unknown): WorkAgent | undefined {
  if (!value || typeof value !== "object") return undefined;
  const { cli, model, effort } = value as Record<string, unknown>;
  if (!CLIS.includes(cli as WorkAgent["cli"])) return undefined;
  const agent: WorkAgent = { cli: cli as WorkAgent["cli"] };
  if (text(model)?.trim()) agent.model = (model as string).trim();
  if (text(effort)?.trim()) agent.effort = (effort as string).trim();
  return agent;
}

function parseItem(raw: unknown): ImportedWorkItem | undefined {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return undefined;
  const value = raw as Record<string, unknown>;
  const body = Array.isArray(value.body) ? value.body.filter((line) => typeof line === "string").join("\n") : text(value.body) ?? "";
  const title = (text(value.title) ?? "").trim();
  if (!hasWorkContent({ title, body })) return undefined;
  const status = WORK_STATUS_ORDER.includes(value.status as WorkStatus) ? value.status as WorkStatus : "todo";
  const dependsOn = Array.isArray(value.dependsOn) ? value.dependsOn.filter((id): id is string => typeof id === "string" && !!id) : undefined;
  return {
    sourceId: text(value.id) || undefined,
    title,
    body,
    status,
    assigneeAgentId: text(value.assigneeAgentId)?.trim() || undefined,
    assigneeName: text(value.assigneeName)?.trim() || undefined,
    agent: parseAgent(value.agent),
    dependsOn: dependsOn?.length ? dependsOn : undefined,
    plan: value.plan === true || undefined,
    createdAt: parseTime(value.createdAt) ?? Date.now(),
  };
}

/** 사람이 고친 파일도 받는다. 내용 없는·모양이 틀린 항목은 건너뛰고 수를 센다. 앱 데이터의 work-items.json 배열도 받는다. */
export function parseWorkItemsFile(source: string): WorkItemsFileParse {
  let root: unknown;
  try { root = JSON.parse(source.replace(/^﻿/u, "")); } catch { return { ok: false, error: "invalid" }; }
  let entries: unknown;
  if (Array.isArray(root)) entries = root;
  else if (root && typeof root === "object") {
    const file = root as Record<string, unknown>;
    if (file.format !== undefined && file.format !== WORK_ITEMS_FILE_FORMAT) return { ok: false, error: "format" };
    if (typeof file.version === "number" && file.version > WORK_ITEMS_FILE_VERSION) return { ok: false, error: "version" };
    entries = file.items;
  }
  if (!Array.isArray(entries)) return { ok: false, error: "format" };
  const items = entries.flatMap((entry) => parseItem(entry) ?? []);
  if (!items.length) return { ok: false, error: "empty" };
  return { ok: true, items, skipped: entries.length - items.length };
}

/** 가져올 업무의 담당 중 지금 없는 캐릭터. 파일 순서대로, 이름은 처음 보이는 값을 쓴다. */
export interface MissingAssignee { id: string; name?: string; count: number }
export function missingAssignees(items: readonly ImportedWorkItem[], known: Readonly<Record<string, unknown>>): MissingAssignee[] {
  const missing = new Map<string, MissingAssignee>();
  for (const { assigneeAgentId: id, assigneeName: name } of items) {
    if (!id || known[id]) continue;
    const entry = missing.get(id) ?? { id, count: 0 };
    entry.name ??= name;
    entry.count += 1;
    missing.set(id, entry);
  }
  return [...missing.values()];
}

/** 없는 캐릭터의 기본 대체: 이름이 같은 캐릭터가 하나뿐이면 그 캐릭터, 아니면 미배정(""). */
export function guessAssignee(missing: MissingAssignee, agents: readonly { id: string; name: string }[]): string {
  const name = missing.name?.trim();
  const same = name ? agents.filter((agent) => agent.name.trim() === name) : [];
  return same.length === 1 ? same[0].id : "";
}

/** 담당 id를 한꺼번에 바꾼다. 대체가 ""면 미배정으로 둔다. 표에 없는 담당은 그대로 둔다. */
export function remapAssignees(items: readonly ImportedWorkItem[], replace: Readonly<Record<string, string>>): ImportedWorkItem[] {
  return items.map((item) => item.assigneeAgentId && item.assigneeAgentId in replace
    ? { ...item, assigneeAgentId: replace[item.assigneeAgentId] || undefined }
    : item);
}
