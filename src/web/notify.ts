// src/web/notify.ts
//
// 브라우저 알림(Notification API). **탭이 열려 있을 때만** 뜬다 — Web Push와
// 서비스 워커는 범위 밖이다(앱을 닫아도 오는 알림은 별개 설계다).
//
// 게이트가 세 겹인 이유:
//   ① 사용자 토글 — 켠 적 없으면 아무것도 뜨지 않는다(localStorage).
//   ② 권한       — 브라우저가 허락해야 한다. 거부는 되돌릴 수 없다(사이트 설정).
//   ③ 가시성     — 이미 보고 있는 탭에 시스템 알림을 겹치지 않는다.
//
// 지원 자체가 없는 환경(iOS Safari 비-PWA 등)에서는 `supported()`가 false라
// 호출부가 버튼을 아예 그리지 않는다. 그 밖의 모든 진입점은 try/catch로 감싼다 —
// 알림이 안 뜨는 것은 불편이지만 화면이 죽는 것은 결함이다.

/** 사용자 토글 저장 키. */
export const NOTIFY_KEY = "ao-web-notify";

/** 이 브라우저에 Notification API가 있는가. */
export function supported(): boolean {
  return typeof window !== "undefined" && "Notification" in window;
}

/** 현재 권한 상태. API가 없으면 null. */
export function permission(): NotificationPermission | null {
  if (!supported()) return null;
  try {
    return Notification.permission;
  } catch {
    return null;
  }
}

/** 사용자 토글 상태(기본 꺼짐). */
export function enabled(): boolean {
  try {
    return localStorage.getItem(NOTIFY_KEY) === "on";
  } catch {
    // 프라이빗 모드·스토리지 차단 — 토글은 이번 세션에서 꺼진 것으로 본다.
    return false;
  }
}

export function setEnabled(on: boolean): void {
  try {
    localStorage.setItem(NOTIFY_KEY, on ? "on" : "off");
  } catch {
    /* 저장 못 해도 이번 화면 동작은 막지 않는다 */
  }
}

/** 권한을 요청한다(이미 결정돼 있으면 그 값 그대로). API가 없으면 null. */
export async function requestPermission(): Promise<NotificationPermission | null> {
  if (!supported()) return null;
  try {
    if (Notification.permission !== "default") return Notification.permission;
    return await Notification.requestPermission();
  } catch {
    return null;
  }
}

/** 알림을 띄울지 판단하는 **순수** 규칙(세 겹 게이트). */
export function shouldNotify(gate: {
  enabled: boolean;
  permission: NotificationPermission | null;
  visibility: DocumentVisibilityState | string;
}): boolean {
  return gate.enabled && gate.permission === "granted" && gate.visibility !== "visible";
}

export interface NotifyOptions {
  title: string;
  body: string;
  /** 같은 알림이 두 번 뜨지 않게 하는 키(알림 id). */
  tag: string;
  icon?: string;
  onClick?: () => void;
}

/** 알림 하나를 띄운다. 실패하면 조용히 null(호출부는 신경 쓰지 않는다). */
export function show(opts: NotifyOptions): Notification | null {
  if (!supported()) return null;
  try {
    const n = new Notification(opts.title, {
      body: opts.body,
      tag: opts.tag,
      icon: opts.icon,
    });
    if (opts.onClick) {
      n.onclick = () => {
        try {
          window.focus();
        } catch {
          /* 브라우저가 막으면 그대로 둔다 */
        }
        n.close();
        opts.onClick?.();
      };
    }
    return n;
  } catch {
    return null;
  }
}
