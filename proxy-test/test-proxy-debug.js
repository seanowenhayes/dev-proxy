const { firefox } = require('@playwright/test');
const path = require('path');
const fs = require('fs');
const { execSync } = require('child_process');

async function main() {
  const PROXY_PORT = 3003;
  const CA_CERT_PATH = path.join(__dirname, '..', 'dev-proxy-ca.pem');

  const tempProfile = fs.mkdtempSync(path.join(require('os').tmpdir(), 'devproxy-firefox-'));
  fs.writeFileSync(path.join(tempProfile, 'prefs.js'), 'user_pref("network.proxy.type", 0);\n');
  execSync(`certutil -A -n "dev-proxy-mitm-ca" -t "CT,C,C" -i "${CA_CERT_PATH}" -d "sql:${tempProfile}"`, { stdio: 'inherit' });

  console.log('\n=== Launching Firefox with proxy ===\n');

  const context = await firefox.launchPersistentContext(tempProfile, {
    headless: true,
    proxy: { server: `http://127.0.0.1:${PROXY_PORT}` },
    firefoxUserPrefs: {
      'network.proxy.allow_hijacking_localhost': true,
      'security.enterprise_roots.enabled': true,
    },
  });

  const page = await context.newPage();
  page.on('request', req => console.log(`  [REQ] ${req.method()} ${req.url()}`));
  page.on('response', async resp => {
    const len = resp.headers()['content-length'];
    console.log(`  [RES] ${resp.status()} ${resp.url()} (CL: ${len})`);
  });
  page.on('requestfailed', req => console.log(`  [FAIL] ${req.failure()?.errorText} ${req.url()}`));

  console.log('Test: https://example.com');
  try {
    const resp = await page.goto('https://example.com', { timeout: 10000, waitUntil: 'commit' });
    console.log(`  Response status: ${resp?.status()}`);
    
    // Wait a bit then check content
    await page.waitForTimeout(2000);
    const body = await page.evaluate(() => document.body?.innerText);
    console.log(`  Body text: "${body}"`);
    console.log(`  SUCCESS`);
  } catch (err) {
    console.log(`  FAILED: ${err.message.split('\n')[0]}`);
  }

  await context.close();
  console.log('\n=== Tests complete ===');
}

main().catch(err => {
  console.error('Unhandled error:', err);
  process.exit(1);
});
