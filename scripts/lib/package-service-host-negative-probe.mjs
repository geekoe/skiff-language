import { createHash } from 'node:crypto';
import { access, cp, readFile, readdir, writeFile } from 'node:fs/promises';
import { createServer, request as requestHttp } from 'node:http';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { captureAttachedCommand } from './command-execution.mjs';
import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { assertPortsClosed } from './local-port-lease.mjs';
import {
  packageServiceHostFixturePaths,
  packageServiceHostFixturePrepareCargoArgs,
  readPackageServiceHostFixtureReceipt,
  skiffSourceTestRunnerCargoArgs,
} from './skiff-source-test-suite.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultSkiffRoot = resolve(scriptDir, '..', '..');
const PROBE_SCHEMA_VERSION = 'skiff-package-service-host-negative-probe-v1';
const NEGATIVE_TEST_RELATIVE_PATH = 'main.test.skiff';
const DEFAULT_GRACE_WINDOW_MS = 500;
const NEGATIVE_TEST_SOURCE = [
  'test "provider observes helper mutation" {',
  '  assert false',
  '}',
  '',
].join('\n');

export async function runPackageServiceHostNegativeProbe({
  skiffRoot = defaultSkiffRoot,
  runtimeOwner = runInIsolatedTestRuntime,
  runCommand = captureAttachedCommand,
  readHostReceipt = readPackageServiceHostFixtureReceipt,
  startProxy = startTransparentCountingProxy,
  inspectCleanup = inspectNegativeProbeCleanup,
  wait = waitForGraceWindow,
  graceWindowMs = DEFAULT_GRACE_WINDOW_MS,
} = {}) {
  const absoluteSkiffRoot = resolve(skiffRoot);
  const fixtureRoot = resolve(
    absoluteSkiffRoot,
    'test-runner',
    'fixtures',
    'package-service-host',
  );
  const consumerRoot = join(fixtureRoot, 'consumer');
  const testRoot = join(fixtureRoot, 'consumer-tests');
  const fixtureHashBefore = await hashDirectory(fixtureRoot);
  const sourceConsumer = await snapshotDirectory(testRoot);
  const commandRuns = { hostPreparer: 0, canonicalRunner: 0 };
  let negativeProbeRuns = 0;
  let runtimeResources;
  let probeEvidence;
  let copiedExternalControlFiles;

  await runtimeOwner({
    skiffRoot: absoluteSkiffRoot,
    runTest: async (isolatedEnv, signal, stack) => {
      negativeProbeRuns += 1;
      if (negativeProbeRuns !== 1) {
        throw new Error('negative Host probe must own exactly one isolated runtime run');
      }
      validateIsolatedStack(stack);
      const host = packageServiceHostFixturePaths({
        skiffRoot: absoluteSkiffRoot,
        tempRoot: stack.tempRoot,
      });
      if (
        host.fixtureRoot !== fixtureRoot
        || host.consumerRoot !== consumerRoot
        || host.testRoot !== testRoot
      ) {
        throw new Error('canonical Host fixture paths did not resolve inside the selected checkout');
      }

      const negativeConsumerRoot = join(
        stack.tempRoot,
        'package-service-host-negative-consumer',
      );
      await cp(testRoot, negativeConsumerRoot, {
        recursive: true,
        errorOnExist: true,
        force: false,
      });
      await writeFile(
        join(negativeConsumerRoot, NEGATIVE_TEST_RELATIVE_PATH),
        NEGATIVE_TEST_SOURCE,
        'utf8',
      );
      const copiedConsumer = await snapshotDirectory(negativeConsumerRoot);
      assertOnlyNegativeTestChanged(sourceConsumer, copiedConsumer);
      copiedExternalControlFiles = externalControlFileReceipt(
        sourceConsumer,
        copiedConsumer,
      );

      commandRuns.hostPreparer += 1;
      const prepareOutcome = await runCommand(
        'cargo',
        packageServiceHostFixturePrepareCargoArgs({
          skiffRoot: absoluteSkiffRoot,
          fixtureRoot: host.fixtureRoot,
          artifactRoot: stack.sourceArtifactRoot,
          workRoot: host.workRoot,
          receipt: host.receipt,
          profile: isolatedEnv.SKIFF_TEST_ENVIRONMENT,
        }),
        { cwd: absoluteSkiffRoot, env: isolatedEnv },
      );
      requireSuccessfulCommand('canonical Host preparer', prepareOutcome);
      const receipt = await readHostReceipt(
        host.receipt,
        isolatedEnv.SKIFF_TEST_ENVIRONMENT,
      );

      const proxy = await startProxy({ targetUrl: stack.routerHttpUrl });
      runtimeResources = {
        tempRoot: stack.tempRoot,
        supervisorPid: stack.supervisor.pid,
        isolatedPorts: [...stack.ports],
        proxyPort: proxy.port,
      };
      try {
        requireRequestCount(proxy.snapshot(), 0, 'before the runner');
        commandRuns.canonicalRunner += 1;
        const runnerOutcome = await runCommand(
          'cargo',
          skiffSourceTestRunnerCargoArgs({
            skiffRoot: absoluteSkiffRoot,
            root: negativeConsumerRoot,
            artifactRoot: stack.sourceArtifactRoot,
            baseAssembly: receipt.baseAssembly.assemblyIdentity,
            baseConfigSnapshot: receipt.baseConfigSnapshot.snapshotId,
          }),
          {
            cwd: absoluteSkiffRoot,
            env: {
              ...isolatedEnv,
              SKIFF_TEST_INGRESS_URL: proxy.url,
            },
          },
        );
        await proxy.waitForIdle();
        const beforeGrace = proxy.snapshot();
        requireRequestCount(beforeGrace, 1, 'when the runner exited');
        await wait(graceWindowMs, signal);
        await proxy.waitForIdle();
        const afterGrace = proxy.snapshot();
        requireRequestCount(afterGrace, 1, 'after the retry grace window');
        probeEvidence = validateNegativeResult({
          runnerOutcome,
          beforeGrace,
          afterGrace,
          graceWindowMs,
        });
      } finally {
        await proxy.close();
      }
    },
  });

  if (negativeProbeRuns !== 1 || probeEvidence === undefined || runtimeResources === undefined) {
    throw new Error('isolated runtime owner omitted the focused negative Host execution');
  }
  if (commandRuns.hostPreparer !== 1 || commandRuns.canonicalRunner !== 1) {
    throw new Error('focused negative Host probe must run its preparer and runner exactly once');
  }
  const fixtureHashAfter = await hashDirectory(fixtureRoot);
  if (fixtureHashAfter !== fixtureHashBefore) {
    throw new Error('checked-in package-service Host fixture changed during the negative probe');
  }
  const cleanup = await inspectCleanup(runtimeResources);
  requireCompleteCleanup(cleanup);

  return {
    schemaVersion: PROBE_SCHEMA_VERSION,
    verdict: 'PASS',
    fullProbeRuns: 0,
    negativeProbeRuns,
    commands: {
      hostPreparerRuns: commandRuns.hostPreparer,
      canonicalRunnerRuns: commandRuns.canonicalRunner,
      sourceSuiteRuns: 0,
    },
    fixture: {
      checkedInSha256Before: fixtureHashBefore,
      checkedInSha256After: fixtureHashAfter,
      checkedInUnchanged: true,
      copiedConsumerModifiedFiles: [NEGATIVE_TEST_RELATIVE_PATH],
      copiedExternalControlFiles,
      falseAssertion: 'assert false',
    },
    ...probeEvidence,
    cleanup,
  };
}

export async function startTransparentCountingProxy({ targetUrl }) {
  const target = new URL(targetUrl);
  if (target.protocol !== 'http:' || target.pathname !== '/' || target.search !== '') {
    throw new Error('negative Host proxy target must be an HTTP origin URL');
  }
  const requests = [];
  const inFlight = new Set();
  const server = createServer((incoming, outgoing) => {
    const operation = forwardRequest({ incoming, outgoing, target, requests });
    inFlight.add(operation);
    operation.finally(() => inFlight.delete(operation));
  });
  await listenOnLoopback(server);
  const address = server.address();
  if (address === null || typeof address === 'string') {
    await closeServer(server);
    throw new Error('negative Host proxy did not acquire a TCP port');
  }
  let closed = false;
  return {
    url: `http://127.0.0.1:${address.port}`,
    port: address.port,
    snapshot: () => ({
      requestCount: requests.length,
      syntheticResponses: 0,
      requests: requests.map((entry) => ({ ...entry })),
    }),
    async waitForIdle() {
      await waitForOperations(inFlight);
    },
    async close() {
      if (closed) return;
      closed = true;
      server.closeIdleConnections?.();
      await closeServer(server);
      await assertPortsClosed([address.port]);
    },
  };
}

async function forwardRequest({ incoming, outgoing, target, requests }) {
  let entry;
  try {
    const body = await readRequestBody(incoming);
    entry = {
      host: incoming.headers.host ?? null,
      method: incoming.method ?? null,
      path: incoming.url ?? null,
      bodyBytes: body.length,
      bodyBase64: body.toString('base64'),
      bodySha256: createHash('sha256').update(body).digest('hex'),
      responseStatus: null,
      forwarded: false,
    };
    requests.push(entry);
    await sendToRouter({ incoming, outgoing, target, body, entry });
  } catch (error) {
    if (entry !== undefined) entry.forwardError = errorMessage(error);
    outgoing.destroy(error instanceof Error ? error : undefined);
  }
}

function sendToRouter({ incoming, outgoing, target, body, entry }) {
  return new Promise((resolvePromise, reject) => {
    const upstream = requestHttp({
      protocol: target.protocol,
      hostname: target.hostname,
      port: target.port,
      method: incoming.method,
      path: incoming.url,
      headers: incoming.headers,
    });
    upstream.once('error', reject);
    upstream.once('response', (response) => {
      if (!Number.isInteger(response.statusCode)) {
        response.destroy();
        reject(new Error('Router response omitted its HTTP status'));
        return;
      }
      entry.forwarded = true;
      entry.responseStatus = response.statusCode;
      if (typeof response.statusMessage === 'string') {
        outgoing.writeHead(response.statusCode, response.statusMessage, response.headers);
      } else {
        outgoing.writeHead(response.statusCode, response.headers);
      }
      response.once('error', reject);
      response.once('end', resolvePromise);
      response.pipe(outgoing);
    });
    upstream.end(body);
  });
}

async function readRequestBody(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  return Buffer.concat(chunks);
}

function validateNegativeResult({ runnerOutcome, beforeGrace, afterGrace, graceWindowMs }) {
  if (runnerOutcome.error !== null || runnerOutcome.signal !== null || runnerOutcome.code !== 1) {
    throw new Error(
      `canonical negative runner must exit 1, got ${runnerOutcome.signal ?? runnerOutcome.code ?? errorMessage(runnerOutcome.error)}`,
    );
  }
  const stdoutLines = lines(runnerOutcome.stdout);
  const combinedLines = lines(`${runnerOutcome.stdout}\n${runnerOutcome.stderr}`);
  const failLines = stdoutLines.filter((line) => line.startsWith('FAIL '));
  if (failLines.length !== 1) {
    throw new Error(`canonical negative runner must print exactly one FAIL, got ${failLines.length}`);
  }
  const summaryLines = combinedLines.filter((line) =>
    /^error: 1 test(?:\(s\))? failed$/.test(line));
  if (summaryLines.length !== 1) {
    throw new Error('canonical negative runner must print exactly one one-test failure summary');
  }
  const httpDiagnostics = combinedLines
    .map((line) => line.match(/^\s*HTTP ([45][0-9]{2}): (.+)$/))
    .filter((match) => match !== null);
  if (httpDiagnostics.length !== 1) {
    throw new Error('canonical negative runner must report exactly one non-2xx HTTP diagnostic');
  }
  const [, rawStatus, runtimeDiagnostic] = httpDiagnostics[0];
  if (!runtimeDiagnostic.includes('assertion failed')) {
    throw new Error('canonical negative runner omitted the Runtime assertion diagnostic');
  }
  const request = afterGrace.requests[0];
  if (
    request.forwarded !== true
    || request.responseStatus !== Number(rawStatus)
    || request.host === null
    || request.method === null
    || request.path === null
  ) {
    throw new Error('counting proxy did not transparently observe the failed business ingress');
  }
  if (beforeGrace.syntheticResponses !== 0 || afterGrace.syntheticResponses !== 0) {
    throw new Error('counting proxy must never synthesize a response');
  }
  return {
    runner: {
      runs: 1,
      exitCode: 1,
      signal: null,
      processExited: true,
      failLines,
      failedTests: 1,
      failureSummary: '1 test failed',
      rawFailureSummary: summaryLines[0],
      httpStatus: Number(rawStatus),
      runtimeDiagnostic,
    },
    ingress: {
      requestCount: afterGrace.requestCount,
      retryCount: afterGrace.requestCount - beforeGrace.requestCount,
      graceWindowMs,
      syntheticResponses: afterGrace.syntheticResponses,
      requests: afterGrace.requests,
    },
  };
}

async function inspectNegativeProbeCleanup({
  tempRoot,
  supervisorPid,
  isolatedPorts,
  proxyPort,
}) {
  const tempRootRemoved = !await pathExists(tempRoot);
  const supervisorPidStopped = !processAlive(supervisorPid);
  let isolatedPortsClosed = true;
  let proxyPortClosed = true;
  try {
    await assertPortsClosed(isolatedPorts);
  } catch {
    isolatedPortsClosed = false;
  }
  try {
    await assertPortsClosed([proxyPort]);
  } catch {
    proxyPortClosed = false;
  }
  return {
    supervisorPid,
    supervisorPidStopped,
    isolatedPorts,
    isolatedPortsClosed,
    proxyPort,
    proxyPortClosed,
    tempRootRemoved,
  };
}

function validateIsolatedStack(stack) {
  if (
    typeof stack?.tempRoot !== 'string'
    || typeof stack.sourceArtifactRoot !== 'string'
    || typeof stack.routerHttpUrl !== 'string'
    || !Array.isArray(stack.ports)
    || stack.ports.length === 0
    || !Number.isInteger(stack.supervisor?.pid)
  ) {
    throw new Error('isolated runtime owner omitted negative probe resource evidence');
  }
}

function requireCompleteCleanup(cleanup) {
  for (const field of [
    'supervisorPidStopped',
    'isolatedPortsClosed',
    'proxyPortClosed',
    'tempRootRemoved',
  ]) {
    if (cleanup?.[field] !== true) {
      throw new Error(`negative Host probe cleanup did not establish ${field}`);
    }
  }
}

function requireSuccessfulCommand(label, outcome) {
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    throw new Error(
      `${label} failed (${outcome.signal ?? outcome.code ?? errorMessage(outcome.error)}): ${commandText(outcome)}`,
    );
  }
}

function requireRequestCount(snapshot, expected, when) {
  if (snapshot.requestCount !== expected) {
    throw new Error(
      `counting proxy expected ${expected} business ingress request(s) ${when}, got ${snapshot.requestCount}`,
    );
  }
}

async function snapshotDirectory(root) {
  const paths = await regularFiles(root);
  const snapshot = new Map();
  for (const path of paths) {
    const contents = await readFile(path);
    const name = relative(root, path).split(sep).join('/');
    snapshot.set(name, {
      sha256: createHash('sha256').update(contents).digest('hex'),
      size: contents.length,
    });
  }
  return snapshot;
}

async function hashDirectory(root) {
  const snapshot = await snapshotDirectory(root);
  const hash = createHash('sha256');
  for (const [name, metadata] of [...snapshot.entries()].sort(([left], [right]) =>
    left.localeCompare(right))) {
    hash.update(name);
    hash.update('\0');
    hash.update(metadata.sha256);
    hash.update('\0');
    hash.update(String(metadata.size));
    hash.update('\n');
  }
  return hash.digest('hex');
}

async function regularFiles(root) {
  const files = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) files.push(...await regularFiles(path));
    else if (entry.isFile()) files.push(path);
    else throw new Error(`negative Host fixture contains unsupported entry ${path}`);
  }
  return files.sort();
}

function assertOnlyNegativeTestChanged(source, copied) {
  const sourceNames = [...source.keys()].sort();
  const copiedNames = [...copied.keys()].sort();
  if (JSON.stringify(copiedNames) !== JSON.stringify(sourceNames)) {
    throw new Error('negative Host consumer copy changed the fixture file set');
  }
  for (const name of sourceNames) {
    if (name !== NEGATIVE_TEST_RELATIVE_PATH
      && copied.get(name).sha256 !== source.get(name).sha256) {
      throw new Error(`negative Host consumer copy unexpectedly changed ${name}`);
    }
  }
  const expectedHash = createHash('sha256').update(NEGATIVE_TEST_SOURCE).digest('hex');
  if (copied.get(NEGATIVE_TEST_RELATIVE_PATH)?.sha256 !== expectedHash) {
    throw new Error('negative Host consumer copy omitted the deterministic false assertion');
  }
}

function externalControlFileReceipt(source, copied) {
  return [...source.entries()]
    .filter(([name]) =>
      name === 'http.yml'
      || name.endsWith('/http.yml')
      || name === 'websocket.yml'
      || name.endsWith('/websocket.yml'))
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([path, metadata]) => {
      const copiedMetadata = copied.get(path);
      if (
        copiedMetadata?.sha256 !== metadata.sha256
        || copiedMetadata?.size !== metadata.size
      ) {
        throw new Error(`negative Host consumer copy lost external control file ${path}`);
      }
      return { path, ...metadata };
    });
}

function listenOnLoopback(server) {
  return new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject);
      resolvePromise();
    });
  });
}

function closeServer(server) {
  return new Promise((resolvePromise, reject) => {
    server.close((error) => error ? reject(error) : resolvePromise());
  });
}

async function waitForOperations(inFlight) {
  while (inFlight.size > 0) {
    await Promise.allSettled([...inFlight]);
  }
}

function waitForGraceWindow(milliseconds, signal) {
  return delay(milliseconds, undefined, { signal });
}

function pathExists(path) {
  return access(path).then(() => true, () => false);
}

function processAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === 'EPERM';
  }
}

function lines(value) {
  return String(value ?? '').split(/\r?\n/).filter((line) => line.length > 0);
}

function commandText(outcome) {
  return `${outcome.stdout ?? ''}\n${outcome.stderr ?? ''}`.trim();
}

function errorMessage(error) {
  return error?.message || String(error);
}

export const packageServiceHostNegativeProbeConstants = Object.freeze({
  defaultGraceWindowMs: DEFAULT_GRACE_WINDOW_MS,
  negativeTestRelativePath: NEGATIVE_TEST_RELATIVE_PATH,
  negativeTestSource: NEGATIVE_TEST_SOURCE,
  schemaVersion: PROBE_SCHEMA_VERSION,
});
