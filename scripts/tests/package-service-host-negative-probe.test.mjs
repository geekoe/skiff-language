import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer, request as requestHttp } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  packageServiceHostNegativeProbeConstants,
  runPackageServiceHostNegativeProbe,
  startTransparentCountingProxy,
} from '../lib/package-service-host-negative-probe.mjs';

const configSnapshotId =
  `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`;

test('command-double runs only one copied-consumer negative Host probe', async () => {
  const root = await makeCheckout();
  const fixtureTest = join(
    root,
    'test-runner/fixtures/package-service-host/consumer-tests/main.test.skiff',
  );
  const fixtureBefore = await readFile(fixtureTest, 'utf8');
  const commands = [];
  const proxyState = { requests: [] };
  try {
    const result = await runPackageServiceHostNegativeProbe({
      skiffRoot: root,
      graceWindowMs: 25,
      wait: async () => {},
      runtimeOwner: commandDoubleRuntimeOwner,
      readHostReceipt: async (path, profile) => {
        assert.match(path, /package-service-host-receipt\.json$/);
        assert.equal(profile, 'skiff-test');
        return {
          baseConfigSnapshot: { snapshotId: configSnapshotId },
        };
      },
      runCommand: async (command, args, options) => {
        commands.push({ command, args, options });
        if (args.includes('--prepare-host-base')) return commandOutcome(0);
        const inputRoot = args.at(args.indexOf('--') + 1);
        assert.match(inputRoot, /package-service-host-negative-consumer$/);
        assert.equal(
          await readFile(join(inputRoot, 'main.test.skiff'), 'utf8'),
          packageServiceHostNegativeProbeConstants.negativeTestSource,
        );
        assert.equal(options.env.SKIFF_TEST_INGRESS_URL, 'http://127.0.0.1:49151');
        proxyState.requests.push(failedRequest());
        return commandOutcome(1, {
          stdout: [
            'FAIL main.__test::provider observes helper mutation',
            '  HTTP 500: {"message":"assertion failed"}',
            '',
          ].join('\n'),
          stderr: 'error: 1 test(s) failed\n',
        });
      },
      startProxy: async ({ targetUrl }) => {
        assert.equal(targetUrl, 'http://127.0.0.1:46100');
        return commandDoubleProxy(proxyState);
      },
      inspectCleanup: async (resources) => ({
        ...resources,
        supervisorPidStopped: true,
        isolatedPortsClosed: true,
        proxyPortClosed: true,
        tempRootRemoved: true,
      }),
    });

    assert.equal(result.verdict, 'PASS');
    assert.equal(result.fullProbeRuns, 0);
    assert.equal(result.negativeProbeRuns, 1);
    assert.deepEqual(result.commands, {
      hostPreparerRuns: 1,
      canonicalRunnerRuns: 1,
      sourceSuiteRuns: 0,
    });
    assert.equal(result.fixture.checkedInUnchanged, true);
    assert.equal(result.fixture.checkedInSha256After, result.fixture.checkedInSha256Before);
    assert.deepEqual(result.fixture.copiedConsumerModifiedFiles, ['main.test.skiff']);
    assert.deepEqual(result.fixture.copiedExternalControlFiles, [{
      path: 'http.yml',
      sha256: createHash('sha256').update(TEMP_HTTP_MANIFEST).digest('hex'),
      size: Buffer.byteLength(TEMP_HTTP_MANIFEST),
    }]);
    assert.equal(result.runner.exitCode, 1);
    assert.equal(result.runner.failedTests, 1);
    assert.equal(result.runner.failureSummary, '1 test failed');
    assert.equal(result.runner.httpStatus, 500);
    assert.equal(result.ingress.requestCount, 1);
    assert.equal(result.ingress.retryCount, 0);
    assert.equal(result.ingress.syntheticResponses, 0);
    assert.equal(commands.length, 2);
    assert.equal(commands[0].args.includes('--prepare-host-base'), true);
    assert.equal(commands[1].args.includes('--base-config-snapshot'), true);
    assert.equal(commands[1].args.includes('--deny-skips'), true);
    assert.equal(commands[1].args.includes('--require-tests'), true);
    assert.equal(commands[1].args.includes('--live'), false);
    assert.equal(await readFile(fixtureTest, 'utf8'), fixtureBefore);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('command-double rejects retry, synthesized response, or a missing Runtime diagnostic', async () => {
  for (const [name, mutate, expected] of [
    [
      'retry',
      (state) => state.requests.push(failedRequest()),
      /expected 1 business ingress request\(s\).*after the retry grace window/,
    ],
    [
      'synthetic response',
      (state) => { state.syntheticResponses = 1; },
      /must never synthesize a response/,
    ],
    [
      'missing assertion diagnostic',
      (state) => { state.runnerDiagnostic = 'runtime failed'; },
      /omitted the Runtime assertion diagnostic/,
    ],
  ]) {
    const root = await makeCheckout();
    const state = { requests: [], syntheticResponses: 0, runnerDiagnostic: 'assertion failed' };
    try {
      await assert.rejects(
        runPackageServiceHostNegativeProbe({
          skiffRoot: root,
          graceWindowMs: 25,
          runtimeOwner: commandDoubleRuntimeOwner,
          readHostReceipt: async () => ({
            baseConfigSnapshot: { snapshotId: configSnapshotId },
          }),
          runCommand: async (_command, args) => {
            if (args.includes('--prepare-host-base')) return commandOutcome(0);
            state.requests.push(failedRequest());
            if (name !== 'retry') mutate(state);
            return commandOutcome(1, {
              stdout: [
                'FAIL main.__test::provider observes helper mutation',
                `  HTTP 500: {"message":"${state.runnerDiagnostic}"}`,
              ].join('\n'),
              stderr: 'error: 1 test(s) failed\n',
            });
          },
          startProxy: async () => commandDoubleProxy(state),
          wait: async () => {
            if (name === 'retry') mutate(state);
          },
        }),
        expected,
      );
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }
});

test('counting proxy forwards Host, method, path, and body and mirrors Router response', async () => {
  const observed = [];
  const router = createServer(async (request, response) => {
    const body = [];
    for await (const chunk of request) body.push(chunk);
    observed.push({
      host: request.headers.host,
      method: request.method,
      path: request.url,
      body: Buffer.concat(body).toString('utf8'),
    });
    response.writeHead(503, { 'content-type': 'application/json', 'x-router': 'real' });
    response.end('{"message":"assertion failed"}');
  });
  const routerPort = await listen(router);
  const proxy = await startTransparentCountingProxy({
    targetUrl: `http://127.0.0.1:${routerPort}`,
  });
  try {
    const response = await makeRequest(proxy.url, {
      host: 'consumer.skiff.localhost',
      method: 'POST',
      path: '/tests/run?case=false',
      body: 'opaque-body',
    });
    await proxy.waitForIdle();
    assert.deepEqual(observed, [{
      host: 'consumer.skiff.localhost',
      method: 'POST',
      path: '/tests/run?case=false',
      body: 'opaque-body',
    }]);
    assert.deepEqual(response, {
      status: 503,
      routerHeader: 'real',
      body: '{"message":"assertion failed"}',
    });
    const snapshot = proxy.snapshot();
    assert.equal(snapshot.requestCount, 1);
    assert.equal(snapshot.syntheticResponses, 0);
    assert.deepEqual(snapshot.requests[0], {
      host: 'consumer.skiff.localhost',
      method: 'POST',
      path: '/tests/run?case=false',
      bodyBytes: 11,
      bodyBase64: Buffer.from('opaque-body').toString('base64'),
      bodySha256: '760e54f34bcf2d53afc2b50c29bdf03d0d632c07f5e98a07238daffade62eb59',
      responseStatus: 503,
      forwarded: true,
    });
  } finally {
    await proxy.close();
    await close(router);
  }
});

async function makeCheckout() {
  const root = await mkdtemp(join(tmpdir(), 'skiff-host-negative-test-'));
  const consumer = join(root, 'test-runner/fixtures/package-service-host/consumer');
  const tests = join(root, 'test-runner/fixtures/package-service-host/consumer-tests');
  await mkdir(consumer, { recursive: true });
  await mkdir(tests, { recursive: true });
  await writeFile(join(consumer, 'api.yml'), 'run: main.run\n');
  await writeFile(join(consumer, 'main.skiff'), 'function run() -> string { return "ok" }\n');
  await writeFile(join(consumer, 'package.yml'), 'id: example.com/consumer\nversion: 1.0.0\n');
  await writeFile(join(tests, 'api.yml'), '{}\n');
  await writeFile(
    join(tests, 'package.yml'),
    [
      'id: test.skiff/consumer-tests',
      'version: 1.0.0',
      'packages:',
      '  - id: example.com/consumer',
      '    version: 1.0.0',
      '    alias: subject',
      '    topLevelAlias: subjectImpl',
      '',
    ].join('\n'),
  );
  await writeFile(
    join(tests, 'service.yml'),
    'id: test.skiff/consumer-tests\nkind: test\n',
  );
  await writeFile(join(tests, 'http.yml'), TEMP_HTTP_MANIFEST);
  await writeFile(join(tests, 'config.skiff-test.yml'), '{}\n');
  await writeFile(
    join(tests, 'main.skiff'),
    'function probe(body: null) -> string { return "ok" }\n',
  );
  await writeFile(
    join(tests, 'main.test.skiff'),
    'import subjectImpl\n\ntest "positive" {\n  assert subjectImpl/main.run() == "ok"\n}\n',
  );
  return root;
}

const TEMP_HTTP_MANIFEST = [
  'probe:',
  '  method: POST',
  '  path: /probe',
  '  kind: typedJson',
  '  handler: main.probe',
  '  adapterArgs:',
  '    - param: body',
  '      source: { kind: http.body }',
  '',
].join('\n');

async function commandDoubleRuntimeOwner({ runTest }) {
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-host-negative-runtime-'));
  await mkdir(join(tempRoot, 'source-artifacts'));
  try {
    return await runTest(
      {
        SKIFF_TEST_ENVIRONMENT: 'skiff-test',
        SKIFF_TEST_INGRESS_URL: 'http://127.0.0.1:46100',
      },
      new AbortController().signal,
      {
        tempRoot,
        sourceArtifactRoot: join(tempRoot, 'source-artifacts'),
        routerHttpUrl: 'http://127.0.0.1:46100',
        ports: [46100, 46101, 46102],
        supervisor: { pid: 98765 },
      },
    );
  } finally {
    await rm(tempRoot, { recursive: true, force: true });
  }
}

function commandDoubleProxy(state) {
  return {
    url: 'http://127.0.0.1:49151',
    port: 49151,
    snapshot: () => ({
      requestCount: state.requests.length,
      syntheticResponses: state.syntheticResponses ?? 0,
      requests: state.requests.map((request) => ({ ...request })),
    }),
    waitForIdle: async () => {},
    close: async () => {},
  };
}

function commandOutcome(code, { stdout = '', stderr = '' } = {}) {
  return { code, signal: null, error: null, stdout, stderr };
}

function failedRequest() {
  return {
    host: 'consumer.skiff.localhost',
    method: 'POST',
    path: '/tests/run',
    bodyBytes: 0,
    bodyBase64: '',
    bodySha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
    responseStatus: 500,
    forwarded: true,
  };
}

function listen(server) {
  return new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject);
      resolvePromise(server.address().port);
    });
  });
}

function close(server) {
  return new Promise((resolvePromise, reject) => {
    server.close((error) => error ? reject(error) : resolvePromise());
  });
}

function makeRequest(baseUrl, { host, method, path, body }) {
  return new Promise((resolvePromise, reject) => {
    const url = new URL(path, baseUrl);
    const request = requestHttp(url, {
      method,
      headers: { host, 'content-length': Buffer.byteLength(body) },
    }, (response) => {
      const chunks = [];
      response.on('data', (chunk) => chunks.push(chunk));
      response.once('error', reject);
      response.once('end', () => resolvePromise({
        status: response.statusCode,
        routerHeader: response.headers['x-router'],
        body: Buffer.concat(chunks).toString('utf8'),
      }));
    });
    request.once('error', reject);
    request.end(body);
  });
}
