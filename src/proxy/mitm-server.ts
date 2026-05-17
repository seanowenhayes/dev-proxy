import { Proxy } from 'http-mitm-proxy';
import { EventEmitter } from 'events';
import { InterceptedSession } from '../types.js';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export class MitmServer extends EventEmitter {
  private proxy: Proxy;
  private sessions: Map<string, InterceptedSession> = new Map();
  private sslCaDir: string;

  constructor(sslCaDir?: string) {
    super();
    this.proxy = new Proxy();
    this.sslCaDir = sslCaDir || join(__dirname, '..', '..', '.http-mitm-proxy');
  }

  public async start(port: number = 8080): Promise<void> {
    this.proxy.onError((ctx: any, err: any) => {
      console.error('Proxy Error:', err);
    });

    this.proxy.onRequest((ctx: any, callback: any) => {
      const requestId = randomUUID();
      const requestData = {
        id: requestId,
        method: ctx.clientToProxyRequest.method,
        url: ctx.clientToProxyRequest.url,
        headers: ctx.clientToProxyRequest.headers,
        startTime: Date.now(),
      };

      // Correctly handle full URL
      const host = ctx.clientToProxyRequest.headers.host;
      const protocol = ctx.isSSL ? 'https' : 'http';
      requestData.url = `${protocol}://${host}${ctx.clientToProxyRequest.url}`;

      const session: InterceptedSession = {
        id: requestId,
        request: requestData,
      };

      this.sessions.set(requestId, session);
      this.emit('request', session);

      ctx.onResponse((ctx: any, callback: any) => {
        const responseChunks: Buffer[] = [];
        ctx.onResponseData((ctx: any, chunk: Buffer, callback: any) => {
          responseChunks.push(chunk);
          return callback(undefined, chunk);
        });

        ctx.onResponseEnd((ctx: any, callback: any) => {
          const session = this.sessions.get(requestId);
          if (session) {
            session.response = {
              id: requestId,
              status: ctx.proxyToClientResponse.statusCode,
              headers: ctx.proxyToClientResponse.headers,
              endTime: Date.now(),
              body: Buffer.concat(responseChunks),
            };
            this.emit('response', session);
          }
          return callback();
        });

        return callback();
      });

      return callback();
    });

    return new Promise((resolve) => {
      this.proxy.listen({ port, sslCaDir: this.sslCaDir }, (err: any) => {
        if (err) {
          console.error('Failed to start proxy:', err);
        } else {
          console.log(`MITM Proxy listening on port ${port}`);
          console.log(`SSL CA directory: ${this.sslCaDir}`);
        }
        resolve();
      });
    });
  }

  public getSession(id: string): InterceptedSession | undefined {
    return this.sessions.get(id);
  }

  public stop() {
    this.proxy.close();
  }
}
