// Render a scene produced by the Core project integration, using pinned View's WASM.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [view, scene] = process.argv.slice(2);
if (!view || !scene) throw new Error("usage: test-view-scene.mjs VIEW_CHECKOUT SCENE_JSON");
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
window.current = () => JSON.parse(view.stats());
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
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.waitForFunction(() => {
    const stats = window.current?.();
    return stats?.scene?.revision === "3" && stats.mesh.visible_instances === 1
      && stats.queue.submissions > 0 && stats.queue.in_flight === 0;
  }, undefined, { timeout: 30000 }).catch((error) => { throw new Error(errors.join("; ") || String(error)); });
  assert.deepEqual(errors, []);
  const png = PNG.sync.read(await page.locator("canvas").screenshot());
  const corner = png.data.readUInt32BE(0);
  let changed = 0;
  for (let index = 0; index < png.data.length; index += 4) {
    if (png.data.readUInt32BE(index) !== corner) changed++;
  }
  assert.ok(changed > 1000, "expected the accepted Core mesh to cover visible pixels");
  process.stdout.write(`PASS View rendered accepted Core revision 3 in Chromium ${browser.version()}\n`);
} finally {
  await browser?.close();
  server.close();
}
