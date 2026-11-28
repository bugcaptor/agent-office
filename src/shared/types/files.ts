// src/shared/types/files.ts
//
// 머신 간 파일 보내기. Rust `files::*` 미러.

/** `files.json`. */
export interface FilesConfig {
  /** 다른 머신·캐릭터가 보낸 파일을 받는다(기본 꺼짐). */
  receiveEnabled: boolean;
  /** 받은 파일 폴더(절대 경로). 비우면 `~/Downloads/Agent Office`. */
  receiveDir?: string;
}

export interface FilesStatus {
  config: FilesConfig;
  /** 실제로 쓰는 받은 파일 폴더. */
  receiveDir: string;
  machineId: string;
  machineName: string;
  /** 머신 연동이 켜져 있다(꺼져 있으면 이 머신 캐릭터끼리만). */
  linked: boolean;
}

/** 보내는 쪽·받는 쪽. 머신은 연동 기계 id로 가리킨다. */
export interface FileParty {
  machineId: string;
  machineName: string;
  agentId?: string;
  agentName?: string;
}

export interface FileEntry {
  /** 항목 기준 상대 경로(`/`). */
  path: string;
  size: number;
  sha256: string;
  mtime: number;
}

export type FileTransferState = "preparing" | "sending" | "done" | "failed" | "cancelled";

export interface FileTransfer {
  id: string;
  createdAt: number;
  updatedAt: number;
  from: FileParty;
  to: FileParty;
  /** 맨 위 항목 이름(파일·폴더). */
  items: string[];
  files: FileEntry[];
  dirs?: string[];
  totalBytes: number;
  doneBytes: number;
  progress?: number[];
  state: FileTransferState;
  /** 이유 코드(`file-target-offline: …`처럼 설명이 붙을 수 있다). */
  error?: string;
  note?: string;
  notify: boolean;
  /** 받는 쪽이 저장한 맨 위 항목의 절대 경로. */
  saved?: string[];
  /** `app`·`web`·`agent`·`cli`·`ctl`. */
  via: string;
}

/** 보낼 수 있는 곳 하나. */
export interface FileTarget {
  machineId: string;
  machineName: string;
  agentId?: string;
  agentName?: string;
  /** 호스트에서 본 연동 기기 id(원격 캐릭터 id `기기id:캐릭터id`와 맞춘다). */
  deviceId?: string;
  /** 이 머신. */
  local: boolean;
  online: boolean;
  /** 그 머신이 파일 받기를 켰다. */
  receive: boolean;
}

export interface FileSendRequest {
  machineId: string;
  agentId?: string;
  paths: string[];
  note?: string;
  notify: boolean;
  /** 보내는 캐릭터(이 머신). */
  fromAgent?: string;
}
