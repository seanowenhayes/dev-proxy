const { firefox } = require('@playwright/test');
const path = require('path');
const fs = require('fs');

async function main() {
  const PROXY_PORT = 3003;
  const CA_CERT_PATH = path.join(__dirname, '..', 'dev-proxy-ca.pem');
  
  console.log('CA cert exists:', fs.existsSync(CA_CERT_PATH));

  console.log('\n=== Launching Firefox with proxy ===\n');

  const browser = await firefox.launch({
    headless: true,
    proxy: {
      server: `http://127.0.0.1:${PROXY_PORT}`,
    },
    firefoxUserPrefs: {
      'security.enterprise_roots.enabled': true,
      'security.cert_pinning.enforcement_level': 0,
      'network.proxy.allow_hijacking_localhost': true,
      'network.dns.disablePrefetch': true,
      'network.dns.disablePrefetchFromHTTPS': true,
    },
  });

  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
  });

  const page = await context.newPage();

  // Log all requests
  page.on('request', req => console.log(`  [REQ] ${req.method()} ${req.url()}`));
  page.on('response', resp => console.log(`  [RES] ${resp.status()} ${resp.url()}`));
  page.on('requestfailed', req => console.log(`  [FAIL] ${req.failure()?.errorText} ${req.url()}`));

  // Test 1: HTTP
  console.log('Test 1: HTTP http://example.com');
  try {
    await page.goto('http://example.com', { timeout: 10000 });
    const title = await page.title();
    console.log(`  SUCCESS - Title: "${title}"`);
  } catch (err) {
    console.log(`  FAILED: ${err.message.split('\n')[0]}`);
  }

  // Test 2: HTTPS httpbin
  console.log('\nTest 2: HTTPS https://httpbin.org/get');
  try {
    const resp = await page.goto('https://httpbin.org/get', { timeout: 10000 });
    console.log(`  SUCCESS - Status: ${resp?.status()}`);
  } catch (err) {
    console.log(`  FAILED: ${err.message.split('\n')[0]}`);
  }

  // Test 3: HTTPS google
  console.log('\nTest 3: HTTPS https://www.google.com');
  try {
    await page.goto('https://www.google.com', { 
      timeout: 15000,
      waitUntil: 'domcontentloaded'
    });
    const title = await page.title();
    console.log(`  SUCCESS - Title: "${title}"`);
  } catch (err) {
    console.log(`  FAILED: ${err.message.split('\n')[0]}`);
  }

  // Test 4: HTTPS example.com
  console.log('\nTest 4: HTTPS https://example.com');
  try {
    await page.goto('https://example.com', { timeout: 10000 });
    const title = await page.title();
    console.log(`  SUCCESS - Title: "${title}"`);
  } catch (err) {
    console.log(`  FAILED: ${err.message.split('\n')[0]}`);
  }

  await browser.close();
  console.log('\n=== Tests complete ===');
}

main().catch(err => {
  console.error('Unhandled error:', err);
  process.exit(1);
});
