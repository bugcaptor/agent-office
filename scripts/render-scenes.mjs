// CPU contact sheets for reviewing authored scene art; no Tauri/GPU required.
// Usage: node scripts/render-scenes.mjs /tmp/agent-office-scenes
// This exercises drawTile directly, not Pixi's texture cache or live overlays.
import { mkdir, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { relative, resolve } from "node:path";
import { createCanvas } from "@napi-rs/canvas";
import { createServer } from "vite";

const output = resolve(process.argv.slice(2).find((arg) => arg !== "--baseline") ?? "/tmp/agent-office-scenes");
const baseline = process.argv.includes("--baseline");
const server = await createServer({
  server: { middlewareMode: true, hmr: false },
  appType: "custom",
  plugins: baseline ? [{
    name: "scene-art-at-head",
    enforce: "pre",
    load(id) {
      if (id.includes("/src/renderer/office/scenes/") && id.endsWith(".ts")) {
        return execFileSync("rtk", ["proxy", "git", "show", `HEAD:${relative(process.cwd(), id)}`], { encoding: "utf8" });
      }
    },
  }] : [],
});
const hex = (color) => `#${color.toString(16).padStart(6, "0")}`;
try {
  const { SCENES, SCENE_ORDER } = await server.ssrLoadModule("/src/renderer/office/scenes/scenes.ts");
  const { THEMES, THEME_ORDER } = await server.ssrLoadModule("/src/renderer/theme/themes.ts");
  const { generateSheet } = await server.ssrLoadModule("/src/renderer/office/gen/sheetGen.ts");
  const factory = (w, h) => {
    const canvas = createCanvas(w, h);
    return { canvas, ctx: canvas.getContext("2d") };
  };
  const people = Array.from({ length: 8 }, (_, i) => generateSheet(`scene-preview-${i}`, factory).sheet);
  await mkdir(output, { recursive: true });
  for (const themeId of THEME_ORDER) {
    const sheet = createCanvas(1328, 5 * 492 + 16);
    const ctx = sheet.getContext("2d");
    ctx.fillStyle = "#171b25";
    ctx.fillRect(0, 0, sheet.width, sheet.height);
    ctx.imageSmoothingEnabled = false;
    for (const [index, id] of SCENE_ORDER.entries()) {
      const scene = SCENES[id];
      const render = scene.resolve(THEMES[themeId]);
      const canvas = createCanvas(320, 224);
      const c = canvas.getContext("2d");
      c.imageSmoothingEnabled = false;
      c.fillStyle = hex(render.background);
      c.fillRect(0, 0, 320, 224);
      for (let ty = 0; ty < scene.map.height; ty++) {
        for (let tx = 0; tx < scene.map.width; tx++) {
          c.save();
          c.translate(tx * 16, ty * 16);
          let rect;
          const g = {
            rect(...bounds) { rect = bounds; return this; },
            fill(style) {
              if (!rect.every(Number.isFinite) || rect[2] < 0 || rect[3] < 0) {
                throw new Error(`Invalid rectangle: ${id}/${themeId}/${tx},${ty}: ${rect}`);
              }
              c.fillStyle = hex(typeof style === "number" ? style : style.color);
              c.globalAlpha = typeof style === "number" ? 1 : style.alpha ?? 1;
              c.fillRect(...rect);
              return this;
            },
          };
          render.drawTile(g, { t: scene.map.tiles[ty][tx], tx, ty, s: 16, map: scene.map });
          c.restore();
        }
      }
      for (const [i, desk] of scene.map.desks.entries()) {
        const person = people[i % people.length];
        c.drawImage(person.canvas, 0, 0, person.cell, person.cell, desk.seat.tx * 16, desk.seat.ty * 16, 16, 16);
      }
      const x = 16 + (index % 2) * 656;
      const y = 16 + Math.floor(index / 2) * 492;
      ctx.font = "16px monospace";
      ctx.fillStyle = "#e4e8ef";
      ctx.fillText(`${id} / ${themeId}`, x, y + 18);
      ctx.drawImage(canvas, x, y + 28, 640, 448);
      await writeFile(resolve(output, `${id}-${themeId}.png`), canvas.toBuffer("image/png"));
    }
    await writeFile(resolve(output, `${themeId}.png`), sheet.toBuffer("image/png"));
  }
  console.log(`Rendered ${SCENE_ORDER.length} scenes × ${THEME_ORDER.length} themes to ${output}`);
} finally {
  await server.close();
}
