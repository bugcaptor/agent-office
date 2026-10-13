import type { WorkAgent, WorkItem, WorkStatus } from "@shared/types";
import { WORK_STATUS_ORDER } from "./workItemStore";
import { hasWorkContent } from "./workItemDraft";

/** 내보내기 파일 계약. docs/work-panels-design.md "내보내기와 가져오기". */
export const WORK_ITEMS_FILE_FORMAT = "agent-office-work-items";
export const WORK_ITEMS_FILE_VERSION = 1;

/** 사람이 고치기 쉽게 본문은 줄 배열, 시각은 ISO 문자열로 쓴다. 나머지는 저장 필드 그대로. */
interface ExportedWorkItem {
  id: string;
  title: string;
  status: WorkStatus;
  assigneeAgentId?: string;
  agent?: WorkAgent;
  dependsOn?: string[];
  body: string[];
  order: number;
  sessionIds: string[];
  lastError?: string;
  createdAt: string;
  updatedAt: string;
}

/** 가져올 때 쓰는 필드. id는 새로 만들고 파일 안 선행 작업 참조를 다시 잇는 데만 쓴다. */
export type ImportedWorkItem = Pick<WorkItem, "title" | "body" | "status" | "assigneeAgentId" | "agent" | "dependsOn" | "createdAt"> & { sourceId?: string };
export type WorkItemsFileError = "invalid" | "format" | "version" | "empty";
export type WorkItemsFileParse = { ok: true; items: ImportedWorkItem[]; skipped: number } | { ok: false; error: WorkItemsFileError };

const CLIS: readonly WorkAgent["cli"][] = ["claude", "codex", "agy", "kilo", "pi"];

/** 고른 상태의 업무만 상태 그룹 순서·그룹 안 순서대로 쓴다. 들여쓰기는 항목별로 지우거나 고치기 쉽게 둔다. */
export function serializeWorkItems(items: WorkItem[], statuses: readonly WorkStatus[], now = Date.now()): { json: string; count: number } {
  const picked = WORK_STATUS_ORDER.filter((status) => statuses.includes(status))
    .flatMap((status) => items.filter((item) => item.status === status).sort((a, b) => a.order - b.order));
  const exported: ExportedWorkItem[] = picked.map((item) => ({
    id: item.id,
    title: item.title,
    status: item.status,
    assigneeAgentId: item.assigneeAgentId,
    agent: item.agent,
    dependsOn: item.dependsOn,
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
    agent: parseAgent(value.agent),
    dependsOn: dependsOn?.length ? dependsOn : undefined,
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
