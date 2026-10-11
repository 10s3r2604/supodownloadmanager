// Load the built MV3 extension in a real browser. X and the SDM bridge are
// deterministic fixtures; no external video service or login is required.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const { chromium } = require(process.env.SDM_SMOKE_PLAYWRIGHT || path.join(process.env.SDM_SMOKE_NODE_MODULES, 'playwright-core'));
const root = process.cwd();
const extension = path.join(root, 'extension/dist/chromium');
const output = process.env.SDM_SMOKE_OUTPUT || path.join(root, 'progress-smoke/extension');
fs.mkdirSync(output, { recursive: true });
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'sdm-extension-smoke-'));
const candidates = process.platform === 'win32'
  ? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles].filter(Boolean).map(p => path.join(p, 'Microsoft/Edge/Application/msedge.exe'))
  : ['/usr/bin/chromium', '/usr/bin/google-chrome'];
const executablePath = process.env.SDM_SMOKE_BROWSER || candidates.find(p => fs.existsSync(p));
assert(executablePath, 'A Chromium browser is required for the extension smoke test.');
const requests = [], errors = [], report = [];
let failProbe = false;
const options = [
  { id: 'v1080', kind: 'video', height: 1080, fps: 60, ext: 'mp4', vcodec: 'H.264', acodec: 'AAC', size: 10 * 1024 ** 2 },
  { id: 'v720', kind: 'video', height: 720, fps: 30, ext: 'mp4', vcodec: 'H.264', acodec: 'AAC', size: 6 * 1024 ** 2 },
  { id: 'mp3', kind: 'audio', ext: 'mp3', size: null },
];
const server = http.createServer(async (req, res) => {
  let raw = '';
  for await (const part of req) raw += part;
  const body = raw ? JSON.parse(raw) : null;
  requests.push({ path: req.url, body, client: req.headers['x-sdm-client'] });
  res.setHeader('Content-Type', 'application/json');
  if (req.url === '/api/ping') return res.end(JSON.stringify({ enabled: true, media: true, version: 'fixture' }));
  if (req.url === '/api/media/probe') {
    if (!body.wait) { res.statusCode = 202; return res.end(); }
    if (failProbe) { res.statusCode = 422; return res.end('Fixture video needs a login'); }
    return res.end(JSON.stringify({ title: `Clip ${body.pageUrl.split('/').at(-1)}`, options }));
  }
  if (req.url === '/api/media/add') return res.end(JSON.stringify({ id: 42 }));
  res.statusCode = 500;
  res.end('The desktop picker route must not be called.');
});
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn, message, timeout = 15_000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try { const result = await fn(); if (result) return result; } catch (e) { last = e; }
    await sleep(100);
  }
  throw new Error(`${message}${last ? `: ${last.message}` : ''}`);
}
function attributes(node) {
  const values = node.attributes || [];
  return Object.fromEntries(Array.from({ length: values.length / 2 }, (_, i) => [values[i * 2], values[i * 2 + 1]]));
}
function fixture(content) {
  return `<!doctype html><html><head><title>Home / X</title><style>body{margin:24px;background:#17202a;color:white;font:16px system-ui}article,[role=article],[data-testid=cellInnerDiv]{width:520px;margin-bottom:28px;border:1px solid #536471;padding:16px}video{display:block;width:500px;height:260px;background:#263e50}a{color:#8bd7c7}</style></head><body>${content}</body></html>`;
}

(async () => {
  let context, worker, page, cdp;
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    context = await chromium.launchPersistentContext(profile, {
      executablePath, headless: true, viewport: { width: 1000, height: 800 },
      ignoreDefaultArgs: ['--disable-extensions'],
      args: ['--no-sandbox', '--lang=en-US', '--enable-unsafe-extension-debugging', `--disable-extensions-except=${extension}`, `--load-extension=${extension}`],
    });
    // Current Chromium disables --load-extension in regular builds. The
    // explicit extension-debugging API loads this test bundle in isolation.
    const loader = await context.browser().newBrowserCDPSession();
    await loader.send('Extensions.loadUnpacked', { path: extension }).catch(error => {
      if (!context.serviceWorkers().length) throw error;
    });
    worker = context.serviceWorkers().find(w => w.url().endsWith('/background.js'))
      || await context.waitForEvent('serviceworker', { predicate: w => w.url().endsWith('/background.js'), timeout: 15_000 });
    await worker.evaluate(async port => {
      await chrome.storage.local.set({ port });
      await ping(true);
    }, server.address().port);
    page = await context.newPage();
    page.on('pageerror', error => errors.push(error.message));
    let html = fixture('<article><a href="/a/status/111"><time>First post</time></a><video></video></article><div role="article"><a href="/b/status/222/video/2"><time>Second post</time></a><video></video></div>');
    await context.route('https://x.com/**', route => route.fulfill({ status: 200, contentType: 'text/html', body: html }));
    await page.goto('https://x.com/home');
    cdp = await context.newCDPSession(page);
    async function nodeWhere(predicate) {
      const { root } = await cdp.send('DOM.getDocument', { depth: -1, pierce: true });
      function find(node) {
        if (predicate(node, attributes(node))) return node;
        for (const child of [...(node.children || []), ...(node.shadowRoots || [])]) {
          const found = find(child); if (found) return found;
        }
      }
      return find(root);
    }
    async function textOf(node) {
      const { object } = await cdp.send('DOM.resolveNode', { nodeId: node.nodeId });
      try {
        const { result } = await cdp.send('Runtime.callFunctionOn', { objectId: object.objectId, functionDeclaration: 'function(){return this.innerText}', returnByValue: true });
        return result.value;
      } finally { await cdp.send('Runtime.releaseObject', { objectId: object.objectId }); }
    }
    async function clickNode(predicate) {
      const node = await until(() => nodeWhere(predicate), 'Extension menu button was not found');
      const { model } = await cdp.send('DOM.getBoxModel', { nodeId: node.nodeId });
      await page.mouse.click((model.border[0] + model.border[4]) / 2, (model.border[1] + model.border[5]) / 2);
    }
    async function menuText() {
      const node = await nodeWhere((n, a) => a.class === 'menu' && !('hidden' in a));
      return node ? textOf(node) : '';
    }
    async function hover(index) {
      const video = page.locator('video').nth(index);
      await video.scrollIntoViewIfNeeded();
      const box = await video.boundingBox();
      let move = 0;
      await until(async () => {
        // document_idle may register the listener after the first synthetic
        // mousemove. Keep moving inside the player until the button appears.
        await page.mouse.move(box.x + 80 + (move++ % 2), box.y + 60);
        return nodeWhere((n, a) => a.class === 'trigger');
      }, 'The video button did not appear');
    }
    async function open(index) {
      await hover(index);
      await clickNode((n, a) => a.class === 'trigger');
      await until(async () => (await menuText()).includes('1080p60'), 'Quality menu did not load');
    }
    const adds = () => requests.filter(r => r.path === '/api/media/add');
    const probes = () => requests.filter(r => r.path === '/api/media/probe');

    await open(0);
    const labels = await menuText();
    assert(labels.includes('~10 MB') && labels.includes('~6 MB') && labels.includes('Size unknown'));
    assert(labels.includes('H.264') && labels.includes('AAC') && labels.includes('MP4'));
    assert(!labels.includes('More options in SDM'));
    assert.equal(adds().length, 0, 'Opening qualities must not create a download');
    await page.screenshot({ path: path.join(output, 'x-feed-qualities.png') });
    await clickNode((n, a) => a['aria-label']?.startsWith('720p'));
    await until(() => adds().length === 1, 'The selected quality did not start');
    assert.equal(adds()[0].body.source.pageUrl, 'https://x.com/a/status/111');
    assert.equal(adds()[0].body.option, 'v720');
    report.push({ name: 'feed-quality-size-and-selected-download', passed: true });
    await page.keyboard.press('Escape');

    await open(1);
    await clickNode((n, a) => a['aria-label']?.startsWith('1080p60'));
    await until(() => adds().length === 2, 'Second feed video did not start');
    assert.equal(adds()[1].body.source.pageUrl, 'https://x.com/b/status/222/video/2');
    assert.equal(adds()[1].body.option, 'v1080');
    report.push({ name: 'second-feed-post-and-clip-index', passed: true });
    await page.keyboard.press('Escape');

    failProbe = true;
    await hover(0);
    await clickNode((n, a) => a.class === 'trigger');
    await until(async () => (await menuText()).includes('Fixture video needs a login'), 'Discovery failure was not visible');
    failProbe = false;
    await clickNode((n, a) => a.class === 'row more');
    await until(async () => (await menuText()).includes('1080p60'), 'Inline retry did not recover');
    assert.equal(adds().length, 2);
    report.push({ name: 'inline-error-and-retry', passed: true });
    await page.keyboard.press('Escape');

    // Send the context menu's frame message through the actual service
    // worker/content-script channel, preserving the right-clicked player.
    await page.locator('video').nth(0).click({ button: 'right' });
    await page.keyboard.press('Escape');
    await worker.evaluate(async () => {
      const [tab] = await chrome.tabs.query({ url: 'https://x.com/home' });
      await chrome.tabs.sendMessage(tab.id, { type: 'video-open-menu' }, { frameId: 0 });
    });
    await until(async () => (await menuText()).includes('1080p60'), 'Context-menu quality selection did not open');
    report.push({ name: 'context-menu-opens-in-page', passed: true });
    await page.keyboard.press('Escape');

    await page.setViewportSize({ width: 320, height: 360 });
    await open(0);
    const narrowMenu = await nodeWhere((n, a) => a.class === 'menu' && !('hidden' in a));
    const { model: narrowBox } = await cdp.send('DOM.getBoxModel', { nodeId: narrowMenu.nodeId });
    assert(Math.min(narrowBox.border[0], narrowBox.border[6]) >= -0.5);
    assert(Math.max(narrowBox.border[2], narrowBox.border[4]) <= 320.5);
    assert(narrowBox.border[1] >= -0.5 && narrowBox.border[5] <= 360.5);
    await page.screenshot({ path: path.join(output, 'x-feed-narrow-menu.png') });
    report.push({ name: 'narrow-menu-stays-in-viewport', passed: true });
    await page.keyboard.press('Escape');

    html = fixture('<div data-testid="cellInnerDiv"><video></video></div>');
    await page.reload();
    const before = probes().length;
    await hover(0);
    await clickNode((n, a) => a.class === 'trigger');
    await until(async () => (await menuText()).includes('post link is not available'), 'Unresolved post did not stay in the extension');
    assert.equal(probes().length, before, 'An unresolved feed must not reach yt-dlp');
    report.push({ name: 'unresolved-post-stays-inline', passed: true });

    assert(probes().every(r => /\/status\/\d+/.test(r.body.pageUrl)));
    assert(requests.every(r => r.path !== '/api/media'), 'The desktop picker must never open for X');
    assert(requests.every(r => r.client === 'extension'));
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify({ passed: report.length, report, errors }, null, 2));
    console.log(JSON.stringify({ passed: report.length, report, errors }, null, 2));
  } catch (error) {
    console.error('Bridge fixture requests:', JSON.stringify(requests.map(r => ({ path: r.path, pageUrl: r.body?.pageUrl || r.body?.source?.pageUrl, wait: r.body?.wait }))));
    if (worker) console.error('Extension status:', await worker.evaluate(async () => ({ version: chrome.runtime.getManifest().version, prefs: await prefs(), status: await ping(true) })).catch(e => String(e)));
    if (page) {
      console.error('Fixture DOM:', await page.evaluate(() => ({ url: location.href, videos: document.querySelectorAll('video').length, button: Boolean(document.querySelector('sdm-video-button')) })).catch(e => String(e)));
      await page.screenshot({ path: path.join(output, 'failure.png'), timeout: 3000 }).catch(() => {});
    }
    if (cdp) {
      const snapshot = await cdp.send('DOMSnapshot.captureSnapshot', { computedStyles: [] }).catch(() => null);
      if (snapshot) console.error('Fixture labels:', snapshot.strings.filter(s => /SDM|qualities|video|1080|720|Retry/.test(s)));
    }
    throw error;
  } finally {
    if (context) await context.close();
    server.closeAllConnections(); server.close();
    fs.rmSync(profile, { recursive: true, force: true });
  }
})().catch(error => { console.error(error); process.exit(1); });
