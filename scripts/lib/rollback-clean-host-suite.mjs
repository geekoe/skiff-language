// Clean-host HTTP-only assertion suite for the rollback final rehearsal.
//
// The clean-host rehearsal runs the production topology without the test
// relay, so the five rollback cases are asserted at the HTTP level only
// (status + body), while a real unary readiness poll proves the Runtime
// reconnected to the Router before the suite starts.

import { setTimeout as delay } from 'node:timers/promises';

import {
  HTTP_LIVE_SERVICE_ID,
  HTTP_LIVE_VERSION,
} from './http_live_fixture.mjs';
import {
  openHttpLiveStream,
  requestFull,
  selectorHeaders,
} from './http_live_client.mjs';

export async function runCleanHostHttpSuite({ port, phase }) {
  const evidence = [];
  await waitForCleanHostUnary({
    port,
    serviceId: HTTP_LIVE_SERVICE_ID,
    version: HTTP_LIVE_VERSION,
    timeoutMs: 90_000,
    phase,
  });
  await cleanHostUnaryHappy({ port, phase, evidence });
  await cleanHostTypedUnary({ port, phase, evidence });
  await cleanHostMissingSelector({ port, phase, evidence });
  await cleanHostWrongPath({ port, phase, evidence });
  await cleanHostStreamRoundtrip({ port, phase, evidence });
  return evidence;
}

export async function waitForCleanHostUnary({
  port,
  serviceId,
  version,
  timeoutMs,
  phase,
}) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    try {
      const response = await requestFull({
        port,
        method: 'POST',
        path: '/unary',
        headers: selectorHeaders({ service: serviceId, version }),
        body: Buffer.from(`ready-${phase}`, 'utf8'),
      });
      if (response.status === 201) {
        return;
      }
    } catch {
      // Router or Runtime still converging; retry.
    }
    await delay(250);
  }
  throw new Error(
    `clean-host Runtime reconnect/unary did not become ready within ${timeoutMs}ms`,
  );
}

async function cleanHostUnaryHappy({ port, phase, evidence }) {
  const body = Buffer.from(`rollback-${phase}`, 'utf8');
  const response = await requestFull({
    port,
    method: 'POST',
    path: '/unary',
    headers: serviceHeaders(),
    body,
  });
  assertEqual(response.status, 201, `${phase} clean-host unary status`);
  assertEqual(
    response.body.toString('utf8'),
    `unary:rollback-${phase}`,
    `${phase} clean-host unary body`,
  );
  evidence.push({ phase, name: 'unary-happy', status: response.status });
}

async function cleanHostTypedUnary({ port, phase, evidence }) {
  const response = await requestFull({
    port,
    method: 'POST',
    path: '/typed-unary',
    headers: serviceHeaders(),
    body: Buffer.from('"hello"', 'utf8'),
  });
  assertEqual(response.status, 200, `${phase} clean-host typed unary status`);
  assertEqual(
    JSON.parse(response.body.toString('utf8')),
    'typed:hello',
    `${phase} clean-host typed unary body`,
  );
  evidence.push({ phase, name: 'typed-unary', status: response.status });
}

async function cleanHostMissingSelector({ port, phase, evidence }) {
  const response = await requestFull({
    port,
    method: 'POST',
    path: '/unary',
    headers: {},
    body: Buffer.from('x', 'utf8'),
  });
  assertJsonError(
    response,
    400,
    'ServiceSelectorRequired',
    `${phase} clean-host missing selector`,
  );
  evidence.push({ phase, name: 'missing-selector', status: response.status });
}

async function cleanHostWrongPath({ port, phase, evidence }) {
  const response = await requestFull({
    port,
    method: 'GET',
    path: '/missing',
    headers: serviceHeaders(),
  });
  assertJsonError(
    response,
    404,
    'AssemblyIngressNotFound',
    `${phase} clean-host wrong path`,
  );
  evidence.push({ phase, name: 'wrong-path', status: response.status });
}

async function cleanHostStreamRoundtrip({ port, phase, evidence }) {
  const stream = await openHttpLiveStream({
    port,
    method: 'POST',
    path: '/stream',
    headers: serviceHeaders(),
    body: Buffer.from('middle', 'utf8'),
  });
  assertEqual(stream.status, 206, `${phase} clean-host stream status`);
  const chunks = [];
  while (true) {
    const chunk = await stream.readChunk();
    if (chunk === null) {
      break;
    }
    chunks.push(chunk);
  }
  assertEqual(
    Buffer.concat(chunks).toString('utf8'),
    'alpha|middle|omega',
    `${phase} clean-host stream body`,
  );
  evidence.push({ phase, name: 'stream-roundtrip', status: stream.status });
}

function serviceHeaders() {
  return selectorHeaders({
    service: HTTP_LIVE_SERVICE_ID,
    version: HTTP_LIVE_VERSION,
  });
}

function assertJsonError(response, status, code, label) {
  assertEqual(response.status, status, `${label} status`);
  let body;
  try {
    body = JSON.parse(response.body.toString('utf8'));
  } catch (error) {
    throw new Error(`${label} expected JSON error body`, { cause: error });
  }
  assertEqual(body?.error?.code, code, `${label} code`);
}

function assertEqual(actual, expected, label) {
  if (!Object.is(actual, expected)) {
    throw new Error(
      `${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}
