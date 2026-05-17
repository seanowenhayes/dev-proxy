const { firefox } = require('@playwright/test');
const path = require('path');
const fs = require('fs');
const { execSync } = require('child_process');

async function main() {
  const PROXY_PORT = 3003;
  const CA_CERT_PATH = path.join(__dirname, '..', 'dev-proxy-ca.pem');

  console.log('CA cert exists:', fs.existsSync(CA_CERT_PATH));

  // Create temp profile and import CA cert
  const tempProfile = fs.mkdtempSync(path.join(require('os').tmpdir(), 'devproxy-firefox-'));
  fs.writeFileSync(path.join(tempProfile, 'prefs.js'), 'user_pref("network.proxy.type", 0);\n');
  execSync(`certutil -A -n "dev-proxy-mitm-ca" -t "CT,C,C" -i "${CA_CERT_PATH}" -d "sql:${tempProfile}"`, { stdio: 'inherit' });
  console.log('Created temp profile:', tempProfile);

  console.log('\n=== Launching Firefox with proxy (CA cert imported) ===\n');

  const context = await firefox.launchPersistentContext(tempProfile, {
    headless: true,
    proxy: {
      server: `http://127.0.0.1:${PROXY_PORT}`,
    },
    firefoxUserPrefs: {
      'network.proxy.allow_hijacking_localhost': true,
      'network.dns.disablePrefetch': true,
      'network.dns.disablePrefetchFromHTTPS': true,
      'security.enterprise_roots.enabled': true,
      'browser.download.folderList': 2,
      'browser.startup.homepage': 'about:blank',
    },
  });

  const page = await context.newPage();

  page.on('request', req => console.log(`  [REQ] ${req.method()} ${req.url()}`));
  page.on('response', resp => console.log(`  [RES] ${resp.status()} ${resp.url()}`));
  page.on('requestfailed', req => console.log(`  [FAIL] ${req.failure()?.errorText} ${req.url()}`));

  const urls = [
    'http://example.com',
    'https://example.com',
    'https://httpbin.org/get',
    'https://www.google.com',
  ];

  for (const url of urls) {
    console.log(`\nTest: ${url}`);
    try {
      await page.goto(url, { timeout: 15000, waitUntil: 'domcontentloaded' });
      const title = await page.title();
      const content = await page.content();
      const hasBody = content.includes('<body') || content.includes('<BODY');
      console.log(`  SUCCESS - Title: "${title}", Has body: ${hasBody}`);
    } catch (err) {
      console.log(`  FAILED: ${err.message.split('\n')[0]}`);
    }
  }

  await context.close();
  console.log('\n=== Tests complete ===');
}

main().catch(err => {
  console.error('Unhandled error:', err);
  process.exit(1);
});
