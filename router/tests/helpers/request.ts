import {
  request as createHttpRequest,
  type IncomingHttpHeaders
} from 'node:http';

export async function requestHttp(input: {
  url: string;
  method?: string;
  headers?: Record<string, string | string[]>;
  body?: string | Buffer;
}): Promise<{
  status: number;
  headers: IncomingHttpHeaders;
  rawHeaders: string[];
  rawBody: Buffer;
  body: string;
}> {
  const url = new URL(input.url);
  return await new Promise((resolve, reject) => {
    const request = createHttpRequest(
      {
        hostname: url.hostname,
        port: url.port,
        path: `${url.pathname}${url.search}`,
        method: input.method ?? 'GET',
        headers: input.headers
      },
      (response) => {
        const chunks: Buffer[] = [];
        response.on('data', (chunk: Buffer | string) => {
          chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
        });
        response.on('end', () => {
          const rawBody = Buffer.concat(chunks);
          resolve({
            status: response.statusCode ?? 0,
            headers: response.headers,
            rawHeaders: response.rawHeaders,
            rawBody,
            body: rawBody.toString('utf8')
          });
        });
      }
    );
    request.on('error', reject);
    if (input.body !== undefined) {
      request.write(input.body);
    }
    request.end();
  });
}

export function firstHeader(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}
