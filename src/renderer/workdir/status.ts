// src/renderer/workdir/status.ts
//
// git 상태 단일 문자 뱃지(M/A/D/R/C/U/T/?)를 사람이 읽는 문구로 옮기는 공용
// 헬퍼. 파일 목록(WorkdirPalette)·상세 페인·로그 브라우저가 모두 같은 뱃지
// 어휘를 쓰도록 한 곳에 모았다(이슈 #54).
//
// i18n: 이 모듈은 **번역문이 아니라 키를 다룬다**. 완성된 문구를 만들어 돌려주면
// 모듈 로드 시점의 언어에 굳어 버리므로, 표는 `workdir` 네임스페이스의 키만
// 담고 실제 번역은 호출자가 넘긴 `t`(컴포넌트의 useTranslation)로 한다 —
// 그래야 언어를 바꿨을 때 이미 그려진 뱃지도 함께 바뀐다.

/** 뱃지 문자 → `workdir` 네임스페이스의 상태 라벨 키. */
const STATUS_KEYS: Record<string, string> = {
  M: "status.modified",
  A: "status.added",
  D: "status.deleted",
  R: "status.renamed",
  C: "status.copied",
  U: "status.conflicted",
  T: "status.typeChanged",
  "?": "status.untracked",
};

/** 뱃지 문자의 상태 라벨 키. 모르는 문자는 키가 없다(undefined). */
export function statusLabelKey(status: string): string | undefined {
  return STATUS_KEYS[status];
}

/** 뱃지 문자 → 사람이 읽는 상태(툴팁·접근성). `t`는 `workdir` 네임스페이스에
 *  바인딩된 번역 함수. 모르는 문자는 문자 그대로 돌려준다. */
export function statusLabel(status: string, t: (key: string) => string): string {
  const key = statusLabelKey(status);
  return key ? t(key) : status;
}
