// Exercise Chrome's real download events with the packaged MV3 extension.
// Fixtures resemble GitHub redirects; two public SDM release assets also
// verify the live GitHub response. No downloaded file is executed.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const { chromium } = require(process.env.SDM_SMOKE_PLAYWRIGHT || path.join(process.env.SDM_SMOKE_NODE_MODULES, 'playwright-core'));
const root = process.cwd();
const extension = path.join(root, 'extension/dist/chromium');
const output = process.env.SDM_SMOKE_OUTPUT || path.join(root, 'download-smoke');
fs.mkdirSync(output, { recursive: true });
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'sdm-download-smoke-'));
const executablePath = process.env.SDM_SMOKE_BROWSER || [process.env.ProgramFiles, process.env['ProgramFiles(x86)'], process.env.LOCALAPPDATA]
  .filter(Boolean).map(p => path.join(p, 'Google/Chrome/Application/chrome.exe')).find(p => fs.existsSync(p));
assert(executablePath, 'Google Chrome is required for download capture checks.');
const calls = [], errors = [], report = [];
let accepted = true, enabled = true;
const streams = new Set();
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://fixture');
  if (url.pathname === '/api/ping') {
    res.setHeader('Content-Type', 'application/json');
    return res.end(JSON.stringify({ enabled, captureExtensions: ['exe', 'zip'], version: 'fixture' }));
  }
  if (url.pathname === '/api/download') {
    let raw = '';
    for await (const chunk of req) raw += chunk;
    calls.push({ body: JSON.parse(raw), client: req.headers['x-sdm-client'] });
    res.statusCode = accepted ? 202 : 503;
    return res.end();
  }
  if (url.pathname.startsWith('/owner/repo/releases/download/v1/')) {
    res.writeHead(302, { Location: `/release-assets/uuid?name=${encodeURIComponent(url.pathname.split('/').at(-1))}` });
    return res.end();
  }
  if (url.pathname === '/release-assets/uuid' || url.pathname === '/download/opaque') {
    const name = url.searchParams.get('name') || 'opaque.zip';
    // Even a very small ZIP must be captured before Chrome completes it.
    const bytes = name.endsWith('.exe') ? 1024 * 128 : 512;
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Disposition': `attachment; filename="${name}"`, 'Content-Length': bytes });
    res.write(Buffer.alloc(64));
    const timer = setTimeout(() => res.end(Buffer.alloc(bytes - 64)), 1200);
    streams.add(timer);
    res.on('close', () => { clearTimeout(timer); streams.delete(timer); });
    return;
  }
  res.setHeader('Content-Type', 'text/html');
  res.end('<!doctype html><html><head><title>Download fixture</title></head><body><a id="download">Download asset</a></body></html>');
});
async function until(fn, message, timeout = 20_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await fn()) return;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(message);
}

(async () => {
  let context, popup, page;
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const origin = `http://127.0.0.1:${server.address().port}`;
    context = await chromium.launchPersistentContext(profile, {
      executablePath, headless: true, acceptDownloads: true,
      ignoreDefaultArgs: ['--disable-extensions'],
      args: ['--no-sandbox', '--lang=en-US', '--enable-unsafe-extension-debugging'],
    });
    const loader = await context.browser().newBrowserCDPSession();
    const installed = await loader.send('Extensions.loadUnpacked', { path: extension });
    popup = await context.newPage();
    await popup.goto(`chrome-extension://${installed.id}/popup.html`);
    await popup.evaluate(async port => {
      await chrome.storage.local.set({ port });
      await chrome.runtime.sendMessage({ type: 'status' });
    }, server.address().port);
    const downloads = () => popup.evaluate(() => chrome.downloads.search({}));
    const configure = values => popup.evaluate(async values => {
      await chrome.storage.local.set(values);
      await chrome.runtime.sendMessage({ type: 'status' });
    }, values);
    page = await context.newPage();
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(origin);
    async function click(url) {
      await page.locator('#download').evaluate((a, url) => a.href = url, url);
      await page.locator('#download').click();
    }
    async function captured(url, filename, name) {
      const before = calls.length;
      await click(url);
      await until(() => calls.length === before + 1, `SDM did not receive ${name}`);
      assert.equal(calls.at(-1).body.url, url, 'The original request URL must survive redirects');
      assert.equal(calls.at(-1).body.fileName, filename);
      assert.equal(calls.at(-1).client, 'extension');
      await until(async () => !(await downloads()).some(d => d.url === url && d.state === 'in_progress'), `${name} remained active in Chrome`);
      assert(!(await downloads()).some(d => d.url === url && d.state === 'complete'), `${name} also completed in Chrome`);
      report.push({ name, passed: true });
    }
    await captured(`${origin}/owner/repo/releases/download/v1/setup.exe`, 'setup.exe', 'redirected-exe');
    await captured(`${origin}/owner/repo/releases/download/v1/bundle.zip`, 'bundle.zip', 'redirected-small-zip');
    await captured(`${origin}/download/opaque`, 'opaque.zip', 'filename-from-content-disposition');

    await configure({ capture: false });
    const browserUrl = `${origin}/owner/repo/releases/download/v1/browser.zip`;
    const beforeDisabled = calls.length;
    await click(browserUrl);
    await until(async () => (await downloads()).some(d => d.url === browserUrl && d.state === 'complete'), 'Disabled capture did not leave the download in Chrome');
    assert.equal(calls.length, beforeDisabled);
    report.push({ name: 'capture-disabled-stays-in-chrome', passed: true });
    await configure({ capture: true });

    accepted = false;
    const fallbackUrl = `${origin}/owner/repo/releases/download/v1/fallback.zip`;
    const beforeFallback = calls.length;
    await click(fallbackUrl);
    await until(async () => (await downloads()).some(d => d.url === fallbackUrl && d.state === 'complete'), 'Rejected handover was not returned to Chrome');
    assert.equal(calls.length, beforeFallback + 1, 'Fallback must not loop through SDM');
    report.push({ name: 'rejected-handover-browser-fallback', passed: true });
    accepted = true;

    enabled = false;
    await configure({});
    const disabledUrl = `${origin}/owner/repo/releases/download/v1/disabled.zip`;
    const beforeOffline = calls.length;
    await click(disabledUrl);
    await until(async () => (await downloads()).some(d => d.url === disabledUrl && d.state === 'complete'), 'Disabled app integration did not leave the file in Chrome');
    assert.equal(calls.length, beforeOffline);
    report.push({ name: 'app-integration-disabled-stays-in-chrome', passed: true });
    enabled = true;
    await configure({});

    const release = 'https://github.com/10s3r2604/supodownloadmanager/releases/download/v0.6.0/';
    for (const name of ['sdm-extension-chromium-v0.6.0.zip', 'Supo.Download.Manager_0.6.0_x64-setup.exe']) {
      await captured(release + name, name, `live-github-${name.endsWith('.exe') ? 'exe' : 'zip'}`);
    }
    assert.deepEqual(errors, []);
    const result = { browser: 'Chrome', passed: report.length, report, errors };
    fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result, null, 2));
  } catch (error) {
    console.error('Handover count:', calls.length);
    if (popup) console.error('Download states:', await popup.evaluate(async () => (await chrome.downloads.search({})).map(d => ({ state: d.state, error: d.error, name: d.filename.split(/[\\/]/).at(-1), host: new URL(d.url).hostname }))).catch(e => String(e)));
    if (page) await page.screenshot({ path: path.join(output, 'failure.png'), timeout: 3000 }).catch(() => {});
    fs.writeFileSync(path.join(output, 'failure.txt'), error.stack || String(error));
    throw error;
  } finally {
    if (context) await context.close();
    for (const timer of streams) clearTimeout(timer);
    server.closeAllConnections(); server.close();
    fs.rmSync(profile, { recursive: true, force: true });
  }
})().catch(error => { console.error(error); process.exit(1); });
