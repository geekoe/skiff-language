import assert from 'node:assert/strict';
import http from 'node:http';
import net from 'node:net';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import {
  createLocalIngress,
  loadLocalIngressConfig,
  normalizeLocalIngressHost,
  parseLocalIngressArgs,
  validateLocalIngressConfig,
} from '../local-ingress.mjs';

const ROUTES = {
  'agine.localhost': { service: 'agine.ai/api', version: '0.1.0' },
};

test('local ingress normalizes request Host case and ignores its port', () => {
  assert.equal(normalizeLocalIngressHost('AgInE.Localhost:4003'), 'agine.localhost');
  assert.equal(normalizeLocalIngressHost('agine.localhost'), 'agine.localhost');
  assert.equal(normalizeLocalIngressHost('[::1]:4003'), '::1');
  assert.equal(normalizeLocalIngressHost('agine.localhost:bad'), null);
  assert.equal(normalizeLocalIngressHost('agine.localhost,evil.localhost'), null);
});

test('local ingress loads an explicit port config and requires its config path', async () => {
  const temporaryRoot = await mkdtemp(path.join(os.tmpdir(), 'skiff-local-ingress-config-'));
  const configPath = path.join(temporaryRoot, 'config.json');
  try {
    await writeFile(configPath, JSON.stringify({
      listen: { host: '127.0.0.1', port: 43123 },
      upstream: { host: '127.0.0.1', port: 43124 },
      hosts: ROUTES,
    }));
    const config = await loadLocalIngressConfig(configPath);
    assert.equal(config.listen.port, 43123);
    assert.equal(config.upstream.port, 43124);
    assert.deepEqual(
      parseLocalIngressArgs(['--config', configPath], {}),
      { configPath },
    );
    assert.deepEqual(
      parseLocalIngressArgs([], { SKIFF_LOCAL_INGRESS_CONFIG: configPath }),
      { configPath },
    );
    assert.throws(() => parseLocalIngressArgs([], {}), /requires --config/);
    assert.throws(
      () => validateLocalIngressConfig({
        listen: { host: '127.0.0.1', port: -1 },
        upstream: { host: '127.0.0.1', port: 4000 },
        hosts: ROUTES,
      }),
      /listen\.port/,
    );
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
});

test('local ingress streams HTTP and owns trusted selectors', async (context) => {
  let releaseTail;
  const tailGate = new Promise((resolve) => {
    releaseTail = resolve;
  });
  const seen = {};
  const upstream = http.createServer(async (request, response) => {
    seen.method = request.method;
    seen.url = request.url;
    seen.headers = request.headers;
    seen.body = await collectStream(request);
    response.writeHead(207, [
      'Content-Type', 'application/octet-stream',
      'X-Upstream', 'preserved',
      'Set-Cookie', 'a=1',
      'Set-Cookie', 'b=2',
      'Connection', 'keep-alive, x-drop-response',
      'X-Drop-Response', 'secret',
    ]);
    response.write('first:');
    await tailGate;
    response.end('second');
  });
  const upstreamAddress = await listen(upstream);
  context.after(() => closeServer(upstream));

  const ingress = createLocalIngress(configFor(upstreamAddress.port));
  const ingressAddress = await listen(ingress);
  context.after(() => closeServer(ingress));

  const response = await request({
    port: ingressAddress.port,
    method: 'POST',
    path: '/chat/send?keep=yes',
    headers: {
      Host: `AgInE.Localhost:${ingressAddress.port}`,
      'Content-Type': 'application/octet-stream',
      'X-Skiff-Service': 'attacker.invalid/service',
      'x-skiff-version': 'attacker-version',
      Connection: 'close, x-drop-request',
      'X-Drop-Request': 'secret',
    },
    bodyChunks: [Buffer.from('raw-'), Buffer.from([0, 1, 2, 255])],
  });

  const firstChunk = await onceData(response);
  assert.equal(firstChunk.toString(), 'first:');
  assert.equal(response.complete, false);
  releaseTail();
  const responseBody = Buffer.concat([firstChunk, await collectStream(response)]);

  assert.equal(response.statusCode, 207);
  assert.equal(response.headers['x-upstream'], 'preserved');
  assert.deepEqual(response.headers['set-cookie'], ['a=1', 'b=2']);
  assert.equal(response.headers['x-drop-response'], undefined);
  assert.equal(responseBody.toString(), 'first:second');
  assert.equal(seen.method, 'POST');
  assert.equal(seen.url, '/chat/send?keep=yes');
  assert.equal(seen.headers.host, `AgInE.Localhost:${ingressAddress.port}`);
  assert.equal(seen.headers['x-skiff-service'], 'agine.ai/api');
  assert.equal(seen.headers['x-skiff-version'], '0.1.0');
  assert.equal(seen.headers['x-drop-request'], undefined);
  assert.deepEqual(
    seen.body,
    Buffer.concat([Buffer.from('raw-'), Buffer.from([0, 1, 2, 255])]),
  );
});

test('local ingress fails unknown Hosts closed and owns its health endpoint', async (context) => {
  let upstreamRequests = 0;
  const upstream = http.createServer((_request, response) => {
    upstreamRequests += 1;
    response.end('unexpected');
  });
  const upstreamAddress = await listen(upstream);
  context.after(() => closeServer(upstream));
  const ingress = createLocalIngress(configFor(upstreamAddress.port));
  const ingressAddress = await listen(ingress);
  context.after(() => closeServer(ingress));

  const unknown = await requestAndCollect({
    port: ingressAddress.port,
    path: '/business',
    headers: { Host: 'unknown.localhost:4003' },
  });
  assert.equal(unknown.statusCode, 421);
  assert.match(unknown.body.toString(), /unknown local ingress Host/);

  const health = await requestAndCollect({
    port: ingressAddress.port,
    path: '/__local_ingress/health',
    headers: { Host: 'health.localhost:4003' },
  });
  assert.equal(health.statusCode, 200);
  assert.deepEqual(JSON.parse(health.body), { status: 'ok' });
  assert.equal(upstreamRequests, 0);
});

test('local ingress propagates downstream cancellation upstream', async (context) => {
  let resolveUpstreamClosed;
  const upstreamClosed = new Promise((resolve) => {
    resolveUpstreamClosed = resolve;
  });
  const upstream = http.createServer((_request, response) => {
    response.once('close', resolveUpstreamClosed);
    response.write('first');
    const timer = setInterval(() => response.write('tail'), 20);
    response.once('close', () => clearInterval(timer));
  });
  const upstreamAddress = await listen(upstream);
  context.after(() => closeServer(upstream));
  const ingress = createLocalIngress(configFor(upstreamAddress.port));
  const ingressAddress = await listen(ingress);
  context.after(() => closeServer(ingress));

  const response = await request({
    port: ingressAddress.port,
    path: '/cancel',
    headers: { Host: 'agine.localhost:4003' },
  });
  await onceData(response);
  response.destroy();
  await withTimeout(upstreamClosed, 1_000, 'upstream response was not cancelled');
});

test('local ingress returns 502 when its Router upstream is unavailable', async (context) => {
  const unavailable = http.createServer();
  const unavailableAddress = await listen(unavailable);
  await closeServer(unavailable);

  const ingress = createLocalIngress(configFor(unavailableAddress.port));
  const ingressAddress = await listen(ingress);
  context.after(() => closeServer(ingress));

  const response = await requestAndCollect({
    port: ingressAddress.port,
    path: '/session',
    headers: { Host: 'agine.localhost:4003' },
  });
  assert.equal(response.statusCode, 502);
  assert.match(response.body.toString(), /upstream unavailable/);
});

test('local ingress proxies WebSocket upgrade bytes with trusted selectors', async (context) => {
  let seenRequest;
  let upstreamSocket;
  const upstream = http.createServer();
  upstream.on('upgrade', (request, socket) => {
    seenRequest = request;
    upstreamSocket = socket;
    socket.write(
      'HTTP/1.1 101 Switching Protocols\r\n'
      + 'Connection: Upgrade\r\n'
      + 'Upgrade: websocket\r\n'
      + 'X-Upstream-Upgrade: preserved\r\n'
      + '\r\n'
      + 'server-head',
    );
    socket.on('data', (chunk) => socket.write(Buffer.concat([Buffer.from('echo:'), chunk])));
  });
  const upstreamAddress = await listen(upstream);
  context.after(() => {
    upstreamSocket?.destroy();
    return closeServer(upstream);
  });
  const ingress = createLocalIngress(configFor(upstreamAddress.port));
  const ingressAddress = await listen(ingress);
  context.after(() => closeServer(ingress));

  const socket = net.connect({ host: '127.0.0.1', port: ingressAddress.port });
  await onceEvent(socket, 'connect');
  socket.write(
    'GET /ws?platform=web HTTP/1.1\r\n'
    + `Host: AgInE.Localhost:${ingressAddress.port}\r\n`
    + 'Connection: Upgrade, x-drop-upgrade\r\n'
    + 'Upgrade: websocket\r\n'
    + 'Sec-WebSocket-Key: test-key\r\n'
    + 'Sec-WebSocket-Version: 13\r\n'
    + 'X-Skiff-Service: attacker.invalid/service\r\n'
    + 'X-Skiff-Version: attacker-version\r\n'
    + 'X-Drop-Upgrade: secret\r\n'
    + '\r\n'
    + 'client-head',
  );

  const receivedHead = await waitForSocket(
    socket,
    (buffer) => buffer.includes('echo:client-head'),
    1_000,
  );
  socket.write('later-bytes');
  const received = await waitForSocket(
    socket,
    (buffer) => buffer.includes('echo:later-bytes'),
    1_000,
    [receivedHead],
  );

  assert.match(received.toString('latin1'), /^HTTP\/1\.1 101 Switching Protocols\r\n/mu);
  assert.match(received.toString('latin1'), /X-Upstream-Upgrade: preserved\r\n/iu);
  assert.equal(seenRequest.url, '/ws?platform=web');
  assert.equal(seenRequest.headers.host, `AgInE.Localhost:${ingressAddress.port}`);
  assert.equal(seenRequest.headers['x-skiff-service'], 'agine.ai/api');
  assert.equal(seenRequest.headers['x-skiff-version'], '0.1.0');
  assert.equal(seenRequest.headers['x-drop-upgrade'], undefined);

  socket.destroy();
  await onceEvent(socket, 'close');
});

function configFor(upstreamPort) {
  return {
    listen: { host: '127.0.0.1', port: 0 },
    upstream: { host: '127.0.0.1', port: upstreamPort },
    hosts: ROUTES,
  };
}

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject);
      resolve(server.address());
    });
  });
}

function closeServer(server) {
  server.closeAllConnections?.();
  return new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

function request({ port, path: requestPath, method = 'GET', headers = {}, bodyChunks = [] }) {
  return new Promise((resolve, reject) => {
    const outgoing = http.request({
      host: '127.0.0.1',
      port,
      path: requestPath,
      method,
      headers,
    }, resolve);
    outgoing.once('error', reject);
    for (const chunk of bodyChunks) outgoing.write(chunk);
    outgoing.end();
  });
}

async function requestAndCollect(options) {
  const response = await request(options);
  return {
    statusCode: response.statusCode,
    headers: response.headers,
    body: await collectStream(response),
  };
}

function collectStream(stream) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    stream.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
    stream.once('end', () => resolve(Buffer.concat(chunks)));
    stream.once('error', reject);
  });
}

function onceData(stream) {
  return new Promise((resolve, reject) => {
    stream.once('data', resolve);
    stream.once('error', reject);
  });
}

function onceEvent(emitter, event) {
  return new Promise((resolve, reject) => {
    emitter.once(event, resolve);
    emitter.once('error', reject);
  });
}

function waitForSocket(socket, predicate, timeoutMs, initialChunks = []) {
  return new Promise((resolve, reject) => {
    const chunks = [...initialChunks];
    const finish = (callback, value) => {
      clearTimeout(timer);
      socket.off('data', onData);
      socket.off('error', onError);
      callback(value);
    };
    const onData = (chunk) => {
      chunks.push(Buffer.from(chunk));
      const buffer = Buffer.concat(chunks);
      if (predicate(buffer.toString('latin1'))) finish(resolve, buffer);
    };
    const onError = (error) => finish(reject, error);
    const timer = setTimeout(() => {
      finish(
        reject,
        new Error(`timed out waiting for socket bytes: ${Buffer.concat(chunks).toString('latin1')}`),
      );
    }, timeoutMs);
    socket.on('data', onData);
    socket.once('error', onError);
  });
}

function withTimeout(promise, timeoutMs, message) {
  return Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error(message)), timeoutMs)),
  ]);
}
