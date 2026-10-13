const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const output = process.env.SDM_UI_OUTPUT || path.join(os.tmpdir(), 'sdm-extension-update-ui');
fs.mkdirSync(output, { recursive: true });
const base = process.env.SDM_UI_BASE || 'http://127.0.0.1:1433';
const { chromium } = require(process.env.SDM_SMOKE_PLAYWRIGHT || path.join(process.env.SDM_SMOKE_NODE_MODULES, 'playwright-core'));

(async () => {
  const browser = await chromium.launch({ executablePath: process.env.SDM_UI_BROWSER || '/usr/bin/chromium', headless: true, args: ['--no-sandbox'] });
  const report = [];
  try {
    for (const [lang, width, height] of [['en', 880, 560], ['vi', 660, 420]]) {
      const page = await browser.newPage({ viewport: { width, height } });
      page.setDefaultTimeout(10000);
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(`${base}/?lang=${lang}`);
      await page.locator('.idm-table').waitFor();
      await page.evaluate(async () => {
        const { api } = await import('/src/api.ts');
        window.calls = [];
        window.reportedVersion = '0.6.5';
        api.listBrowsers = async () => [
          { id: 'chrome', name: 'Google Chrome', engine: 'chromium', connected: true, extensionSeen: true, extensionVersion: window.reportedVersion, availableExtensionVersion: '0.6.6', extensionUpdateAvailable: window.reportedVersion !== '0.6.6' },
          { id: 'edge', name: 'Microsoft Edge', engine: 'chromium', connected: false, extensionSeen: true, extensionVersion: '0.6.6', availableExtensionVersion: '0.6.6', extensionUpdateAvailable: false },
        ];
        api.installExtension = async id => { window.calls.push(`install:${id}`); throw new Error('Existing extensions must not be installed twice'); };
        api.updateExtension = async id => {
          window.calls.push(`update:${id}`);
          return { method: 'automatic', updating: true, browser: 'Google Chrome', engine: 'chromium', extensionDir: '/fixture/SDM/extension/chromium', extensionsPage: 'chrome://extensions' };
        };
      });
      await page.getByRole('button', { name: lang === 'en' ? 'Browsers' : 'Trình duyệt', exact: true }).click();
      const dialog = page.getByRole('dialog', { name: lang === 'en' ? 'Browser integration' : 'Tích hợp trình duyệt', exact: true });
      const chrome = dialog.locator('.browser').filter({ hasText: 'Google Chrome' });
      const edge = dialog.locator('.browser').filter({ hasText: 'Microsoft Edge' });
      const update = lang === 'en' ? 'Update extension' : 'Cập nhật tiện ích';
      const current = lang === 'en' ? 'Up to date' : 'Đã cập nhật';
      await chrome.getByRole('button', { name: update, exact: true }).waitFor();
      assert(await edge.getByText(lang === 'en' ? 'Extension recognized; browser not connected' : 'Đã nhận diện tiện ích; trình duyệt chưa kết nối', { exact: true }).isVisible());
      assert(await edge.getByRole('button', { name: current, exact: true }).isDisabled());
      await chrome.getByRole('button', { name: update, exact: true }).click();
      await dialog.locator('.steps').waitFor();
      assert.deepEqual(await page.evaluate(() => window.calls), ['update:chrome']);
      assert(await chrome.getByText(/v0\.6\.5/).isVisible());
      await page.waitForTimeout(3200);
      assert(await dialog.locator('.steps').isVisible());
      assert.equal(await dialog.getByText('Load unpacked', { exact: true }).count(), 0);
      const footer = await dialog.locator('.dialog-footer').boundingBox();
      assert(footer.y + footer.height <= height);
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await page.screenshot({ path: path.join(output, `extension-update-${lang}.png`) });
      await page.evaluate(() => window.reportedVersion = '0.6.6');
      await chrome.getByRole('button', { name: current, exact: true }).waitFor();
      assert(await chrome.getByRole('button', { name: current, exact: true }).isDisabled());
      assert(await chrome.getByText(/v0\.6\.6/).isVisible());
      assert.equal(await dialog.locator('.steps').count(), 0);
      assert.deepEqual(errors, []);
      report.push({ lang, width, height, disconnectedRecognized: true, noDuplicateInstall: true, versionConfirmedByHeartbeat: true, footerVisible: true });
      await page.close();
    }
    fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
