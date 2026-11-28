// src/renderer/files/filesStore.ts
//
// 머신 간 파일 보내기 상태: 설정·받은 파일 폴더(`files_status`), 보낸·받은 기록(`file-transfers` 이벤트),
// 보낼 수 있는 곳(`files_targets`, 대화상자가 열려 있는 동안 몇 초마다), 방금 받은 것 알림(`file-received`).
import { create } from "zustand";
import type { FileParty, FilesStatus, FileTarget, FileTransfer } from "@shared/types";
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import { tauriApi } from "../ipc/tauriApi";

interface FilesUiState {
  status: FilesStatus | null;
  transfers: FileTransfer[];
  targets: FileTarget[];
  /** 방금 다 받은 것(화면 알림). */
  arrivals: FileTransfer[];
}

export const useFilesStore = create<FilesUiState>(() => ({ status: null, transfers: [], targets: [], arrivals: [] }));

export function setFilesStatus(status: FilesStatus): void {
  useFilesStore.setState({ status });
}

export async function refreshTargets(): Promise<void> {
  try {
    useFilesStore.setState({ targets: await tauriApi.filesTargets() });
  } catch {
    // 앱이 아직 뜨는 중.
  }
}

export function dismissArrival(id: string): void {
  useFilesStore.setState((s) => ({ arrivals: s.arrivals.filter((t) => t.id !== id) }));
}

/** 머신·캐릭터 이름(`캐릭터@머신`). */
export function partyLabel(p: FileParty): string {
  return p.agentName ? `${p.agentName}@${p.machineName}` : p.machineName;
}

export function targetKey(t: { machineId: string; agentId?: string }): string {
  return `${t.machineId}|${t.agentId ?? ""}`;
}

/** 이 머신 기준 방향. */
export function direction(t: FileTransfer, machineId: string | undefined): "sent" | "received" | "local" | "relay" {
  const from = t.from.machineId === machineId;
  const to = t.to.machineId === machineId;
  if (from && to) return "local";
  if (from) return "sent";
  if (to) return "received";
  return "relay";
}

export function formatBytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return u === 0 ? `${n} B` : `${v.toFixed(1)} ${units[u]}`;
}

export function percent(t: FileTransfer): number {
  if (t.state === "done") return 100;
  return t.totalBytes > 0 ? Math.min(100, Math.floor((t.doneBytes * 100) / t.totalBytes)) : 0;
}

/** 이유 코드 → 번역 키(`file-target-offline: pc` → `file-target-offline`, 뒤는 설명). */
export function fileErrorKey(code: string): { key: string; detail: string } {
  const i = code.indexOf(":");
  if (i < 0) return { key: code.trim(), detail: "" };
  return { key: code.slice(0, i).trim(), detail: code.slice(i + 1).trim() };
}

/** 돌고 있는 전송 수(하단 바 표시). */
export function activeCount(list: FileTransfer[]): number {
  return list.filter((t) => t.state === "preparing" || t.state === "sending").length;
}

/** 부팅 배선. 해제 함수를 돌려준다. */
export function installFiles(): () => void {
  let disposed = false;
  const offs: (() => void)[] = [];
  const keep = (p: Promise<() => void>) =>
    void p.then((off) => (disposed ? off() : offs.push(off))).catch(() => {});
  keep(listen<FileTransfer[]>(Events.fileTransfers, (e) => useFilesStore.setState({ transfers: e.payload })));
  keep(
    listen<FileTransfer>(Events.fileReceived, (e) => {
      const t = e.payload;
      useFilesStore.setState((s) => ({ arrivals: [...s.arrivals.filter((a) => a.id !== t.id), t].slice(-3) }));
      setTimeout(() => dismissArrival(t.id), 12_000);
    }),
  );
  void Promise.resolve()
    .then(() => tauriApi.filesList())
    .then((transfers) => {
      if (!disposed && Array.isArray(transfers)) useFilesStore.setState({ transfers });
    })
    .catch(() => {});
  void Promise.resolve()
    .then(() => tauriApi.filesStatus())
    .then((status) => {
      if (!disposed && status) setFilesStatus(status);
    })
    .catch(() => {});
  return () => {
    disposed = true;
    offs.forEach((off) => off());
  };
}
