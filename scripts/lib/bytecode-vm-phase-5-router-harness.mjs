#!/usr/bin/env node

// Phase 5 G7/G8 process proof. One external HTTP request traverses the
// production Router listener, its production dispatch/session machinery, a
// transparent frame-recording WebSocket relay, and the production RuntimeHost.
// The only test server is the remote HTTP peer consumed by std.http; it never
// creates a deployment image, executor, stream handle, or Runtime response.

import assert from 'node:assert/strict';
import http from 'node:http';
import {
  access,
  mkdir,
  mkdtemp,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import {
  closeLogs,
  renderHttpLiveRouterConfig,
  spawnLoggedProcess,
  stopProcess,
  waitForHandshakeAfter,
  waitForListeners,
  writeHttpLiveRouterConfig,
  writeHttpLiveRuntimeConfig,
} from './http_live_process.mjs';
import { requestFull, selectorHeaders } from './http_live_client.mjs';
import { leaseConsecutiveLocalPorts } from './local-port-lease.mjs';
import { MongodLiveHarness } from './mongod-live-harness.mjs';
import { createRollbackRelay } from './rollback-relay.mjs';

const CARRIER_ENV = 'SKIFF_BYTECODE_VM_PHASE5_CARRIER_ROOT';
const RUNTIME_BIN_ENV = 'SKIFF_BYTECODE_VM_PHASE5_RUNTIME_BIN';
const ROUTER_BIN_ENV = 'SKIFF_BYTECODE_VM_PHASE5_ROUTER_BIN';
const PROFILE = 'skiff-test';
const SERVICE_ID = 'test.skiff/bytecode-vm-phase-5';
const VERSION = '1.0.0';
const RUNTIME_ID = 'phase-5-router-runtime';
const ZERO_COUNTERS = Object.freeze({
  outboundRequestsPending: 0,
  outboundStreamLeasesActive: 0,
  streamRuntimeStreamsActive: 0,
  flagBackedCancelWaitersActive: 0,
  taskRequestsActive: 0,
});

await main().catch((error) => {
  process.stderr.write(`${error?.stack ?? error}\n`);
  process.exitCode = 1;
});

async function main() {
  const repository = resolve(new URL('../..', import.meta.url).pathname);
  const carrierRoot = await requiredCanonicalDirectory(CARRIER_ENV);
  const runtimeBin = await requiredFile(RUNTIME_BIN_ENV);
  const routerBin = await requiredFile(ROUTER_BIN_ENV);
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-phase5-router-vcp-'));
  const lease = await leaseConsecutiveLocalPorts({
    rangeStart: 46000,
    rangeEnd: 46999,
    count: 3,
  });
  const [httpPort, runtimePort, relayPort] = lease.ports;
  const upstream = await createGatedUpstream();
  const disconnectUpstream = await createGatedUpstream();
  const mongo = await MongodLiveHarness.create({ repoRoot: repository });
  let router;
  let runtime;
  let relay;
  let primaryError;
  try {
    await mongo.start();
    const runtimeHome = join(tempRoot, 'runtime-home');
    await mkdir(runtimeHome, { recursive: true });
    await writeFile(join(runtimeHome, 'runtime-id'), `${RUNTIME_ID}\n`, { flag: 'wx' });

    const routerConfig = join(tempRoot, 'router.yml');
    await writeHttpLiveRouterConfig(routerConfig, renderHttpLiveRouterConfig({
      profile: PROFILE,
      artifactsPath: carrierRoot,
      httpPort,
      runtimePort,
      mongoUrl: mongo.mongoUrl,
      requestTimeoutMs: 15_000,
      httpMaxRequestBytes: 65_536,
      httpMaxResponseBytes: 65_536,
      runtimeMaxConcurrency: 1,
    }));
    const runtimeConfig = join(tempRoot, 'runtime.yml');
    await writeHttpLiveRuntimeConfig(runtimeConfig, { relayPort, runtimeHome });

    router = await spawnLoggedProcess(routerBin, [routerConfig], {
      cwd: repository,
      stdoutPath: join(tempRoot, 'router.stdout.log'),
      stderrPath: join(tempRoot, 'router.stderr.log'),
    });
    await waitForListeners({
      httpPort,
      runtimePort,
      child: router.child,
      stderrPath: join(tempRoot, 'router.stderr.log'),
    });
    relay = await createRollbackRelay({
      port: relayPort,
      routerUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
    });
    runtime = await spawnLoggedProcess(runtimeBin, [runtimeConfig], {
      cwd: repository,
      stdoutPath: join(tempRoot, 'runtime.stdout.log'),
      stderrPath: join(tempRoot, 'runtime.stderr.log'),
    });
    await waitForHandshakeAfter(relay, 0);

    const disconnect = await exerciseClientDisconnect({
      relay,
      httpPort,
      upstream: disconnectUpstream,
    });

    const fromIndex = relay.records.length;
    const responseOutcome = requestFull({
      port: httpPort,
      method: 'POST',
      path: '/phase-5/vcp',
      headers: selectorHeaders({ service: SERVICE_ID, version: VERSION }),
      body: Buffer.from(upstream.baseUrl, 'utf8'),
      timeoutMs: 15_000,
    }).then(
      (value) => ({ value }),
      (error) => ({ error }),
    );

    try {
      await upstream.waitForTwoOpenStreams();
    } catch (error) {
      const early = await responseOutcome;
      const external = early.error === undefined
        ? {
          status: early.value.status,
          aborted: early.value.aborted,
          body: early.value.body.toString('utf8'),
        }
        : { error: early.error?.message ?? String(early.error) };
      const frames = relay.records.slice(fromIndex).map((record) => ({
        direction: record.direction,
        type: record.type,
        requestId: record.header?.requestId,
      }));
      throw new Error(
        `${error.message}; external=${JSON.stringify(external)}; relay=${JSON.stringify(frames)}`,
      );
    }
    const nonzeroHealth = await waitForHealth(relay, fromIndex, (counters) => (
      counters?.outboundStreamLeasesActive === 2
      && counters?.streamRuntimeStreamsActive === 0
    ), 'two coexisting production stream authorities');
    upstream.releaseBodies();
    const outcome = await responseOutcome;
    if (outcome.error !== undefined) throw outcome.error;
    const response = outcome.value;

    assert.equal(response.aborted, false, 'external response must have one clean terminal');
    assert.equal(response.status, 207, 'external response status crosses the Router boundary');
    assert.equal(
      response.body.toString('utf8'),
      'U=UNARY|A=LEFT-ALEFT-B|B=RIGHT-ARIGHT-B',
      'external response body preserves the six Runtime chunks and A/B routing',
    );
    assert.deepEqual(upstream.routes, [
      { method: 'GET', path: '/request' },
      { method: 'GET', path: '/stream/left' },
      { method: 'GET', path: '/stream/right' },
    ], 'the pinned service executes exactly three distinguishable outbound routes');
    assert.equal(upstream.twoStreamsOpenBeforeRelease, true);

    const requestEvidence = assertRuntimeFrames(relay.records.slice(fromIndex));
    const zeroHealth = await waitForHealth(
      relay,
      nonzeroHealth.index + 1,
      (counters) => exactObject(counters, ZERO_COUNTERS),
      'zero RuntimeHost inventory after the external response terminal',
    );

    process.stdout.write(`${JSON.stringify({
      schemaVersion: 'skiff-bytecode-vm-phase-5-router-proof-r1',
      verdict: 'PASS',
      external: {
        method: 'POST',
        path: '/phase-5/vcp',
        status: response.status,
        body: response.body.toString('utf8'),
      },
      request: requestEvidence,
      upstream: {
        routes: upstream.routes,
        twoStreamsOpenBeforeRelease: upstream.twoStreamsOpenBeforeRelease,
      },
      runtimeHealth: {
        pending: nonzeroHealth.counters,
        terminal: zeroHealth.counters,
      },
      disconnect,
    })}\n`);
  } catch (error) {
    primaryError = error;
    throw error;
  } finally {
    upstream.releaseBodies();
    disconnectUpstream.releaseBodies();
    const cleanupErrors = [];
    await cleanupProcess(router, 'Phase 5 Router', 'SIGTERM', cleanupErrors);
    await cleanupProcess(runtime, 'Phase 5 Runtime', 'SIGINT', cleanupErrors);
    if (relay !== undefined) await relay.close().catch((error) => cleanupErrors.push(error));
    await upstream.close().catch((error) => cleanupErrors.push(error));
    await disconnectUpstream.close().catch((error) => cleanupErrors.push(error));
    await mongo.cleanup().catch((error) => cleanupErrors.push(error));
    await lease.release().catch((error) => cleanupErrors.push(error));
    await rm(tempRoot, { recursive: true, force: true }).catch((error) => cleanupErrors.push(error));
    if (primaryError === undefined && cleanupErrors.length > 0) {
      throw new AggregateError(cleanupErrors, 'Phase 5 Router proof cleanup failed');
    }
  }
}

async function exerciseClientDisconnect({ relay, httpPort, upstream }) {
  const fromIndex = relay.records.length;
  const external = observeRawRequest({
    port: httpPort,
    method: 'POST',
    path: '/phase-5/vcp',
    headers: selectorHeaders({ service: SERVICE_ID, version: VERSION }),
    body: Buffer.from(upstream.baseUrl, 'utf8'),
  });
  try {
    await upstream.waitForTwoOpenStreams();
  } catch (error) {
    const early = await Promise.race([
      external.outcome,
      delay(100).then(() => ({ pending: true })),
    ]);
    const frames = relay.records.slice(fromIndex).map((record) => ({
      direction: record.direction,
      type: record.type,
      requestId: record.header?.requestId,
    }));
    throw new Error(
      `${error.message}; external=${JSON.stringify(early)}; relay=${JSON.stringify(frames)}`,
    );
  }
  const activeHealth = await waitForHealth(relay, fromIndex, (counters) => (
    counters?.outboundStreamLeasesActive === 2
    && counters?.streamRuntimeStreamsActive === 0
  ), 'two table-backed streams before external client disconnect');
  const starts = relay.records.slice(fromIndex).filter(({ type }) => type === 'request.start');
  assert.equal(starts.length, 1, 'disconnect case must dispatch exactly one production request');
  const start = starts[0];
  assert.equal(start.direction, 'ToRuntime');
  assert.equal(start.header?.mode, 'serverStream');
  assert.equal(start.header?.routing?.deployment?.serviceId, SERVICE_ID);
  assert.equal(start.header?.routing?.deployment?.contractVersion, VERSION);
  assert.equal(start.header?.routing?.ingress?.path, '/phase-5/vcp');
  const requestId = start.header?.requestId;
  assert.equal(typeof requestId, 'string');

  external.destroy();
  const cancel = await waitForRecord(relay, activeHealth.index + 1, (record) => (
    record.type === 'request.cancel'
    && record.header?.requestId === requestId
  ), 'Router client_disconnect request.cancel');
  const cancels = relay.records.slice(fromIndex).filter((record) => (
    record.type === 'request.cancel' && record.header?.requestId === requestId
  ));
  assert.deepEqual(
    cancels.map(({ header }) => header?.reason),
    ['client_disconnect'],
    'the real external disconnect must have one exact Router terminal',
  );
  await upstream.waitForTwoClosedStreams();
  const terminalHealth = await waitForHealth(
    relay,
    cancel.index + 1,
    (counters) => exactObject(counters, ZERO_COUNTERS),
    'zero RuntimeHost inventory after Router client_disconnect',
  );
  assert.deepEqual(upstream.routes, [
    { method: 'GET', path: '/request' },
    { method: 'GET', path: '/stream/left' },
    { method: 'GET', path: '/stream/right' },
  ]);
  upstream.releaseBodies();
  return {
    requestId,
    cancelReason: cancels[0].header.reason,
    providerStreamsClosed: true,
    terminalHealth: terminalHealth.counters,
  };
}

function observeRawRequest({ port, method, path, headers, body }) {
  let request;
  const outcome = new Promise((resolvePromise) => {
    request = http.request({
      host: '127.0.0.1',
      port,
      method,
      path,
      headers: { ...headers, host: `127.0.0.1:${port}` },
    }, (response) => {
      const chunks = [];
      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.on('end', () => resolvePromise({
        status: response.statusCode,
        body: Buffer.concat(chunks).toString('utf8'),
      }));
    });
    request.once('error', (error) => resolvePromise({ error: error.message }));
    request.end(body);
  });
  return {
    outcome,
    destroy() { request.destroy(); },
  };
}

async function requiredCanonicalDirectory(name) {
  const value = requiredEnvironment(name);
  const canonical = await realpath(value);
  assert.equal(canonical, value, `${name} must be canonical`);
  return canonical;
}

async function requiredFile(name) {
  const value = requiredEnvironment(name);
  await access(value);
  return value;
}

function requiredEnvironment(name) {
  const value = process.env[name];
  assert.equal(typeof value, 'string', `${name} is required`);
  assert.notEqual(value.length, 0, `${name} is required`);
  assert.equal(resolve(value), value, `${name} must be absolute and normalized`);
  return value;
}

async function cleanupProcess(handle, label, signal, errors) {
  if (handle === undefined) return;
  try {
    const exit = await stopProcess(handle.child, signal, { label });
    if (exit.code !== 0) throw new Error(`${label} exited ${JSON.stringify(exit)}`);
  } catch (error) {
    errors.push(error);
  }
  await closeLogs(handle).catch((error) => errors.push(error));
}

async function waitForHealth(relay, fromIndex, predicate, label) {
  const deadline = Date.now() + 7_000;
  while (Date.now() < deadline) {
    for (let index = fromIndex; index < relay.records.length; index += 1) {
      const record = relay.records[index];
      if (record.type === 'runtime.health'
        && record.direction === 'ToRouter'
        && predicate(record.header?.counters)) {
        return { index, counters: record.header.counters };
      }
    }
    await delay(25);
  }
  const observed = relay.records.slice(fromIndex)
    .filter(({ type }) => type === 'runtime.health')
    .map(({ header }) => header?.counters);
  throw new Error(`timed out waiting for ${label}; observed health=${JSON.stringify(observed)}`);
}

async function waitForRecord(relay, fromIndex, predicate, label) {
  const deadline = Date.now() + 7_000;
  while (Date.now() < deadline) {
    for (let index = fromIndex; index < relay.records.length; index += 1) {
      const record = relay.records[index];
      if (predicate(record)) return { index, record };
    }
    await delay(25);
  }
  throw new Error(`timed out waiting for ${label}`);
}

function assertRuntimeFrames(records) {
  const starts = records.filter(({ type }) => type === 'request.start');
  assert.equal(starts.length, 1, 'external HTTP request must mint exactly one request.start');
  const start = starts[0];
  assert.equal(start.direction, 'ToRuntime');
  assert.equal(start.header?.mode, 'serverStream');
  assert.equal(start.header?.routing?.deployment?.serviceId, SERVICE_ID);
  assert.equal(start.header?.routing?.deployment?.contractVersion, VERSION);
  assert.equal(start.header?.routing?.ingress?.path, '/phase-5/vcp');
  const requestId = start.header.requestId;
  const responseFrames = records.filter((record) => (
    record.header?.requestId === requestId
    && ['response.start', 'response.chunk', 'response.end'].includes(record.type)
  ));
  assert.deepEqual(responseFrames.map(({ type }) => type), [
    'response.start',
    'response.chunk',
    'response.chunk',
    'response.chunk',
    'response.chunk',
    'response.chunk',
    'response.chunk',
    'response.end',
  ]);
  assert.equal(responseFrames.every(({ direction }) => direction === 'ToRouter'), true);
  assert.equal(responseFrames[0].header?.httpResponse?.status, 207);
  assert.deepEqual(
    responseFrames.filter(({ type }) => type === 'response.chunk').map(({ header }) => header.seq),
    [0, 1, 2, 3, 4, 5],
  );
  assert.equal(records.some((record) => (
    record.header?.requestId === requestId
    && ['request.cancel', 'response.error'].includes(record.type)
  )), false, 'successful request must not emit cancel/error');
  return { requestId, responseFrameTypes: responseFrames.map(({ type }) => type) };
}

async function createGatedUpstream() {
  const routes = [];
  const streams = new Map();
  const closedStreams = new Set();
  let release = false;
  let twoStreamsOpenBeforeRelease = false;
  const server = http.createServer((request, response) => {
    routes.push({ method: request.method, path: request.url });
    request.resume();
    if (request.method !== 'GET') {
      response.writeHead(405).end();
      return;
    }
    if (request.url === '/request') {
      response.writeHead(200, { 'content-type': 'application/octet-stream' });
      response.end('UNARY');
      return;
    }
    const fixture = streamFixture(request.url);
    if (fixture === undefined) {
      response.writeHead(404).end();
      return;
    }
    response.writeHead(200, { 'content-type': 'application/octet-stream' });
    response.flushHeaders();
    streams.set(request.url, response);
    response.once('close', () => {
      streams.delete(request.url);
      closedStreams.add(request.url);
    });
    if (!release && streams.has('/stream/left') && streams.has('/stream/right')) {
      twoStreamsOpenBeforeRelease = true;
    }
    if (release) writeFixture(response, fixture);
  });
  await new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolvePromise);
  });
  const address = server.address();
  assert(address !== null && typeof address === 'object');
  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    routes,
    get twoStreamsOpenBeforeRelease() { return twoStreamsOpenBeforeRelease; },
    async waitForTwoOpenStreams() {
      const deadline = Date.now() + 7_000;
      while (Date.now() < deadline) {
        if (twoStreamsOpenBeforeRelease) return;
        await delay(10);
      }
      throw new Error(`two outbound stream heads did not coexist; routes=${JSON.stringify(routes)}`);
    },
    async waitForTwoClosedStreams() {
      const deadline = Date.now() + 7_000;
      while (Date.now() < deadline) {
        if (closedStreams.has('/stream/left') && closedStreams.has('/stream/right')) return;
        await delay(10);
      }
      throw new Error(
        `client disconnect did not close both provider streams; closed=${JSON.stringify([...closedStreams])}`,
      );
    },
    releaseBodies() {
      if (release) return;
      release = true;
      for (const [path, response] of streams) {
        const fixture = streamFixture(path);
        if (fixture !== undefined) writeFixture(response, fixture);
      }
    },
    close() {
      return new Promise((resolvePromise, reject) => {
        server.close((error) => error === undefined ? resolvePromise() : reject(error));
        server.closeAllConnections?.();
      });
    },
  };
}

function streamFixture(path) {
  if (path === '/stream/left') return ['LEFT-A', 'LEFT-B'];
  if (path === '/stream/right') return ['RIGHT-A', 'RIGHT-B'];
  return undefined;
}

function writeFixture(response, chunks) {
  if (response.writableEnded || response.destroyed) return;
  response.write(chunks[0]);
  setImmediate(() => {
    if (!response.writableEnded && !response.destroyed) response.end(chunks[1]);
  });
}

function exactObject(actual, expected) {
  return actual !== null
    && typeof actual === 'object'
    && Object.keys(expected).every((key) => actual[key] === expected[key])
    && Object.keys(actual).length === Object.keys(expected).length;
}
