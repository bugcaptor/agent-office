// Headless GUI harness: renderer + fake backend in headless Chrome, no Tauri build.
// Design and limits: docs/gui-harness-design.md
//
//   npm run gui:shot -- [--scenario office[,working,...]] [--out DIR] [--size 1280x800]
//                       [--lang ko] [--script steps.mjs] [--serve]
//
// Writes DIR/<scenario>.png, prints console errors and calls the fake backend
// does not answer. Exit code 1 when the page threw (uncaught error).
// --script: `export default async ({ page, shot, harness }) => {}` per scenario.
// --serve: keep the harness Vite server up and print URLs (for DevTools MCP etc.).
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { createServer } from "vite";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : fallback;
};
const scenarios = opt("scenario", "office").split(",");
const outDir = resolve(opt("out", "/tmp/agent-office-gui"));
const [width, height] = opt("size", "1280x800").split("x").map(Number);
const lang = opt("lang", "ko");
const scriptPath = opt("script");
const serveOnly = args.includes("--serve");

const root = process.cwd();
const harnessDir = resolve(root, "src/renderer/guiHarness");
const realApi = resolve(root, "src/renderer/ipc/tauriApi.ts");

const harnessPlugin = {
  name: "gui-harness",
  enforce: "pre",
  async resolveId(source, importer, options) {
    if (!importer || importer.startsWith(harnessDir) || !source.includes("tauriApi")) return null;
    const resolved = await this.resolve(source, importer, { ...options, skipSelf: true });
    return resolved?.id === realApi ? resolve(harnessDir, "fakeApi.ts") : null;
  },
  transformIndexHtml: () => [
    { tag: "script", attrs: { type: "module", src: "/src/renderer/guiHarness/tauriStub.ts" }, injectTo: "head-prepend" },
  ],
};

const server = await createServer({
  plugins: [harnessPlugin],
  server: { port: 1430, strictPort: false, hmr: false },
  clearScreen: false,
  logLevel: "warn",
});
await server.listen();
const base = server.resolvedUrls.local[0];
const urlFor = (scenario, page = "") => `${base}${page}?scenario=${scenario}&lang=${lang}`;

if (serveOnly) {
  for (const s of scenarios) console.log(`${s}: ${urlFor(s)}   mascot: ${urlFor(s, "mascot.html")}`);
  console.log("Ctrl+C to stop");
} else {
  let failed = false;
  try {
    failed = await shootAll();
  } finally {
    await server.close();
  }
  process.exit(failed ? 1 : 0);
}

async function launch() {
  const { chromium } = await import("playwright-core");
  // 시스템 Chrome 재사용(다운로드 없음). swiftshader로 GPU 없이 Pixi WebGL을 그린다.
  const launchArgs = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--autoplay-policy=no-user-gesture-required"];
  for (const channel of ["chrome", "msedge"]) {
    try {
      return await chromium.launch({ channel, headless: true, args: launchArgs });
    } catch (err) {
      if (channel === "msedge") throw err;
    }
  }
}

async function shootAll() {
  const steps = scriptPath ? (await import(pathToFileURL(resolve(scriptPath)).href)).default : null;
  await mkdir(outDir, { recursive: true });
  const browser = await launch();
  let failed = false;
  try {
    for (const scenario of scenarios) {
      const t0 = Date.now();
      const page = await browser.newPage({ viewport: { width, height }, locale: lang });
      const problems = [];
      page.on("console", (m) => {
        if ((m.type() === "error" || m.type() === "warning") && !m.text().startsWith("Failed to load resource")) problems.push(`[${m.type()}] ${m.text().split("\n")[0]}`);
      });
      page.on("response", (r) => {
        if (r.status() >= 400) problems.push(`[http ${r.status()}] ${r.url()}`);
      });
      page.on("pageerror", (e) => {
        failed = true;
        problems.push(`[pageerror] ${e.message}`);
      });
      await page.goto(urlFor(scenario), { waitUntil: "networkidle" });
      await page.waitForTimeout(600);
      const shots = [];
      const shot = async (name = scenario) => {
        const file = resolve(outDir, `${name}.png`);
        await page.screenshot({ path: file });
        shots.push(file);
        return file;
      };
      const harness = {
        /** 페이지 안 `window.__guiHarness`의 함수를 부른다. */
        call: (fn, ...a) => page.evaluate(([f, x]) => window.__guiHarness[f](...x), [fn, a]),
        /** 스토어 상태를 바꾸는 함수(문자열 아님)를 페이지에서 실행한다. */
        store: (fn, ...a) =>
          page.evaluate(async ([src, x]) => {
            const store = await window.__guiHarness.store();
            return new Function(`return (${src})`)()(store.getState(), ...x);
          }, [fn.toString(), a]),
      };
      if (steps) await steps({ page, shot, harness, scenario });
      else await shot();
      const log = await page.evaluate(() => window.__guiHarness.log);
      const unfaked = [...new Set(log.unfaked)];
      console.log(`${scenario}: ${shots.join(", ")} (${Date.now() - t0}ms)`);
      if (unfaked.length) console.log(`  unfaked: ${unfaked.join(" ")}`);
      for (const p of problems.filter((p) => !/GL Driver Message|AudioContext/.test(p))) console.log(`  ${p}`);
      await page.close();
    }
  } finally {
    await browser.close();
  }
  return failed;
}
