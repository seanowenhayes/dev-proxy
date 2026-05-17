export interface RequestData {
  id: string;
  method: string;
  url: string;
  headers: Record<string, string | string[] | undefined>;
  startTime: number;
}

export interface ResponseData {
  id: string;
  status: number;
  headers: Record<string, string | string[] | undefined>;
  endTime: number;
  body?: Buffer;
}

export interface InterceptedSession {
  id: string;
  request: RequestData;
  response?: ResponseData;
}
