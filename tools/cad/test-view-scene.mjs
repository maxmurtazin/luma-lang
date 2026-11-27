// Render a scene produced by the Core project integration, using pinned View's WASM.
import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { cpus, platform, release, arch } from "node:os";
import { createServer } from "node:http";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [view, scene, report] = process.argv.slice(2);
const expected = process.env.VIEW_EXPECTED_REVISION ?? "3";
const brep = process.env.VIEW_BREP === "1";
if (!view || !scene) throw new Error("usage: test-view-scene.mjs VIEW_CHECKOUT SCENE_JSON [REPORT_JSON]");
const { chromium } = await import(pathToFileURL(resolve(view, "node_modules/playwright/index.mjs")));
const { PNG } = await import(pathToFileURL(resolve(view, "node_modules/pngjs/lib/png.js")));
const wasm = resolve(view, "packages/react-view/src/wasm");
const script = `
<!doctype html><canvas width="480" height="360"></canvas>
<script type="module">
import init, { create_mesh_viewport } from '/living_view_web.js';
await init();
const view = await create_mesh_viewport(document.querySelector('canvas'));
view.resize(480, 360, false);
const data = await (await fetch('/scene.json')).json();
view.open_scene(JSON.stringify(data));
if (${brep || process.env.VIEW_FIT === "1"}) view.fit();
window.current = () => JSON.parse(view.stats());
window.pickFace = async (x, y) => JSON.parse(await view.pick_topology('cad-test', x, y, 'face', 0));
</script>`;
const files = new Map([
  ["/", ["text/html", script]],
  ["/living_view_web.js", ["text/javascript", await readFile(resolve(wasm, "living_view_web.js"))]],
  ["/living_view_web_bg.wasm", ["application/wasm", await readFile(resolve(wasm, "living_view_web_bg.wasm"))]],
  ["/scene.json", ["application/json", await readFile(scene)]],
]);
const server = createServer((request, response) => {
  const file = files.get(request.url);
  if (!file) return response.writeHead(404).end();
  response.writeHead(200, { "Content-Type": file[0] }).end(file[1]);
});
await new Promise((ready) => server.listen(0, "127.0.0.1", ready));
const software = process.env.WEBGPU_SOFTWARE === "1";
let browser;
try {
  browser = await chromium.launch({
    executablePath: process.env.CHROMIUM_BIN || undefined,
    args: software ? [
      "--enable-unsafe-webgpu", "--use-angle=swiftshader", "--enable-unsafe-swiftshader",
      "--enable-features=Vulkan", "--use-vulkan=swiftshader",
    ] : [],
  });
  const page = await browser.newPage({ viewport: { width: 480, height: 360 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  async function render() {
    const start = performance.now();
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForFunction((revision) => {
      const stats = window.current?.();
      return stats?.scene?.revision === revision && stats.mesh.visible_instances === 1
        && stats.queue.submissions > 0 && stats.queue.in_flight === 0;
    }, expected, { timeout: 30000 }).catch((error) => {
      throw new Error(errors.join("; ") || String(error));
    });
    return performance.now() - start;
  }

  const coldMs = await render();
  assert.deepEqual(errors, []);
  const png = PNG.sync.read(await page.locator("canvas").screenshot());
  const corner = png.data.readUInt32BE(0);
  let changed = 0;
  for (let index = 0; index < png.data.length; index += 4) {
    if (png.data.readUInt32BE(index) !== corner) changed++;
  }
  assert.ok(changed > 1000, `expected the accepted mesh to cover visible pixels, got ${changed}`);
  if (brep) {
    const pick = await page.evaluate(async () => {
      for (const x of [200, 280, 160, 320]) {
        const candidate = await window.pickFace(x, 180);
        if (candidate.result.status === "hit") return candidate;
      }
      return null;
    });
    assert.ok(pick, "expected a face hit away from the fixture's central hole");
    assert.equal(pick.scene_revision, expected);
    assert.equal(pick.result.source.kind, "brep_publication");
    assert.ok(Number.isInteger(pick.result.entity_ordinal), JSON.stringify(pick));
    assert.match(pick.result.source.geometry_sha256, /^[0-9a-f]{64}$/);
    assert.equal(pick.result.precision, "brep_display_tessellation", JSON.stringify(pick));
  }
  if (report) {
    const adapter = await page.evaluate(async () => {
      const device = await navigator.gpu.requestAdapter();
      if (!device) throw new Error("WebGPU adapter unavailable");
      const { vendor, architecture, device: name, description, isFallbackAdapter } = device.info;
      return { vendor, architecture, device: name, description, isFallbackAdapter };
    });
    if (software) assert.match(`${adapter.architecture} ${adapter.description}`, /swiftshader|software/i);
    const session = await browser.newBrowserCDPSession();
    async function rss() {
      if (platform() !== "linux") return null;
      const { processInfo } = await session.send("SystemInfo.getProcessInfo");
      const bytes = await Promise.all(processInfo.map(async ({ id }) => {
        try {
          const status = await readFile(`/proc/${id}/status`, "utf8");
          return Number(status.match(/^VmRSS:\s+(\d+) kB$/m)?.[1] || 0) * 1024;
        } catch (error) {
          if (error.code !== "ENOENT") throw error;
          return 0;
        }
      }));
      return bytes.reduce((total, value) => total + value, 0) || null;
    }

    for (let index = 0; index < 5; index++) await render();
    const samples = [];
    const memory = [];
    for (let index = 0; index < 30; index++) {
      samples.push(await render());
      const bytes = await rss();
      if (bytes !== null) memory.push(bytes);
    }
    await session.detach();
    assert.deepEqual(errors, []);
    const ordered = [...samples].sort((left, right) => left - right);
    const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
    const sceneBytes = files.get("/scene.json")[1];
    const wasmBytes = files.get("/living_view_web_bg.wasm")[1];
    await writeFile(report, JSON.stringify({
      profile: brep ? "cad-2/view-brep-publication" : "cad-1/t1-view-accepted-plate",
      timestamp_utc: new Date().toISOString(),
      benchmark: "navigation to accepted revision rendered with idle GPU queue",
      browser: browser.version(),
      view_pin: execFileSync("git", ["-C", view, "rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      software,
      adapter,
      canvas: [480, 360],
      host: { os: platform(), kernel: release(), arch: arch(), cpu: cpus()[0]?.model },
      scene_sha256: sha256(sceneBytes),
      scene_bytes: sceneBytes.length,
      wasm_sha256: sha256(wasmBytes),
      wasm_bytes: wasmBytes.length,
      cold_ms: coldMs,
      warmup_iterations: 5,
      samples: samples.length,
      p50_ms: (ordered[14] + ordered[15]) / 2,
      p95_ms: ordered[28],
      max_sampled_browser_rss_bytes: memory.length ? Math.max(...memory) : null,
      changed_pixels: changed,
    }, null, 2) + "\n", { flag: "wx" });
  }
  process.stdout.write(`PASS View rendered accepted revision ${expected} in Chromium ${browser.version()}\n`);
} finally {
  await browser?.close();
  server.close();
}
