import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import {
  access,
  chmod,
  mkdtemp,
  mkdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { createRequire } from 'node:module';
import http from 'node:http';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const stressPath = join(root, 'scripts', 'check-loop-risk-stress-live.mjs');
const routerRequire = createRequire(join(root, 'router', 'package.json'));

test('stress help needs no target and never opens the supplied target', async () => {
  const probe = await startProbeServer();
  try {
    for (const args of [[], ['--ws-url', `${probe.wsUrl}/unused?token=a=b`]]) {
      const result = await runStress(['--help', ...args]);
      assert.equal(result.code, 0, result.stderr);
    }
    assert.equal(probe.connections, 0);
  } finally {
    await probe.close();
  }
});

test('stress rejects missing health, log, and CPU targets before any I/O', async () => {
  const probe = await startProbeServer();
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-loop-stress-preflight-'));
  const logFile = join(tempRoot, 'runtime.log');
  await writeFile(logFile, '');
  try {
    const cases = [
      {
        args: ['--ws-url', probe.wsUrl, '--runtime-pid', String(process.pid), '--runtime-log', logFile],
        error: /--health-url and --runtime-id\(s\) are required/,
      },
      {
        args: [
          '--ws-url', probe.wsUrl,
          '--health-url', `${probe.httpUrl}/health`,
          '--runtime-id', 'runtime-test',
          '--runtime-pid', String(process.pid),
        ],
        error: /--runtime-log or --log-file is required/,
      },
      {
        args: [
          '--ws-url', probe.wsUrl,
          '--health-url', `${probe.httpUrl}/health`,
          '--runtime-id', 'runtime-test',
          '--runtime-log', logFile,
        ],
        error: /--runtime-pid or --runtime-pgrep is required/,
      },
    ];
    for (const { args, error } of cases) {
      const result = await runStress(args);
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, error);
    }
    assert.equal(probe.connections, 0);
  } finally {
    await probe.close();
    await rm(tempRoot, { recursive: true, force: true });
  }
});

test('stress strict parser fails closed before loading WebSocket', async () => {
  const probe = await startProbeServer();
  const base = ['--ws-url', probe.wsUrl, '--skip-health', '--skip-cpu', '--skip-log-check'];
  try {
    const cases = [
      { args: [...base, '--unknown'], error: /unknown option --unknown/ },
      { args: ['positional', ...base], error: /unexpected positional argument/ },
      { args: [...base, '--messages'], error: /--messages requires a value/ },
      {
        args: [...base, `--ws-url=${probe.wsUrl}/other`],
        error: /--ws-url was provided more than once/,
      },
      { args: [...base, '--skip-health'], error: /--skip-health was provided more than once/ },
      { args: ['--skip-health=true', ...base], error: /does not accept a value/ },
      { args: [...base, 'true'], error: /unexpected positional argument/ },
      { args: [...base, '--messages', '0'], error: /--messages must be a positive integer/ },
      {
        args: [...base, '--runtime-pid', `123,not-a-pid`],
        error: /must contain only positive integers/,
      },
      { args: [...base, '--header='], error: /--header requires a value/ },
    ];
    for (const { args, error } of cases) {
      const result = await runStress(args);
      assert.notEqual(result.code, 0, `unexpected success for ${args.join(' ')}`);
      assert.match(result.stderr, error);
    }
    assert.equal(probe.connections, 0);
  } finally {
    await probe.close();
  }
});

test('stress only invokes pgrep for an explicit runtime-pgrep diagnostic', async () => {
  const probe = await startProbeServer();
  const fake = await fakePgrepFixture();
  const base = ['--ws-url', probe.wsUrl, '--skip-health', '--skip-log-check'];
  try {
    const missing = await runStress(base, fake.env);
    assert.notEqual(missing.code, 0);
    assert.match(missing.stderr, /--runtime-pid or --runtime-pgrep is required/);
    await assert.rejects(access(fake.marker), { code: 'ENOENT' });

    const explicit = await runStress([...base, '--runtime-pgrep', '^explicit-runtime$'], fake.env);
    assert.notEqual(explicit.code, 0);
    assert.match(explicit.stderr, /no runtime pid found/);
    assert.equal(await readMarker(fake.marker), '^explicit-runtime$');
    assert.equal(probe.connections, 0);
  } finally {
    await probe.close();
    await rm(fake.root, { recursive: true, force: true });
  }
});

test('explicit skip mode succeeds, preserves URL equals signs, and redacts output', async () => {
  const websocket = await startWebSocketServer();
  const sentinel = 'stress-target-secret';
  const rawUrl = `ws://user:password@127.0.0.1:${websocket.port}/${sentinel}?token=a=b=c`;
  try {
    const result = await runStress([
      `--ws-url=${rawUrl}`,
      '--header', 'x-list=a,b',
      '--messages', '1',
      '--concurrency', '1',
      '--open-timeout-ms', '500',
      '--close-timeout-ms', '500',
      '--skip-health',
      '--skip-cpu',
      '--skip-log-check',
    ]);
    assert.equal(result.code, 0, result.stderr);
    assert.deepEqual(websocket.requests, [{
      url: `/${sentinel}?token=a=b=c`,
      header: 'a,b',
    }]);
    const output = JSON.parse(result.stdout);
    assert.equal(output.ok, true);
    assert.equal(output.wsUrl, `ws://127.0.0.1:${websocket.port}/<redacted-path>`);
    assert.equal(output.health.checked, false);
    assert.equal(output.cpu.checked, false);
    assert.equal(output.runtimeRequestErrorLogs.checked, false);
    assert.doesNotMatch(result.stdout, /stress-target-secret|password|token|a=b=c/);
  } finally {
    await websocket.close();
  }
});

test('stress scrubs raw targets from fetch and WebSocket errors', async () => {
  const probe = await startProbeServer();
  const websocket = await startWebSocketServer();
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-loop-stress-errors-'));
  const hook = join(tempRoot, 'fetch-hook.mjs');
  const healthSentinel = 'stress-health-error-secret';
  const healthUrl = `http://user:${healthSentinel}@router.invalid/private?token=a=b`;
  await writeFile(hook, [
    'globalThis.fetch = async (url) => {',
    '  throw new Error(`third-party fetch failed for ${url}`);',
    '};',
    '',
  ].join('\n'));
  try {
    const fetchFailure = await runStress([
      '--ws-url', `ws://127.0.0.1:${websocket.port}/runtime`,
      '--health-url', healthUrl,
      '--runtime-id', 'runtime-test',
      '--messages', '1',
      '--health-timeout-ms', '1',
      '--skip-cpu',
      '--skip-log-check',
    ], { NODE_OPTIONS: `--import=${hook}` });
    assert.notEqual(fetchFailure.code, 0);
    assert.match(fetchFailure.stderr, /http:\/\/router\.invalid\/<redacted-path>/);
    assert.doesNotMatch(fetchFailure.stderr, new RegExp(healthSentinel));
    assert.doesNotMatch(fetchFailure.stderr, /user:|private|token|a=b/);

    const wsSentinel = 'stress-ws-error-secret';
    const invalidWsUrl = `ws://user:${wsSentinel}@`;
    const wsFailure = await runStress([
      '--ws-url', invalidWsUrl,
      '--skip-health',
      '--skip-cpu',
      '--skip-log-check',
      '--messages', '1',
      '--open-timeout-ms', '1',
      '--close-timeout-ms', '1',
    ]);
    assert.notEqual(wsFailure.code, 0);
    assert.match(wsFailure.stderr, /<invalid-url>\/<redacted-path>/);
    assert.doesNotMatch(wsFailure.stderr, new RegExp(wsSentinel));
    assert.doesNotMatch(wsFailure.stderr, /user:/);
    assert.equal(probe.connections, 0);
  } finally {
    await probe.close();
    await websocket.close();
    await rm(tempRoot, { recursive: true, force: true });
  }
});

test('canonical stress config runs all three checked gates and rejects every override', async () => {
  const websocket = await startWebSocketServer();
  const health = await startCanonicalHealthServer();
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-loop-stress-canonical-'));
  const logFile = join(tempRoot, 'runtime.log');
  const configPath = join(tempRoot, 'loop-risk.json');
  const sentinel = 'canonical-stress-secret';
  const wsUrl = `ws://127.0.0.1:${websocket.port}/${sentinel}?token=a=b`;
  await writeFile(logFile, '');
  await writeFile(configPath, JSON.stringify({
    healthUrl: health.url,
    runtimeIds: ['runtime-test'],
    stress: {
      wsUrl,
      runtimePids: [process.pid],
      runtimeLogs: [logFile],
    },
  }));
  try {
    const result = await runStress([
      '--config', configPath,
      '--messages', '1',
      '--concurrency', '1',
      '--cpu-seconds', '1',
      '--cpu-median-threshold', '10000',
      '--cpu-post-grace-threshold', '10000',
      '--cpu-grace-seconds', '0',
    ]);
    assert.equal(result.code, 0, result.stderr);
    assert.equal((result.stdout.match(/"checked": true/g) ?? []).length, 3);
    assert.doesNotMatch(result.stdout, new RegExp(sentinel));
    assert.doesNotMatch(result.stdout, /token|a=b/);
    assert.deepEqual(websocket.requests.map((entry) => entry.url), [
      `/${sentinel}?token=a=b`,
    ]);
    assert.deepEqual(health.requests, ['/__router/health?detail=loop-risk']);

    for (const { args, env } of [
      { args: ['--config', configPath, '--skip-health'] },
      { args: ['--config', configPath, '--runtime-pid', String(process.pid)] },
      { args: ['--config', configPath, '--ws-url', wsUrl] },
      { args: ['--config', configPath], env: { SKIFF_LOOP_RISK_WS_URL: wsUrl } },
    ]) {
      const mixed = await runStress(args, env);
      assert.notEqual(mixed.code, 0);
      assert.match(mixed.stderr, /cannot be combined/);
      assert.doesNotMatch(mixed.stderr, new RegExp(sentinel));
    }
    assert.equal(websocket.requests.length, 1);
    assert.equal(health.requests.length, 1);
  } finally {
    await websocket.close();
    await health.close();
    await rm(tempRoot, { recursive: true, force: true });
  }
});

async function startProbeServer() {
  let connections = 0;
  const server = net.createServer((socket) => {
    connections += 1;
    socket.destroy();
  });
  await listen(server);
  const { port } = server.address();
  return {
    httpUrl: `http://127.0.0.1:${port}`,
    wsUrl: `ws://127.0.0.1:${port}/runtime`,
    get connections() { return connections; },
    close: () => closeServer(server),
  };
}

async function startWebSocketServer() {
  const { WebSocketServer } = routerRequire('ws');
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  const requests = [];
  server.on('connection', (socket, request) => {
    requests.push({ url: request.url, header: request.headers['x-list'] });
    socket.on('error', () => {});
  });
  await new Promise((resolvePromise, reject) => {
    server.once('listening', resolvePromise);
    server.once('error', reject);
  });
  return {
    port: server.address().port,
    requests,
    close: () => new Promise((resolvePromise, reject) => {
      server.close((error) => error ? reject(error) : resolvePromise());
    }),
  };
}

async function startCanonicalHealthServer() {
  const requests = [];
  const server = http.createServer((request, response) => {
    requests.push(request.url);
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({
      loopRisk: {
        observedAt: '2026-07-14T00:00:00.000Z',
        router: {
          dispatcher: { pendingUnary: 0, pendingStream: 0 },
          httpStream: { backpressureWaiters: 0, backpressureCancels: 0 },
        },
        runtimes: [{
          runtimeId: 'runtime-test',
          connected: true,
          fresh: true,
          counters: {
            outboundRequestsPending: 0,
            outboundStreamLeasesActive: 0,
            streamRuntimeStreamsActive: 0,
            flagBackedCancelWaitersActive: 0,
            spawnedTasksActive: 0,
          },
        }],
      },
    }));
  });
  await listen(server);
  const { port } = server.address();
  return {
    url: `http://127.0.0.1:${port}/__router/health?detail=loop-risk`,
    requests,
    close: () => closeServer(server),
  };
}

async function fakePgrepFixture() {
  const fixtureRoot = await mkdtemp(join(tmpdir(), 'skiff-loop-pgrep-'));
  const bin = join(fixtureRoot, 'bin');
  const marker = join(fixtureRoot, 'pgrep-marker');
  const pgrep = join(bin, 'pgrep');
  await mkdir(bin);
  await writeFile(pgrep, [
    '#!/usr/bin/env node',
    "const fs = require('node:fs');",
    "fs.writeFileSync(process.env.SKIFF_PGREP_MARKER, process.argv.at(-1));",
    'process.exit(1);',
    '',
  ].join('\n'));
  await chmod(pgrep, 0o755);
  return {
    root: fixtureRoot,
    marker,
    env: {
      PATH: `${bin}${delimiter}${process.env.PATH ?? ''}`,
      SKIFF_PGREP_MARKER: marker,
    },
  };
}

async function readMarker(path) {
  return await readFile(path, 'utf8');
}

function runStress(args, envOverrides = {}) {
  const env = { ...process.env, ...envOverrides };
  if (!Object.hasOwn(envOverrides, 'SKIFF_LOOP_RISK_WS_URL')) {
    delete env.SKIFF_LOOP_RISK_WS_URL;
  }
  return runProcess(process.execPath, [stressPath, ...args], env);
}

function runProcess(command, args, env) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', (code, signal) => resolvePromise({ code, signal, stdout, stderr }));
  });
}

function listen(server) {
  return new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolvePromise);
  });
}

function closeServer(server) {
  return new Promise((resolvePromise, reject) => {
    server.close((error) => error ? reject(error) : resolvePromise());
  });
}
