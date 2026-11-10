// Phone stills of the helper's phone page (flockbook-modfeed serve) on the herdr-core demo: scripts/demo-hc.sh seed, then
// scripts/demo-hc.sh serve (127.0.0.1:8797). Headless Chromium, no window, no phone, no Tailscale change: a small proxy
// on 127.0.0.1 stands in for Tailscale Serve and adds what it would (the identity header, the page's https Origin).
// The page may only read: the proxy refuses every POST (approve, answer, send) and the run then fails, and the socket
// may carry only reads.
//   node scripts/demo/phone-stills-hc.mjs <out-dir> [--port 8797] [--user LOGIN]
// -> <out>/phone-<shot>-light.png and -dark.png at 390x805 (a 390x844 screen under its status bar), pixel ratio 3.
// Shots: list (every agent), conversation (a reply with a table and code), question (a question card), approval
// (an approval card), pictures (screenshots in a conversation). phone-frames.mjs takes list and pictures.
import { execFileSync } from 'node:child_process';
import http from 'node:http';
import net from 'node:net';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '../..');
const args = process.argv.slice(2);
const opt = (name, fallback) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : fallback; };
const out = args.find((a, i) => !a.startsWith('--') && !args[i - 1]?.startsWith('--')) ?? 'build/phone-stills-hc';
const port = opt('--port', '8797');
const user = opt('--user', null) ?? JSON.parse(execFileSync('tailscale', ['status', '--json']).toString()).User[
	String(JSON.parse(execFileSync('tailscale', ['status', '--json']).toString()).Self.UserID)].LoginName;
mkdirSync(out, { recursive: true });
if (/"CGSSessionScreenIsLocked"\s*=\s*Yes/.test(execFileSync('/usr/sbin/ioreg', ['-n', 'Root', '-d1']).toString())) {
	console.log('phone-stills-hc: the screen is locked; not starting');
	process.exit(3);
}
// Playwright from this checkout's site/, else from the main checkout's (a worktree may not have run npm ci).
const site = [join(root, 'site'), join(root, '../../site')].find((d) => existsSync(join(d, 'node_modules/playwright')));
if (!site) { console.log('phone-stills-hc: no playwright (cd site && npm ci)'); process.exit(2); }
const { chromium } = createRequire(join(site, 'package.json'))('playwright');

// Tailscale Serve, as far as the helper sees it: Host is the helper's, Origin is https on that host, the login is set.
const posted = [];
const upstream = (h) => ({ ...h, host: `127.0.0.1:${port}`, 'tailscale-user-login': user,
	...(h.origin ? { origin: `https://127.0.0.1:${port}` } : {}) });
const proxy = http.createServer((req, res) => {
	if (req.method !== 'GET' && req.method !== 'HEAD') { posted.push(`${req.method} ${req.url}`); res.writeHead(403).end(); return; }
	const up = http.request({ host: '127.0.0.1', port, method: req.method, path: req.url, headers: upstream(req.headers) }, (r) => {
		res.writeHead(r.statusCode, r.headers);
		r.pipe(res);
	});
	up.on('error', () => res.writeHead(502).end());
	req.pipe(up);
});
proxy.on('upgrade', (req, sock, head) => {
	const up = net.connect(Number(port), '127.0.0.1', () => {
		const lines = Object.entries(upstream(req.headers)).map(([k, v]) => `${k}: ${v}`);
		up.write(`${req.method} ${req.url} HTTP/1.1\r\n${lines.join('\r\n')}\r\n\r\n`);
		up.write(head);
		sock.pipe(up).pipe(sock);
	});
	up.on('error', () => sock.destroy());
	sock.on('error', () => up.destroy());
});
await new Promise((r) => proxy.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${proxy.address().port}`;

let failures = 0;
const step = (n, ok) => { if (!ok) failures++; console.log(`phone-stills-hc: ${n} ${ok ? 'ok' : 'FAIL'}`); };
// The newest chrome-headless-shell in Playwright's cache (as web-test.mjs finds it), else Playwright's own.
function headlessShell() {
	const cache = join(homedir(), 'Library/Caches/ms-playwright');
	for (const d of existsSync(cache) ? readdirSync(cache).filter((d) => d.startsWith('chromium_headless_shell-')).sort().reverse() : [])
		for (const sub of readdirSync(join(cache, d))) {
			const p = join(cache, d, sub, 'chrome-headless-shell');
			if (existsSync(p)) return p;
		}
	return undefined;
}
const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME_HEADLESS ?? headlessShell() });
try {
	for (const look of ['light', 'dark']) {
		const ctx = await browser.newContext({ viewport: { width: 390, height: 805 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true,
			colorScheme: look });
		const p = await ctx.newPage();
		await p.addInitScript(() => {
			const reads = new Set(['hello', 'watch', 'unwatch', 'conversation', 'loadEarlier', 'picture', 'pong', 'screen']);
			const send = WebSocket.prototype.send; window.__dropped = [];
			WebSocket.prototype.send = function (d) {
				let type; try { type = JSON.parse(d).type; } catch { type = '(not JSON)'; }
				if (!reads.has(type)) { window.__dropped.push(type); return; }
				return send.call(this, d);
			};
		});
		await p.goto(origin + '/');
		step(`${look}: agent list`, await p.waitForSelector('.list', { timeout: 15000 }).then(() => true, () => false));
		await p.waitForTimeout(2500);
		const shot = async (name) => { await p.waitForTimeout(700); await p.screenshot({ path: join(out, `phone-${name}-${look}.png`) }); };
		await shot('list');
		// A pane by its row's words: open it, wait for `ready`.
		const open = async (words, ready) => {
			await p.evaluate(() => { location.hash = ''; });
			await p.waitForSelector('.list', { timeout: 10000 });
			await p.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
			await p.waitForTimeout(800);
			const id = await p.evaluate((w) => [...document.querySelectorAll('button[data-pane]')].find((b) => b.innerText.includes(w))?.dataset.pane, words);
			step(`${look}: row "${words}"`, !!id);
			if (!id) return false;
			await p.evaluate((i) => { location.hash = '#pane=' + encodeURIComponent(i); }, id);
			return p.waitForFunction(ready, null, { timeout: 15000 }).then(() => true, () => false);
		};
		// The last `sel`, with the block before it, just under the top bar (a block edge, so no line is cut there).
		// After the page has stopped keeping the newest message in view (it does while the conversation loads).
		const scrollTo = async (sel) => { await p.waitForTimeout(1500); await p.evaluate((s) => {
			const e = [...document.querySelectorAll(s)].at(-1);
			const top = e && (e.closest('.table-wrap, .scroll-x') ?? e);
			if (top) window.scrollBy(0, (top.previousElementSibling ?? top).getBoundingClientRect().top - 76);
		}, sel); };
		// A finished conversation: the cause in a table, then the fix as code.
		if (await open('Fix rounding', () => document.querySelectorAll('.msg').length > 3 && !!document.querySelector('table'))) {
			await scrollTo('table');
			await shot('conversation');
		} else step(`${look}: conversation`, false);
		if (await open('Promo codes', () => !!document.querySelector('.question, .prompt-card, [data-question]') || /Recommended/.test(document.body.innerText))) {
			await p.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
			await shot('question');
		} else step(`${look}: question card`, false);
		if (await open('Staging dry-run', () => !!document.querySelector('pre.command'))) {
			await p.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
			await shot('approval');
		} else step(`${look}: approval card`, false);
		if (await open('Harvest card', () => document.querySelectorAll('button.picture').length >= 2)) {
			// The person's message with its screenshot, just under the top bar.
			await p.waitForTimeout(1500);
			await p.evaluate(() => { const m = document.querySelector('.msg.user'); if (m) window.scrollBy(0, m.getBoundingClientRect().top - 76); });
			step(`${look}: pictures shown`, await p.waitForFunction(() => [...document.querySelectorAll('button.picture img')]
				.filter((i) => i.complete && i.naturalWidth > 0).length >= 1, null, { timeout: 10000 }).then(() => true, () => false));
			await shot('pictures');
		} else step(`${look}: pictures`, false);
		const dropped = await p.evaluate(() => window.__dropped);
		step(`${look}: the page sent only reads (socket dropped: ${JSON.stringify(dropped)}, posts: ${posted.length})`,
			dropped.length === 0 && posted.length === 0);
		await ctx.close();
	}
} finally {
	await browser.close();
	proxy.close();
	proxy.closeAllConnections?.();
}
console.log(`phone-stills-hc: ${failures ? `FAIL x${failures}` : 'PASS'}`);
process.exit(failures ? 1 : 0);
