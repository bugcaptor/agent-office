// src/renderer/ipc/osNotify.ts
//
// OS 데스크탑 알림 발송 래퍼(이슈 #39). 앱 창이 백그라운드일 때 완료/확인
// 알림을 OS 알림 센터로 내보내, 터미널이 열려 있어도(=인앱 억제 대상이어도)
// 놓치지 않게 한다. 발송 판단(창 포커스 여부)은 호출자(sessionBridge)가 하고,
// 이 모듈은 권한 확인/요청 + 발송만 책임진다.
//
// `@tauri-apps/plugin-notification`은 동적 import 한다 — 정적 import 하면
// sessionBridge를 로드하는 모든 vitest 파일이 이 플러그인 모듈을 해석해야
// 하는데, 실제 발송(창 비포커스)이 없는 테스트에선 불필요하다. 발송 시점에만
// 로드된다.

/** 권한 확인/요청을 최초 발송 전 1회만 수행하기 위한 모듈 캐시. */
let permissionChecked = false;
let permissionGranted = false;

/**
 * OS 데스크탑 알림을 보낸다. 권한이 없으면 최초 1회 요청하고, 거부되면 조용히
 * no-op. 어떤 실패도(플러그인 부재/런타임 밖 등) 콘솔 경고로만 삼킨다 —
 * 인앱 티커/배지는 이와 무관하게 이미 동작한다.
 */
export async function maybeSendOsNotification(title: string, body: string): Promise<void> {
  try {
    const { isPermissionGranted, requestPermission, sendNotification } = await import(
      "@tauri-apps/plugin-notification"
    );
    if (!permissionChecked) {
      permissionChecked = true;
      permissionGranted = await isPermissionGranted();
      if (!permissionGranted) {
        permissionGranted = (await requestPermission()) === "granted";
      }
    }
    if (!permissionGranted) return;
    sendNotification({ title, body });
  } catch (err) {
    console.warn("osNotify: OS 알림 발송 실패", err);
  }
}

/** 테스트 전용: 권한 캐시 초기화(각 케이스가 권한 흐름을 독립적으로 검증). */
export function __resetOsNotifyPermissionCacheForTests(): void {
  permissionChecked = false;
  permissionGranted = false;
}
