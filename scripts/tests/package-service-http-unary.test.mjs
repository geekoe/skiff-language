import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import test from 'node:test';

import {
  requestPackageServiceHttpUnary,
  validatePackageServiceHttpUnaryStringResponse,
} from '../lib/package-service-http-unary.mjs';
import { encodeRuntimePayload } from '../lib/runtime-payload-codec.mjs';

const SERVICE_ID = 'test.skiff/package-service-http-unary';
const SERVICE_VERSION = '1.0.0';

test('HTTP unary client preserves a bounded successful raw body', async () => {
  const encoded = encodeRuntimePayload('raw-success', { type: 'string' });
  const server = await listenUnaryServer({ status: 200, body: encoded });
  try {
    const response = await requestPackageServiceHttpUnary({
      url: `${server.origin}/probe`,
      serviceId: SERVICE_ID,
      serviceVersion: SERVICE_VERSION,
      signal: new AbortController().signal,
    });
    assert.deepEqual(response, {
      status: 200,
      body: encoded,
      bodyBytes: encoded.byteLength,
      bodyTruncated: false,
    });
    assert.deepEqual(server.requests, [{
      method: 'POST',
      url: '/probe',
      serviceId: SERVICE_ID,
      serviceVersion: SERVICE_VERSION,
    }]);
  } finally {
    await server.close();
  }
});

test('HTTP unary client rejects a truncated oversized 200 body', async () => {
  const server = await listenUnaryServer({
    status: 200,
    body: Buffer.alloc(513, 0x61),
  });
  try {
    await assert.rejects(
      requestPackageServiceHttpUnary({
        url: `${server.origin}/probe`,
        serviceId: SERVICE_ID,
        serviceVersion: SERVICE_VERSION,
        signal: new AbortController().signal,
      }),
      /package-service HTTP unary response exceeded 512 bytes/,
    );
  } finally {
    await server.close();
  }
});

test('HTTP unary client reports exact non-200 wire metadata', async () => {
  const responseBody = 'not found';
  const server = await listenUnaryServer({ status: 404, body: responseBody });
  try {
    await assert.rejects(
      requestPackageServiceHttpUnary({
        url: `${server.origin}/probe`,
        serviceId: SERVICE_ID,
        serviceVersion: SERVICE_VERSION,
        signal: new AbortController().signal,
      }),
      (error) => {
        assert.match(error.message, /method=POST/);
        assert.match(error.message, new RegExp(`url=${server.origin}/probe`));
        assert.match(error.message, new RegExp(`serviceId=${SERVICE_ID.replaceAll('.', '\\.')}`));
        assert.match(error.message, /serviceVersion=1\.0\.0/);
        assert.match(error.message, /status=404/);
        assert.match(error.message, /responseBody="not found"/);
        assert.match(error.message, /responseBodyBytes=9/);
        assert.match(error.message, /responseBodyTruncated=false/);
        return true;
      },
    );
  } finally {
    await server.close();
  }
});

test('HTTP unary client bounds and redacts non-200 response diagnostics', async () => {
  const secret = 'P5_HTTP_UNARY_SECRET';
  const responseBody =
    `token=${secret} ${'/private/fixture/main.skiff '.repeat(40)}`;
  const server = await listenUnaryServer({ status: 503, body: responseBody });
  try {
    await assert.rejects(
      requestPackageServiceHttpUnary({
        url: `${server.origin}/probe`,
        serviceId: SERVICE_ID,
        serviceVersion: SERVICE_VERSION,
        signal: new AbortController().signal,
      }),
      (error) => {
        assert.match(error.message, /status=503/);
        assert.match(error.message, /responseBodyBytes=/);
        assert.match(error.message, /responseBodyTruncated=true/);
        assert.match(error.message, /<REDACTED_SECRET>/);
        assert.match(error.message, /<PATH>/);
        assert.doesNotMatch(error.message, new RegExp(secret));
        assert.doesNotMatch(error.message, /private\/fixture/);
        return true;
      },
    );
  } finally {
    await server.close();
  }
});

test('HTTP unary string validator decodes only canonical RuntimePayload bytes', () => {
  const expected = 'typed-result';
  const encoded = encodeRuntimePayload(expected, { type: 'string' });
  assert.equal(
    validatePackageServiceHttpUnaryStringResponse(
      { status: 200, body: encoded },
      expected,
    ),
    expected,
  );
  assert.throws(
    () => validatePackageServiceHttpUnaryStringResponse(
      { status: 200, body: Buffer.from(JSON.stringify(expected)) },
      expected,
    ),
    /runtime payload bytes missing SKPV magic/,
  );
  assert.throws(
    () => validatePackageServiceHttpUnaryStringResponse(
      { status: 200, body: encoded.subarray(0, encoded.byteLength - 1) },
      expected,
    ),
    /runtime payload ended early/,
  );
  assert.throws(
    () => validatePackageServiceHttpUnaryStringResponse(
      {
        status: 200,
        body: encodeRuntimePayload('wrong-result', { type: 'string' }),
      },
      expected,
    ),
    /Expected values to be strictly equal/,
  );
});

async function listenUnaryServer({ status, body }) {
  const requests = [];
  const server = createServer((request, response) => {
    requests.push({
      method: request.method,
      url: request.url,
      serviceId: request.headers['x-skiff-service'],
      serviceVersion: request.headers['x-skiff-version'],
    });
    response.statusCode = status;
    response.end(body);
  });
  await new Promise((resolveListen) => {
    server.listen(0, '127.0.0.1', resolveListen);
  });
  const address = server.address();
  assert.ok(address !== null && typeof address !== 'string');
  return {
    close: () => new Promise((resolveClose, rejectClose) => {
      server.close((error) => {
        if (error !== undefined) rejectClose(error);
        else resolveClose();
      });
    }),
    origin: `http://127.0.0.1:${address.port}`,
    requests,
  };
}
