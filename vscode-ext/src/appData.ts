// vscode-ext/src/appData.ts
//
// app_data 폴더 발견. 앱의 `ctl` 클라이언트(src-tauri/src/control/client.rs
// `resolve_app_data`/`default_app_data`)와 같은 규약을 그대로 옮긴 순수 모듈이다.
//
//   VSCode 설정 agentOffice.appDataDir > env AGENT_OFFICE_APP_DATA > OS 기본
//
// vscode 모듈에 의존하지 않는다(테스트 대상).

import * as path from "node:path";

/** Tauri identifier — app_data 폴더 이름이 된다. */
export const IDENTIFIER = "com.bugcaptor.agent-office";

/** 발견에 필요한 바깥 세계 입력. 전부 주입받아 플랫폼 무관하게 테스트한다. */
export interface AppDataInput {
  /** VSCode 설정 `agentOffice.appDataDir`. */
  configValue?: string;
  /** 환경변수 `AGENT_OFFICE_APP_DATA`(앱이 세션 터미널에 자동 주입한다). */
  env?: string;
  /** `process.platform` 값. */
  platform: string;
  /** `$HOME` (macOS·Linux 기본 경로용). */
  home?: string;
  /** `$XDG_DATA_HOME` (Linux). */
  xdgDataHome?: string;
  /** `%APPDATA%` (Windows). */
  appData?: string;
}

/** 비었거나 공백뿐인 값은 "없음"으로 취급한다. */
function clean(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

/**
 * 사용자가 설정에 `~/…`를 적는 일이 흔하므로 홈을 펼친다. 앱의 ctl은 셸이
 * 이미 펼쳐 주므로 이 처리가 없지만, VSCode 설정 값은 아무도 펼쳐 주지 않는다.
 */
function expandHome(value: string, home: string | undefined): string {
  if (!home) return value;
  if (value === "~") return home;
  if (value.startsWith("~/") || value.startsWith("~\\")) {
    return path.join(home, value.slice(2));
  }
  return value;
}

/** OS별 기본 app_data 경로. 알 수 없는 플랫폼이면 undefined. */
export function defaultAppData(input: AppDataInput): string | undefined {
  const home = clean(input.home);
  switch (input.platform) {
    case "darwin":
      return home
        ? path.join(home, "Library", "Application Support", IDENTIFIER)
        : undefined;
    case "linux": {
      const base = clean(input.xdgDataHome) ?? (home ? path.join(home, ".local", "share") : undefined);
      return base ? path.join(base, IDENTIFIER) : undefined;
    }
    case "win32": {
      const base = clean(input.appData);
      return base ? path.join(base, IDENTIFIER) : undefined;
    }
    default:
      return undefined;
  }
}

/** 발견 규약 본체: 설정값 > env > OS 기본. 어느 것도 없으면 undefined. */
export function resolveAppData(input: AppDataInput): string | undefined {
  const home = clean(input.home);
  const explicit = clean(input.configValue) ?? clean(input.env);
  if (explicit) return expandHome(explicit, home);
  return defaultAppData(input);
}

/** 실행 중인 프로세스 환경에서 입력을 모은다(설정값만 호출부가 넣어 준다). */
export function appDataInputFromProcess(configValue?: string): AppDataInput {
  return {
    configValue,
    env: process.env.AGENT_OFFICE_APP_DATA,
    platform: process.platform,
    home: process.env.HOME ?? process.env.USERPROFILE,
    xdgDataHome: process.env.XDG_DATA_HOME,
    appData: process.env.APPDATA,
  };
}
