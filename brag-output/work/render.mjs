// Usage: node render.mjs stills 1.2 3.0 ...   |   node render.mjs all
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const require = createRequire('/opt/node-tools/node_modules/');
const { chromium } = require('playwright');
const here = path.dirname(fileURLToPath(import.meta.url));
const FPS = 30, DUR = 24;
const [mode, ...rest] = process.argv.slice(2);
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: Number(process.env.DSF || 1) });
page.on('console', (m) => { if (m.type() === 'error') console.log('console:', m.text()); });
page.on('pageerror', (e) => console.log('pageerror:', e.message));
await page.goto('file://' + path.join(here, 'composition.html'));
await page.evaluate(() => window.ready);
// warm every scene once so background-image pieces are decoded before capture
for (let t = 0; t < DUR; t += 0.5) await page.evaluate((t) => window.seek(t), t);
if (mode === 'stills') {
  mkdirSync(path.join(here, process.env.OUT || 'stills2'), { recursive: true });
  for (const s of rest) {
    const t = Number(s);
    await page.evaluate((t) => window.seek(t), t);
    await page.screenshot({ path: path.join(here, process.env.OUT || 'stills2', `t${t.toFixed(2).padStart(5, '0')}.jpg`), type: 'jpeg', quality: 80 });
  }
} else {
  mkdirSync(path.join(here, 'frames2'), { recursive: true });
  for (let f = Number(process.env.START || 0); f < Number(process.env.END || FPS * DUR); f++) {
    await page.evaluate((t) => window.seek(t), f / FPS);
    await page.screenshot({ path: path.join(here, 'frames2', `f${String(f).padStart(4, '0')}.jpg`), type: 'jpeg', quality: 96 });
    if (f % 90 === 0) console.log('frame', f);
  }
}
await browser.close();
