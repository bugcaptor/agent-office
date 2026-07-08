// src/renderer/office/gen/spriteOverrides.ts
//
// 커스텀 스프라이트 시트(디코드된 64×16 캔버스) 레지스트리. Zustand 밖의
// 모듈 상태로 두어 office/gen이 스토어에 의존하지 않게 한다(레이어 방향 유지).
// 쓰기: sprite/spriteCache·SpriteEditor, 읽기: characterFactory·OfficeWorld.
const overrides = new Map<string, CanvasImageSource>();

export function setSpriteOverride(id: string, sheet: CanvasImageSource): void {
  overrides.set(id, sheet);
}

export function clearSpriteOverride(id: string): void {
  overrides.delete(id);
}

export function getSpriteOverride(id: string): CanvasImageSource | undefined {
  return overrides.get(id);
}

/** 테스트 격리용 전체 초기화. */
export function resetSpriteOverrides(): void {
  overrides.clear();
}
