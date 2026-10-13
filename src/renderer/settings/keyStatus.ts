// src/renderer/settings/keyStatus.ts
//
// API 키 저장 상태를 사람이 읽는 한 조각으로 만드는 문구(요약 탭).


/** "있음 / 있음(환경변수) / 없음" — 키 상태 한 조각. */
export function keyStateLabel(
  t: (key: string) => string,
  set: boolean,
  fromEnv: boolean
): string {
  if (!set) return t("keys.absent");
  return fromEnv ? t("keys.presentEnv") : t("keys.present");
}
