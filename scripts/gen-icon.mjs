// scripts/gen-icon.mjs — 앱 아이콘 소스(1024×1024) 생성.
// 32×32 픽셀 그리드에 캐릭터 생성기 룩(palette.ts 참고)의 얼굴 1개를 그려
// 32배 확대한다. 실행: node scripts/gen-icon.mjs → scripts/icon-source.png
import { createCanvas } from "@napi-rs/canvas";
import { writeFileSync } from "node:fs";

const G = 32; // grid
const S = 32; // scale
const PAL = {
  bg: "#1b1b24",       // 앱 배경색
  bgLight: "#26263a",
  outline: "#1a1420",  // palette.ts outline
  skin: "#e8b08a",
  skinShadow: "#c98d68",
  hair: "#4a3120",
  hairLight: "#6b4a30",
  shirt: "#3f6fb5",
  eye: "#1a1420",
  highlight: "#ffffff",
};

// 32×32 도트맵. 키는 PAL 키, "." = 배경.
// 얼굴(머리+피부+눈+셔츠 어깨선)을 중앙에 크게 배치.
const rows = [];
for (let y = 0; y < G; y++) rows.push(new Array(G).fill("."));

function rect(x0, y0, x1, y1, c) {
  for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) rows[y][x] = c;
}

// 머리카락(상단 둥근 블록 + 옆머리)
rect(9, 3, 22, 9, "hair");
rect(10, 2, 21, 2, "hair");
rect(7, 8, 9, 16, "hair");
rect(22, 8, 24, 16, "hair");
rect(11, 4, 16, 6, "hairLight");
// 얼굴
rect(10, 9, 21, 21, "skin");
rect(10, 19, 21, 21, "skinShadow");
// 눈 (2×2 도트 두 개)
rect(12, 14, 13, 15, "eye");
rect(18, 14, 19, 15, "eye");
rect(12, 14, 12, 14, "highlight");
rect(18, 14, 18, 14, "highlight");
// 목 + 셔츠(어깨)
rect(13, 22, 18, 23, "skin"); // 목
rect(7, 24, 24, 29, "shirt");
// 외곽선: 실루엣(배경 아닌 도트)에 4방향으로 인접한 배경 도트를 외곽선색으로
// — 캐릭터 스프라이트의 1px 아웃라인 룩 재현.
const outlineDots = [];
for (let y = 0; y < G; y++)
  for (let x = 0; x < G; x++) {
    if (rows[y][x] !== ".") continue;
    const touchesBody = [[1, 0], [-1, 0], [0, 1], [0, -1]].some(([dx, dy]) => {
      const nx = x + dx, ny = y + dy;
      return nx >= 0 && ny >= 0 && nx < G && ny < G && rows[ny][nx] !== "." && rows[ny][nx] !== "outline";
    });
    if (touchesBody) outlineDots.push([x, y]);
  }
for (const [x, y] of outlineDots) rows[y][x] = "outline";

const canvas = createCanvas(G * S, G * S);
const ctx = canvas.getContext("2d");
// 배경: 모서리 라운드 사각형(살짝) + 대각 그라디언트 느낌의 2톤
ctx.fillStyle = PAL.bg;
ctx.fillRect(0, 0, G * S, G * S);
ctx.fillStyle = PAL.bgLight;
for (let y = 0; y < G; y++)
  for (let x = 0; x < G; x++)
    if ((x + y) % 2 === 0 && (x < 3 || y < 3 || x >= G - 3 || y >= G - 3))
      ctx.fillRect(x * S, y * S, S, S);
// 도트 렌더
for (let y = 0; y < G; y++)
  for (let x = 0; x < G; x++) {
    const k = rows[y][x];
    if (k === ".") continue;
    ctx.fillStyle = PAL[k];
    ctx.fillRect(x * S, y * S, S, S);
  }
writeFileSync(new URL("./icon-source.png", import.meta.url), canvas.toBuffer("image/png"));
console.log("scripts/icon-source.png written");
