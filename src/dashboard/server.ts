import express from 'express';
import { MitmServer } from '../proxy/mitm-server.js';
import path from 'path';
import fs from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export class DashboardServer {
  private app = express();

  constructor(
    private port: number,
    private mitmServer: MitmServer,
    private cdpPort: number,
    private proxyId: string,
    private sslCaDir: string
  ) {
    this.setupEndpoints();
  }

  private setupEndpoints() {
    this.app.get('/', (req, res) => {
      res.send(`
        <!DOCTYPE html>
        <html>
        <head>
          <title>MITM Proxy Dashboard</title>
          <style>
            body { font-family: sans-serif; padding: 20px; line-height: 1.6; max-width: 800px; margin: 0 auto; }
            .card { border: 1px solid #ccc; padding: 15px; border-radius: 8px; margin-bottom: 20px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
            button { padding: 10px 15px; cursor: pointer; background: #007bff; color: white; border: none; border-radius: 4px; }
            button:hover { background: #0056b3; }
            pre { background: #f4f4f4; padding: 10px; border-radius: 4px; overflow-x: auto; }
            code { color: #d63384; }
            a { color: #007bff; text-decoration: none; }
            a:hover { text-decoration: underline; }
          </style>
        </head>
        <body>
          <h1>MITM Proxy Dashboard</h1>
          
          <div class="card">
            <h2>1. Install Certificate</h2>
            <p>To intercept HTTPS traffic, you must install and trust the Root CA certificate.</p>
            <button onclick="window.location.href='/ca.pem'">Download CA.pem</button>
            <p><small>Note: On macOS, you need to open Keychain Access, import the cert, and set it to "Always Trust".</small></p>
          </div>

          <div class="card">
            <h2>2. Configure Proxy</h2>
            <p>Set your browser or system proxy to:</p>
            <pre>Host: <code>localhost</code>\nPort: <code>8080</code></pre>
          </div>

          <div class="card">
            <h2>3. Inspect Traffic</h2>
            <p>This proxy exposes a Chrome DevTools compatible endpoint.</p>
            <p>1. Open <code>chrome://inspect</code> in a Chrome-based browser.</p>
            <p>2. Ensure <code>localhost:${this.cdpPort}</code> is in the "Configure..." list.</p>
            <p>3. Look for <strong>"MITM Proxy"</strong> under Remote Target and click <strong>inspect</strong>.</p>
            
            <hr>
            <p>Direct Link (might require DevTools extensions or specific browser settings):</p>
            <a href="devtools://devtools/bundled/js_app.html?remoteHub=true&ws=localhost:${this.cdpPort}/devtools/page/${this.proxyId}">
              Open DevTools UI
            </a>
          </div>
        </body>
        </html>
      `);
    });

    this.app.get('/ca.pem', (req, res) => {
      const caPath = join(this.sslCaDir, 'certs', 'ca.pem');
      if (fs.existsSync(caPath)) {
        res.download(caPath);
      } else {
        res.status(404).send('CA certificate not found. Start the proxy first.');
      }
    });
  }

  public async start(): Promise<void> {
    return new Promise((resolve) => {
      this.app.listen(this.port, () => {
        console.log(`Dashboard Server listening on port ${this.port}`);
        resolve();
      });
    });
  }
}
