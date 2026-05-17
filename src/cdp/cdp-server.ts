import { WebSocketServer, WebSocket } from 'ws';
import express from 'express';
import { Server } from 'http';
import { randomUUID } from 'node:crypto';
import { InterceptedSession } from '../types.js';

export class CdpServer {
  private app = express();
  private wss: WebSocketServer;
  private server?: Server;
  private connections: Set<WebSocket> = new Set();
  public proxyId = randomUUID();

  constructor(
    private port: number,
    private getSession: (id: string) => InterceptedSession | undefined
  ) {
    this.setupEndpoints();
    this.wss = new WebSocketServer({ noServer: true });
    this.setupWebSocket();
  }

  private setupEndpoints() {
    this.app.get('/json/version', (req, res) => {
      res.json({
        Browser: 'CDP-Proxy/1.0.0',
        'Protocol-Version': '1.3',
        'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
        'V8-Version': '12.0.267.8',
        'WebKit-Version': '537.36 (@785728f)',
        'webSocketDebuggerUrl': `ws://localhost:${this.port}/devtools/browser/${this.proxyId}`
      });
    });

    this.app.get('/json/list', (req, res) => {
      res.json([{
        description: 'MITM Proxy Session',
        devtoolsFrontendUrl: `devtools://devtools/bundled/js_app.html?remoteHub=true&ws=localhost:${this.port}/devtools/page/${this.proxyId}`,
        id: this.proxyId,
        title: 'MITM Proxy',
        type: 'page',
        url: 'https://proxy.local',
        webSocketDebuggerUrl: `ws://localhost:${this.port}/devtools/page/${this.proxyId}`
      }]);
    });

    this.app.get('/json', (req, res) => {
      res.redirect('/json/list');
    });
  }

  private setupWebSocket() {
    this.wss.on('connection', (ws) => {
      console.log('[CDP] DevTools connected');
      this.connections.add(ws);

      ws.on('message', (data) => {
        try {
          const message = JSON.parse(data.toString());
          this.handleCdpMessage(ws, message);
        } catch (error) {
          console.error('[CDP] Error parsing message:', error);
        }
      });

      ws.on('close', () => {
        console.log('[CDP] DevTools disconnected');
        this.connections.delete(ws);
      });
    });
  }

  private handleCdpMessage(ws: WebSocket, message: any) {
    const { id, method, params } = message;

    if (method === 'Network.getResponseBody') {
      const session = this.getSession(params.requestId);
      if (session?.response?.body) {
        ws.send(JSON.stringify({
          id,
          result: {
            body: session.response.body.toString('base64'),
            base64Encoded: true
          }
        }));
      } else {
        ws.send(JSON.stringify({
          id,
          error: { code: -32000, message: 'Response body not available' }
        }));
      }
      return;
    }

    // Handle common DevTools initialization
    if (method === 'Network.enable') {
      ws.send(JSON.stringify({ id, result: {} }));
    } else if (method === 'Page.enable') {
      ws.send(JSON.stringify({ id, result: {} }));
    } else if (method === 'Runtime.enable') {
      ws.send(JSON.stringify({ id, result: {} }));
    } else if (method === 'Log.enable') {
      ws.send(JSON.stringify({ id, result: {} }));
    } else {
      // Default success for other methods to keep DevTools happy
      ws.send(JSON.stringify({ id, result: {} }));
    }
  }

  public broadcast(method: string, params: any) {
    const message = JSON.stringify({ method, params });
    for (const ws of this.connections) {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(message);
      }
    }
  }

  public async start(): Promise<void> {
    return new Promise((resolve) => {
      this.server = this.app.listen(this.port, '127.0.0.1', () => {
        console.log(`CDP Server listening on 127.0.0.1:${this.port}`);
        resolve();
      });

      this.server.on('upgrade', (request, socket, head) => {
        const url = new URL(request.url!, `http://${request.headers.host}`);
        if (url.pathname.startsWith('/devtools/')) {
          this.wss.handleUpgrade(request, socket, head, (ws) => {
            this.wss.emit('connection', ws, request);
          });
        } else {
          socket.destroy();
        }
      });
    });
  }

  public stop() {
    this.server?.close();
    this.wss.close();
  }
}
