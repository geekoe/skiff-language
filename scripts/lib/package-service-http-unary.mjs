import assert from 'node:assert/strict';
import { request as requestHttp } from 'node:http';

import { sanitizeFixtureCargoDiagnostic } from './package-service-ecosystem-smoke-diagnostic.mjs';
import { decodeRuntimePayload } from './runtime-payload-codec.mjs';

const RESPONSE_BODY_MAX_BYTES = 512;
const RUNTIME_PAYLOAD_STRING_SCHEMA = Object.freeze({ type: 'string' });

export async function requestPackageServiceHttpUnary({
  method = 'POST',
  url,
  serviceId,
  serviceVersion,
  signal,
}) {
  assert.equal(method, 'POST', 'package-service HTTP unary request must use POST');
  assert.equal(
    typeof serviceId,
    'string',
    'package-service HTTP unary serviceId must be a string',
  );
  assert.ok(serviceId.length > 0, 'package-service HTTP unary serviceId must not be empty');
  assert.equal(
    typeof serviceVersion,
    'string',
    'package-service HTTP unary serviceVersion must be a string',
  );
  assert.ok(
    serviceVersion.length > 0,
    'package-service HTTP unary serviceVersion must not be empty',
  );
  const target = new URL(url);
  assert.equal(
    target.protocol,
    'http:',
    'package-service HTTP unary request must target the isolated HTTP ingress',
  );
  const response = await new Promise((resolveResponse, rejectResponse) => {
    const request = requestHttp(target, {
      method,
      headers: {
        'x-skiff-service': serviceId,
        'x-skiff-version': serviceVersion,
      },
      signal,
    }, resolveResponse);
    request.once('error', rejectResponse);
    request.end();
  });
  const body = await readBoundedResponseBody(response);
  const result = {
    status: response.statusCode,
    body: body.value,
    bodyBytes: body.bytes,
    bodyTruncated: body.truncated,
  };
  assertHttpUnarySuccess({ method, url, serviceId, serviceVersion }, result);
  return result;
}

export function validatePackageServiceHttpUnaryStringResponse(
  response,
  expectedValue,
) {
  assert.equal(
    response.status,
    200,
    'package-service HTTP unary response must return HTTP 200',
  );
  assert.equal(
    response.bodyTruncated ?? false,
    false,
    `package-service HTTP unary response exceeded ${RESPONSE_BODY_MAX_BYTES} bytes`,
  );
  assert.ok(
    Buffer.isBuffer(response.body),
    'package-service HTTP unary response body must be raw bytes',
  );
  assert.equal(typeof expectedValue, 'string');
  const value = decodeRuntimePayload(
    response.body,
    RUNTIME_PAYLOAD_STRING_SCHEMA,
  );
  assert.equal(value, expectedValue);
  return value;
}

function assertHttpUnarySuccess(request, response) {
  if (response.status === 200) {
    assert.equal(
      response.bodyTruncated,
      false,
      `package-service HTTP unary response exceeded ${RESPONSE_BODY_MAX_BYTES} bytes`,
    );
    assert.ok(
      Buffer.isBuffer(response.body),
      'package-service HTTP unary response body must remain raw bytes',
    );
    return;
  }
  const body = boundedDiagnostic(
    sanitizeFixtureCargoDiagnostic(
      Buffer.isBuffer(response.body) ? response.body.toString('utf8') : '',
    ),
  );
  throw new Error([
    'package-service HTTP unary request failed',
    `method=${request.method}`,
    `url=${request.url}`,
    `serviceId=${request.serviceId}`,
    `serviceVersion=${request.serviceVersion}`,
    `status=${response.status}`,
    `responseBody=${JSON.stringify(body.value)}`,
    `responseBodyBytes=${response.bodyBytes ?? response.body?.byteLength ?? 0}`,
    `responseBodyTruncated=${response.bodyTruncated === true || body.truncated}`,
  ].join(' '));
}

function boundedDiagnostic(value) {
  let bytes = 0;
  let bounded = '';
  for (const character of value) {
    const characterBytes = Buffer.byteLength(character);
    if (bytes + characterBytes > RESPONSE_BODY_MAX_BYTES) {
      return { value: bounded, truncated: true };
    }
    bounded += character;
    bytes += characterBytes;
  }
  return { value: bounded, truncated: false };
}

async function readBoundedResponseBody(response) {
  const chunks = [];
  let retainedBytes = 0;
  let bytes = 0;
  for await (const chunk of response) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
    bytes += buffer.byteLength;
    if (retainedBytes >= RESPONSE_BODY_MAX_BYTES) continue;
    const retained = buffer.subarray(
      0,
      RESPONSE_BODY_MAX_BYTES - retainedBytes,
    );
    chunks.push(retained);
    retainedBytes += retained.byteLength;
  }
  return {
    value: Buffer.concat(chunks),
    bytes,
    truncated: bytes > retainedBytes,
  };
}
