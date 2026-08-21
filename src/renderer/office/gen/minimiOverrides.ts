// src/renderer/office/gen/minimiOverrides.ts
//
// 서브에이전트 미니미용 커스텀 픽셀아트(디코드된 단일 N×N 캔버스) 레지스트리.
// `spriteOverrides`와 동형이되 담는 것이 4프레임 시트가 아니라 **한 프레임**이다
// (미니미는 애니메이션하지 않고 sin 밥으로만 흔든다).
//
// Zustand 밖의 모듈 상태로 두어 office/gen이 스토어에 의존하지 않게 한다.
// 쓰기: sprite/minimiCache·SpriteEditor, 읽기: minimiFactory·OfficeWorld.
const overrides = new Map<string, CanvasImageSource>();

export function setMinimiOverride(id: string, frame: CanvasImageSource): void {
  overrides.set(id, frame);
}

export function clearMinimiOverride(id: string): void {
  overrides.delete(id);
}

export function getMinimiOverride(id: string): CanvasImageSource | undefined {
  return overrides.get(id);
}

/** 테스트 격리용 전체 초기화. */
export function resetMinimiOverrides(): void {
  overrides.clear();
}
