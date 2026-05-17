import { MitmServer } from './proxy/mitm-server.js';
import { CdpServer } from './cdp/cdp-server.js';
import { EventMapper } from './cdp/event-mapper.js';
import { DashboardServer } from './dashboard/server.js';
import os from 'node:os';
import fs from 'node:fs';
import path from 'node:path';

async function main() {
  const MITM_PORT = 8080;
  const CDP_PORT = 9222;
  const DASHBOARD_PORT = 3000;

  // Use a stable directory for the CA so you only have to trust it once
  const sslCaDir = path.join(process.cwd(), '.http-mitm-proxy');
  if (!fs.existsSync(sslCaDir)) {
    fs.mkdirSync(sslCaDir, { recursive: true });
  }
  console.log(`Using CA directory: ${sslCaDir}`);

  const mitmServer = new MitmServer(sslCaDir);
  const cdpServer = new CdpServer(CDP_PORT, (id) => mitmServer.getSession(id));
  const _eventMapper = new EventMapper(mitmServer, cdpServer);
  const dashboardServer = new DashboardServer(DASHBOARD_PORT, mitmServer, CDP_PORT, cdpServer.proxyId, sslCaDir);

  console.log('Starting MITM Proxy components...');
  
  await Promise.all([
    mitmServer.start(MITM_PORT),
    cdpServer.start(),
    dashboardServer.start(),
  ]);

  console.log('\nAll components started successfully!');
  console.log(`- MITM Proxy: 127.0.0.1:${MITM_PORT}`);
  console.log(`- CDP Server: 127.0.0.1:${CDP_PORT}`);
  console.log(`- Dashboard:  http://127.0.0.1:${DASHBOARD_PORT}`);
  console.log('\nInstructions:');
  console.log('1. Visit the dashboard to download and trust the CA certificate.');
  console.log('2. Configure your browser to use the proxy.');
  console.log('3. Open chrome://inspect and click "Configure..." to add 127.0.0.1:9222.');
  console.log('4. Click "inspect" on the MITM Proxy target.');
}

main().catch((err) => {
  console.error('Failed to start proxy:', err);
  process.exit(1);
});
