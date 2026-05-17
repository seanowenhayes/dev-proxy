import { MitmServer } from '../proxy/mitm-server.js';
import { CdpServer } from './cdp-server.js';
import { InterceptedSession } from '../types.js';

export class EventMapper {
  constructor(
    private mitmServer: MitmServer,
    private cdpServer: CdpServer
  ) {
    this.setupListeners();
  }

  private setupListeners() {
    this.mitmServer.on('request', (session: InterceptedSession) => {
      this.handleRequest(session);
    });

    this.mitmServer.on('response', (session: InterceptedSession) => {
      this.handleResponse(session);
    });
  }

  private handleRequest(session: InterceptedSession) {
    const { request } = session;
    
    this.cdpServer.broadcast('Network.requestWillBeSent', {
      requestId: request.id,
      loaderId: 'proxy-loader',
      documentURL: request.url,
      request: {
        url: request.url,
        method: request.method,
        headers: request.headers,
        initialPriority: 'VeryHigh',
        referrerPolicy: 'no-referrer-when-downgrade',
      },
      timestamp: request.startTime / 1000,
      wallTime: request.startTime / 1000,
      initiator: { type: 'other' },
      type: 'Other',
    });
  }

  private handleResponse(session: InterceptedSession) {
    const { request, response } = session;
    if (!response) return;

    this.cdpServer.broadcast('Network.responseReceived', {
      requestId: response.id,
      loaderId: 'proxy-loader',
      timestamp: response.endTime / 1000,
      type: 'Other',
      response: {
        url: request.url,
        status: response.status,
        statusText: 'OK',
        headers: response.headers,
        mimeType: (response.headers['content-type'] as string) || 'text/plain',
        connectionReused: true,
        connectionId: 0,
        encodedDataLength: response.body?.length || 0,
        fromDiskCache: false,
        fromServiceWorker: false,
        securityState: 'secure',
      },
    });

    this.cdpServer.broadcast('Network.loadingFinished', {
      requestId: response.id,
      timestamp: response.endTime / 1000,
      encodedDataLength: response.body?.length || 0,
    });
  }
}
