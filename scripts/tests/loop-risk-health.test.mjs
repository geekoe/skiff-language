import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { parseLoopRiskConfig } from '../lib/loop-risk-config.mjs';
import { pollLoopRiskHealth } from '../lib/loop-risk-health.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const checkerPath = join(root, 'scripts', 'check-loop-risk-health.mjs');

test('health requires an explicit target while help and self-test stay offline', async () => {
  const server = await startHealthServer(zeroHealth());
  try {
    const missing = await runChecker([]);
    assert.notEqual(missing.code, 0);
    assert.match(missing.stderr, /--url is required/);

    for (const args of [
      ['--help'],
      ['--self-test'],
      ['--help', '--url', server.url],
      ['--self-test', `--url=${server.url}`],
    ]) {
      const result = await runChecker(args);
      assert.equal(result.code, 0, result.stderr);
    }
    assert.equal(server.requests.length, 0);
  } finally {
    await server.close();
  }
});

test('health strict parser rejects malformed, duplicate, and invalid arguments before fetch', async () => {
  const server = await startHealthServer(zeroHealth());
  try {
    const cases = [
      { args: ['--url', server.url, '--unknown'], error: /unknown option --unknown/ },
      { args: ['positional', '--url', server.url], error: /unexpected positional argument/ },
      { args: ['--url'], error: /--url requires a value/ },
      {
        args: ['--url', server.url, `--url=${server.url}`],
        error: /--url was provided more than once/,
      },
      { args: ['--self-test', '--self-test'], error: /provided more than once/ },
      { args: ['--self-test=true'], error: /does not accept a value/ },
      { args: ['--self-test', 'true'], error: /unexpected positional argument/ },
      {
        args: ['--url', server.url, '--timeout-ms', 'not-a-number'],
        error: /--timeout-ms must be a positive integer/,
      },
      {
        args: ['--url', server.url, '--runtime-id=,'],
        error: /--runtime-id requires a non-empty value/,
      },
    ];
    for (const { args, error } of cases) {
      const result = await runChecker(args);
      assert.notEqual(result.code, 0, `unexpected success for ${args.join(' ')}`);
      assert.match(result.stderr, error);
    }
    assert.equal(server.requests.length, 0);
  } finally {
    await server.close();
  }
});

test('health preserves inline URL equals signs for execution but redacts target output', async () => {
  const sentinel = 'health-path-secret';
  const server = await startHealthServer(zeroHealth());
  try {
    const rawUrl = `${server.origin}/${sentinel}?detail=loop-risk&token=a=b=c`;
    const result = await runChecker([`--url=${rawUrl}`]);
    assert.equal(result.code, 0, result.stderr);
    assert.deepEqual(server.requests, [`/${sentinel}?detail=loop-risk&token=a=b=c`]);
    assert.match(result.stdout, new RegExp(`${escapeRegExp(server.origin)}/<redacted-path>`));
    assert.doesNotMatch(result.stdout, /health-path-secret|token|a=b=c/);
  } finally {
    await server.close();
  }
});

test('health scrubs a known raw URL embedded by a third-party fetch error', async () => {
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-loop-health-'));
  const hook = join(tempRoot, 'fetch-hook.mjs');
  const sentinel = 'fetch-error-secret';
  const rawUrl = `http://user:${sentinel}@router.invalid/private/${sentinel}?token=a=b`;
  await writeFile(hook, [
    'globalThis.fetch = async (url) => {',
    '  throw new Error(`third-party fetch failed for ${url}`);',
    '};',
    '',
  ].join('\n'));
  try {
    const result = await runChecker(
      ['--url', rawUrl, '--timeout-ms', '1', '--interval-ms', '1'],
      { NODE_OPTIONS: `--import=${hook}` },
    );
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /http:\/\/router\.invalid\/<redacted-path>/);
    assert.doesNotMatch(result.stderr, new RegExp(sentinel));
    assert.doesNotMatch(result.stderr, /user:|private|token|a=b/);
  } finally {
    await rm(tempRoot, { recursive: true, force: true });
  }
});

test('health canonical config is strict, cannot be overridden, and redacts output', async () => {
  const server = await startHealthServer(zeroHealth());
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-loop-health-config-'));
  const configPath = join(tempRoot, 'loop-risk.json');
  const sentinel = 'canonical-health-secret';
  const healthUrl = `${server.origin}/__router/health?detail=loop-risk`;
  await writeFile(configPath, JSON.stringify({
    healthUrl,
    runtimeIds: ['runtime-test'],
  }));
  try {
    const result = await runChecker(['--config', configPath]);
    assert.equal(result.code, 0, result.stderr);
    assert.match(result.stdout, /"checked": true/);
    assert.match(result.stdout, new RegExp(`${escapeRegExp(server.origin)}/<redacted-path>`));
    assert.deepEqual(server.requests, ['/__router/health?detail=loop-risk']);

    const mixed = await runChecker([
      '--config', configPath,
      '--url', `${server.origin}/${sentinel}?detail=loop-risk`,
    ]);
    assert.notEqual(mixed.code, 0);
    assert.match(mixed.stderr, /cannot be combined/);
    assert.doesNotMatch(mixed.stderr, new RegExp(sentinel));
    assert.equal(server.requests.length, 1);
  } finally {
    await server.close();
    await rm(tempRoot, { recursive: true, force: true });
  }
});

test('health config schema rejects required-field, URL, type, duplicate, and unknown-key errors', () => {
  const valid = {
    healthUrl: 'http://router.test:4101/__router/health?detail=loop-risk',
    runtimeIds: ['runtime-a'],
  };
  assert.deepEqual(parseLoopRiskConfig(valid), valid);
  for (const [value, expected] of [
    [{ runtimeIds: ['runtime-a'] }, /missing field.*healthUrl/],
    [{ ...valid, runtimeIds: [] }, /runtimeIds must be a non-empty array/],
    [{ ...valid, runtimeIds: ['runtime-a', 'runtime-a'] }, /unique/],
    [{ ...valid, runtimeIds: [1] }, /runtimeIds must be/],
    [{ ...valid, healthUrl: 'http://router.test:4101/other?detail=loop-risk' }, /must target/],
    [{ ...valid, healthUrl: 'http://user:secret@router.test:4101/__router/health?detail=loop-risk' }, /must target/],
    [{ ...valid, extra: true }, /unknown field.*extra/],
  ]) {
    assert.throws(() => parseLoopRiskConfig(value), expected);
  }
});

test('shared health poller covers convergence, timeout, transport/schema errors, and runtime loss', async () => {
  const zero = zeroHealth();
  const nonzero = structuredClone(zero);
  nonzero.loopRisk.router.dispatcher.pendingUnary = 1;
  const missingRuntime = structuredClone(zero);
  missingRuntime.loopRisk.runtimes = [];
  const missingHttpStreamWaiters = structuredClone(zero);
  delete missingHttpStreamWaiters.loopRisk.router.httpStream.backpressureWaiters;

  const converged = await pollWithResponses([nonzero, zero]);
  assert.equal(converged.ok, true);
  assert.equal(converged.runtimes[0].runtimeId, 'runtime-test');

  const timeout = await pollWithResponses([nonzero]);
  assert.equal(timeout.ok, false);
  assert.match(timeout.reasons.join('\n'), /pendingUnary/);

  const httpError = await pollWithResponses([], { status: 503 });
  assert.equal(httpError.ok, false);
  assert.match(httpError.latestError, /returned 503/);

  const missingSchema = await pollWithResponses([{}]);
  assert.equal(missingSchema.ok, false);
  assert.match(missingSchema.latestError, /did not include loopRisk/);

  const disappeared = await pollWithResponses([missingRuntime]);
  assert.equal(disappeared.ok, false);
  assert.match(disappeared.reasons.join('\n'), /disappeared/);

  const missingField = await pollWithResponses([missingHttpStreamWaiters]);
  assert.equal(missingField.ok, false);
  assert.match(missingField.reasons.join('\n'), /backpressureWaiters/);
});

test('importing loop-risk CLIs and health library performs no main execution', async () => {
  for (const path of [
    checkerPath,
    join(root, 'scripts', 'lib', 'loop-risk-health.mjs'),
  ]) {
    const result = await runProcess(process.execPath, [
      '--input-type=module',
      '--eval',
      `await import(${JSON.stringify(pathToFileURL(path).href)})`,
    ], process.env);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(result.stdout, '');
    assert.equal(result.stderr, '');
  }
});

async function pollWithResponses(payloads, { status = 200 } = {}) {
  let index = 0;
  let now = 0;
  return await pollLoopRiskHealth({
    url: 'http://router.test:4101/__router/health?detail=loop-risk',
    touchedRuntimeIds: ['runtime-test'],
    timeoutMs: 3,
    intervalMs: 1,
  }, {
    now: () => now++,
    sleep: async () => {},
    fetch: async () => ({
      ok: status >= 200 && status < 300,
      status,
      json: async () => payloads[Math.min(index++, payloads.length - 1)],
    }),
  });
}

function zeroHealth() {
  return {
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
          taskRequestsActive: 0,
        },
      }],
    },
  };
}

async function startHealthServer(payload) {
  const requests = [];
  const server = http.createServer((request, response) => {
    requests.push(request.url);
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify(payload));
  });
  await new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolvePromise);
  });
  const { port } = server.address();
  return {
    origin: `http://127.0.0.1:${port}`,
    url: `http://127.0.0.1:${port}/health?detail=loop-risk`,
    requests,
    close: () => new Promise((resolvePromise, reject) => {
      server.close((error) => error ? reject(error) : resolvePromise());
    }),
  };
}

function runChecker(args, envOverrides = {}) {
  return runProcess(process.execPath, [checkerPath, ...args], {
    ...process.env,
    ...envOverrides,
  });
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

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
