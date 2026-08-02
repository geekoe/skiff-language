// Minimal node:http client used by the `router-live:http` harness.
//
// `requestFull` returns the complete response; `openHttpLiveStream` exposes
// a pull-based body reader so the harness can pause a stream (backpressure),
// read a head and a chunk then destroy the socket (client disconnect), and
// observe truncated bodies (stream ceilings / protocol terminals).

import http from 'node:http';

export const SERVICE_HEADER = 'x-skiff-service';
export const VERSION_HEADER = 'x-skiff-version';
export const RELEASE_HEADER = 'x-skiff-release';

export function selectorHeaders({ service, version, release } = {}) {
  const headers = {};
  if (service !== undefined) {
    headers[SERVICE_HEADER] = service;
  }
  if (version !== undefined) {
    headers[VERSION_HEADER] = version;
  }
  if (release !== undefined) {
    headers[RELEASE_HEADER] = release;
  }
  return headers;
}

export function openHttpLiveStream({
  port,
  method,
  path,
  headers = {},
  body,
}) {
  return new Promise((resolvePromise, reject) => {
    const req = http.request(
      {
        host: '127.0.0.1',
        port,
        method,
        path,
        headers: { ...headers, host: `127.0.0.1:${port}` },
      },
      (res) => {
        const stream = new HttpLiveStream(req, res);
        resolvePromise(stream);
      },
    );
    req.once('error', reject);
    if (body !== undefined && body !== null) {
      req.write(body);
    }
    req.end();
  });
}

export async function requestFull({
  port,
  method,
  path,
  headers,
  body,
  timeoutMs = 20_000,
}) {
  const stream = await openHttpLiveStream({ port, method, path, headers, body });
  const timer = setTimeout(() => {
    stream.destroy();
  }, timeoutMs);
  try {
    const chunks = [];
    while (true) {
      const chunk = await stream.readChunk();
      if (chunk === null) {
        break;
      }
      chunks.push(chunk);
    }
    return {
      status: stream.status,
      headers: stream.headers,
      body: Buffer.concat(chunks),
      aborted: stream.aborted,
    };
  } finally {
    clearTimeout(timer);
  }
}

export class HttpLiveStream {
  constructor(req, res) {
    this.req = req;
    this.res = res;
    this.status = res.statusCode;
    this.headers = res.headers;
    this.buffered = [];
    this.pending = [];
    this.ended = false;
    this.aborted = false;
    this.error = null;

    res.on('data', (chunk) => {
      const waiter = this.pending.shift();
      if (waiter !== undefined) {
        waiter.resolve(Buffer.from(chunk));
      } else {
        this.buffered.push(Buffer.from(chunk));
      }
    });
    // Stay paused until the caller asks for data. The harness uses this to
    // stop reading a stream entirely (backpressure drain) and to read a head
    // plus one chunk before destroying the socket (client disconnect).
    this.res.pause();
    res.on('end', () => {
      this.ended = true;
      this.settle(null);
    });
    res.on('aborted', () => {
      this.aborted = true;
      this.ended = true;
      this.settle(null);
    });
    res.on('error', (error) => {
      this.error = error;
      this.ended = true;
      this.settle(error);
    });
    req.on('error', (error) => {
      this.error = error;
      this.ended = true;
      this.settle(error);
    });
  }

  readChunk() {
    if (this.error !== null) {
      return Promise.reject(this.error);
    }
    if (this.buffered.length > 0) {
      return Promise.resolve(this.buffered.shift());
    }
    if (this.ended) {
      return Promise.resolve(null);
    }
    return new Promise((resolvePromise, reject) => {
      this.pending.push({ resolve: resolvePromise, reject });
      this.res.resume();
    });
  }

  destroy() {
    this.req.destroy();
  }

  setRecvBufferSize(size) {
    this.res.socket?.setRecvBufferSize(size);
  }

  settle(value) {
    while (this.pending.length > 0) {
      const waiter = this.pending.shift();
      if (value instanceof Error) {
        waiter.reject(value);
      } else {
        waiter.resolve(value);
      }
    }
  }
}
