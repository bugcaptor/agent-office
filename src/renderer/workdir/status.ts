// src/renderer/workdir/status.ts
//
// git 상태 단일 문자 뱃지(M/A/D/R/C/U/T/?)를 사람이 읽는 한국어로 옮기는 공용
// 헬퍼. 파일 목록(WorkdirPalette)·상세 페인·로그 브라우저가 모두 같은 뱃지
// 어휘를 쓰도록 한 곳에 모았다(이슈 #54).

/** 뱃지 문자 → 사람이 읽는 상태(툴팁·접근성). */
export function statusLabel(status: string): string {
  switch (status) {
    case "M":
      return "수정됨";
    case "A":
      return "추가됨";
    case "D":
      return "삭제됨";
    case "R":
      return "이름변경";
    case "C":
      return "복사됨";
    case "U":
      return "충돌";
    case "T":
      return "타입변경";
    case "?":
      return "추적 안 됨";
    default:
      return status;
  }
}
