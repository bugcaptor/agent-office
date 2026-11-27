// src/renderer/link/linkStore.ts
//
// 머신 연동 상태(`link-status` 이벤트)를 들고 있다. 설정 › 연동 탭과 화면 아래 왼쪽 표시기가 본다.
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import type { LinkStatus } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";

interface LinkState {
  status: LinkStatus | null;
}

export const useLinkStore = create<LinkState>(() => ({ status: null }));

export function setLinkStatus(status: LinkStatus): void {
  useLinkStore.setState({ status });
}

/** 표시기 한 줄: 호스트는 붙은 기기 수, 클라이언트는 호스트 연결 여부. 보일 게 없으면 null. */
export function linkIndicator(status: LinkStatus | null): { kind: "host"; count: number } | { kind: "client"; connected: boolean; name: string } | null {
  if (status?.host) {
    const count = status.host.devices.filter((d) => d.connected).length;
    return count > 0 ? { kind: "host", count } : null;
  }
  const client = status?.client;
  if (client?.paired && client.address) {
    return { kind: "client", connected: client.state === "connected", name: client.hostName ?? client.address };
  }
  return null;
}

/** 이유 코드 → 번역 키와 값. `link-http-404`처럼 숫자가 붙은 것, `link-unreachable: …`처럼 설명이 붙은 것을 나눈다. */
export function linkErrorKey(code: string): { key: string; detail: string } {
  const [head, ...rest] = code.split(":");
  const detail = rest.join(":").trim();
  const http = /^link-http-(\d+)$/.exec(head);
  if (http) return { key: "link-http", detail: http[1] };
  return { key: head, detail };
}

export function installLink(): () => void {
  let disposed = false;
  let off: (() => void) | undefined;
  void listen<LinkStatus>(Events.linkStatus, (event) => setLinkStatus(event.payload)).then((unlisten) => {
    if (disposed) unlisten();
    else off = unlisten;
  });
  void tauriApi.linkStatus().then((status) => {
    if (!disposed) setLinkStatus(status);
  }).catch(() => {});
  return () => {
    disposed = true;
    off?.();
  };
}
