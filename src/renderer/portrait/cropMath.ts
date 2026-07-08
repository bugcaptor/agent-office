// src/renderer/portrait/cropMath.ts
//
// 크롭 뷰포트 <-> 원본 소스 사각형 변환. 순수(라이브러리/DOM 비의존)라 단위
// 테스트로 검증하고, PortraitEditor는 이 사각형으로 canvas.drawImage(src rect ->
// 240x320 출력)만 수행한다.
//
// 모델: 이미지를 배율 `scale`(이미지px -> 프레임px)로 그리고, 이미지의 좌상단이
// 프레임 좌표 (offsetX, offsetY)에 온다. 프레임은 [0,frameW] x [0,frameH].

export interface CropView {
  scale: number;
  offsetX: number;
  offsetY: number;
}

export interface SourceRect {
  sx: number;
  sy: number;
  sw: number;
  sh: number;
}

/** 프레임을 꽉 채우는 center-cover 초기 뷰. */
export function initialCoverView(
  imgW: number,
  imgH: number,
  frameW: number,
  frameH: number
): CropView {
  const scale = Math.max(frameW / imgW, frameH / imgH);
  return {
    scale,
    offsetX: (frameW - imgW * scale) / 2,
    offsetY: (frameH - imgH * scale) / 2,
  };
}

/** 현재 뷰에서 프레임 전체에 대응하는 원본 이미지 소스 사각형. */
export function viewToSourceRect(
  view: CropView,
  frameW: number,
  frameH: number
): SourceRect {
  return {
    sx: -view.offsetX / view.scale,
    sy: -view.offsetY / view.scale,
    sw: frameW / view.scale,
    sh: frameH / view.scale,
  };
}

/** 프레임 좌표 (px,py)를 고정점으로 삼아 scale을 factor배 한다. */
export function zoomAt(
  view: CropView,
  factor: number,
  px: number,
  py: number
): CropView {
  const scale = view.scale * factor;
  const imgX = (px - view.offsetX) / view.scale;
  const imgY = (py - view.offsetY) / view.scale;
  return {
    scale,
    offsetX: px - imgX * scale,
    offsetY: py - imgY * scale,
  };
}

/** 이미지를 프레임 기준 (dx,dy)만큼 이동. */
export function panBy(view: CropView, dx: number, dy: number): CropView {
  return { ...view, offsetX: view.offsetX + dx, offsetY: view.offsetY + dy };
}
