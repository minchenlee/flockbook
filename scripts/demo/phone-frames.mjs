// The phone slides for the README and the website: phone stills (scripts/demo/phone-stills.mjs) in plain phone
// frames on a transparent 2560x1704 canvas, the shape of the Mac stills.
//   node scripts/demo/phone-frames.mjs <phone-stills-dir> [out-dir]   -> <out>/phone-*.png, push-*.png, lock-*.png
// phone: the list of agents and a conversation with pictures. push: a lock screen with the notifications the hub
// sends (title "Needs you · <agent title>", body "<agent> · <workspace> · <tab>", as PushNotice.agent words them),
// next to the list they open.
import { createRequire } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '../..');
const [stills, out = join(root, 'docs/media')] = process.argv.slice(2);
if (!stills) { console.log('usage: node scripts/demo/phone-frames.mjs <phone-stills-dir> [out-dir]'); process.exit(2); }
// Playwright from this checkout's site/, else from the main checkout's (a worktree may not have run npm ci).
const site = [join(root, 'site'), join(root, '../../site')].find((d) => existsSync(join(d, 'node_modules/playwright'))) ?? join(root, 'site');
const { chromium } = createRequire(join(site, 'package.json'))('playwright');
const data = (file, type) => `data:${type};base64,${readFileSync(file).toString('base64')}`;
const icon = data(join(root, 'site/static/flockbook-light.svg'), 'image/svg+xml');

const notices = [
	['Needs you · Onboarding rework', 'meadow-onboard-dev · Meadow · onboarding', 'now'],
	['Needs you · Staging dry-run deploy', 'harbor-deploy-ops · Harbor · deploy', '2m ago'],
	['Needs you · Retry 429s with backoff', 'beacon-retry-dev · Beacon · retries', '9m ago'],
];

function page(look, phones) {
	const dark = look === 'dark';
	return `<!doctype html><meta charset="utf-8"><style>
	* { box-sizing: border-box; margin: 0; }
	html, body { width: 1280px; height: 852px; background: transparent; }
	body { display: flex; justify-content: center; align-items: center; gap: 72px;
		font-family: -apple-system, "SF Pro Text", "Helvetica Neue", sans-serif; -webkit-font-smoothing: antialiased; }
	.phone { width: 375px; height: 796px; padding: 11px; border-radius: 60px; background: ${dark ? '#2b2c30' : '#1d1d1f'};
		box-shadow: 0 0 0 1px ${dark ? 'rgba(255,255,255,.14)' : 'rgba(0,0,0,.25)'}, 0 24px 60px rgba(0,0,0,${dark ? '.45' : '.18'}); }
	.screen { position: relative; width: 353px; height: 774px; border-radius: 50px; overflow: hidden; background: #000; }
	.screen > img { display: block; position: absolute; top: 46px; width: 100%; }
	/* The status bar: the page's own top colour (set from the still), the time, and room for the island. */
	.bar { position: absolute; inset: 0 0 auto; height: 46px; padding: 15px 0 0 38px; font-size: 15px; font-weight: 600; }
	.island { position: absolute; top: 10px; left: 50%; width: 104px; height: 31px; margin-left: -52px; border-radius: 16px; background: #000; z-index: 2; }
	.lock { height: 100%; padding: 64px 12px 0; color: #fff;
		background: radial-gradient(120% 70% at 20% 10%, #6fa58a 0%, transparent 60%), radial-gradient(90% 60% at 90% 90%, #3e6d8f 0%, transparent 65%),
			linear-gradient(160deg, #2f5a4a, #1d3346 70%, #172433); }
	.date { text-align: center; font-size: 17px; font-weight: 600; opacity: .85; }
	.time { text-align: center; font-size: 84px; font-weight: 700; letter-spacing: -2px; line-height: 1; margin: 2px 0 28px; }
	.note { display: flex; gap: 10px; padding: 12px 13px; margin-bottom: 8px; border-radius: 22px; color: #111;
		background: rgba(245,245,247,.82); backdrop-filter: blur(20px); }
	.note img { width: 36px; height: 36px; flex: none; margin: -2px -2px 0; }
	.note div { min-width: 0; flex: 1; }
	.note header { display: flex; justify-content: space-between; font-size: 13px; color: #6b6b70; }
	.note b { display: block; font-size: 15px; font-weight: 600; margin-top: 1px; }
	.note p { font-size: 14px; color: #333; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
	</style>${phones.map((ph) => `<div class="phone"><div class="screen"><div class="island"></div>${ph}</div></div>`).join('')}
	<script>
	// Each still's screen takes the still's top-left colour, so the status bar runs into the page.
	window.ready = Promise.all([...document.querySelectorAll('.screen > img')].map((img) => img.decode().then(() => {
		const c = document.createElement('canvas'); c.width = c.height = 1;
		const g = c.getContext('2d'); g.drawImage(img, 4, 4, 1, 1, 0, 0, 1, 1);
		const [r, gr, b] = g.getImageData(0, 0, 1, 1).data;
		const screen = img.parentElement; screen.style.background = 'rgb(' + r + ',' + gr + ',' + b + ')';
		const bar = document.createElement('div'); bar.className = 'bar'; bar.textContent = '9:41';
		bar.style.color = r + gr + b > 384 ? '#000' : '#fff'; screen.append(bar);
	})));
	</script>`;
}

const lock = `<div class="lock"><p class="date">Tuesday 20 October</p><p class="time">9:41</p>${notices.map(([t, b, when]) =>
	`<section class="note"><img src="${icon}" alt=""><div><header><span>Flockbook</span><span>${when}</span></header><b>${t}</b><p>${b}</p></div></section>`).join('')}</div>`;

const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME_HEADLESS });
for (const look of ['light', 'dark']) {
	const shot = (name) => `<img src="${data(join(stills, `phone-${name}-${look}.png`), 'image/png')}" alt="">`;
	for (const [name, phones] of [['phone', [shot('list'), shot('pictures')]], ['push', [lock, shot('list')]]]) {
		const p = await browser.newPage({ viewport: { width: 1280, height: 852 }, deviceScaleFactor: 2 });
		await p.setContent(page(look, phones));
		await p.waitForLoadState('load');
		await p.evaluate(() => window.ready);
		await p.screenshot({ path: join(out, `${name}-${look}.png`), omitBackground: true });
		if (name === 'push') {
			// The lock screen alone, square and with no island, for a mockup tool that draws its own phone.
			await p.evaluate(() => { const s = document.querySelector('.screen'); s.style.borderRadius = '0'; s.querySelector('.island').remove(); });
			await p.locator('.screen').first().screenshot({ path: join(out, `lock-${look}.png`) });
			console.log(`lock-${look}.png`);
		}
		await p.close();
		console.log(`${name}-${look}.png`);
	}
}
await browser.close();
