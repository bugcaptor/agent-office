// src/renderer/settings/keyStatus.ts
//
// API 키 저장 상태를 사람이 읽는 한 조각으로 만드는 공용 문구. 요약 탭
// (SummarySection)과 소리·음성 탭(TtsSection)이 같은 0600 저장소를 보므로
// 표현도 하나로 공유한다.


/** "있음 / 있음(환경변수) / 없음" — 키 상태 한 조각. 요약 탭과 소리·음성 탭이
 *  같은 저장소를 보므로 문구도 하나로 공유한다. */
export function keyStateLabel(
  t: (key: string) => string,
  set: boolean,
  fromEnv: boolean
): string {
  if (!set) return t("keys.absent");
  return fromEnv ? t("keys.presentEnv") : t("keys.present");
}
