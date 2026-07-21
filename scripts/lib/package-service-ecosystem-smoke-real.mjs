import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import { devRuntimePaths } from './dev-runtime-paths.mjs';
import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import {
  activate,
  assertHostMarker,
  capabilityConnectionIds,
  committedReplicaIds,
  fixtureEntrypoint,
  inFlightRequestCount,
  openEntrypointStream,
  requestEntrypoint,
  routerHealth,
  waitForCanonicalRouter,
  waitForInFlightRequest,
  waitForRuntimeEvidence,
} from './package-service-ecosystem-smoke-probes.mjs';

export async function runPackageServiceEcosystemSmoke({
  checkout,
  replicaCount,
  environment,
}) {
  let fixtures;
  const result = await runInIsolatedTestRuntime({
    skiffRoot: checkout,
    environment,
    dependencies: {
      seedBootstrap: async ({ artifactRoot, env, signal }) => {
        fixtures = await publishFixtures({
          checkout,
          artifactRoot,
          environment,
          env,
          signal,
        });
      },
      waitReady: waitForCanonicalRouter,
    },
    runTest: async (isolatedEnv, signal, stack) => {
      assert.ok(fixtures, 'canonical fixtures must be published before startup');
      assert.equal(isolatedEnv.SKIFF_DEV_RELOAD_URL, undefined);
      assert.equal(isolatedEnv.SKIFF_TEST_ARTIFACT_ROOT, stack.artifactRoot);
      assert.ok(!stack.tempRoot.includes('.skiff-instance'));
      const extraReplicas = await startExtraReplicas({
        checkout,
        count: replicaCount - 1,
        environment,
        isolatedEnv,
        signal,
        stack,
      });
      try {
        const bootstrap = fixtures.old.bootstrap;
        assert.ok(bootstrap, 'old fixture must initialize canonical generation zero');
        const coldHealth = await waitForRuntimeEvidence(stack.controlUrl, {
          capabilityCount: replicaCount,
          registrationCount: replicaCount,
          generation: bootstrap.generation,
          assemblyIdentity: bootstrap.assembly.assemblyIdentity,
        }, signal);
        const coldCapabilities = capabilityConnectionIds(coldHealth);
        const coldRegistrations = committedReplicaIds(coldHealth, {
          generation: bootstrap.generation,
          assemblyIdentity: bootstrap.assembly.assemblyIdentity,
        });

        const oldActivation = await activate(
          stack.controlUrl,
          fixtures.old.candidate.assembly,
          0,
          'smoke-old',
          environment,
          signal,
        );
        const oldHealth = await waitForRuntimeEvidence(stack.controlUrl, {
          capabilityCount: replicaCount,
          registrationCount: replicaCount,
          generation: 1,
          assemblyIdentity: fixtures.old.candidate.assembly.assemblyIdentity,
        }, signal);
        assert.deepEqual(capabilityConnectionIds(oldHealth), coldCapabilities);
        const oldUnary = fixtureEntrypoint(fixtures.old, 'unary');
        const oldTest = fixtureEntrypoint(fixtures.old, 'packageTest');
        const oldStream = fixtureEntrypoint(fixtures.old, 'serverStream');
        const oldResult = await requestEntrypoint(stack.routerHttpUrl, oldUnary, signal);
        assert.ok(oldResult.ok, `old Host result failed: ${oldResult.status} ${oldResult.body}`);
        assertHostMarker(oldResult, 'old-0');
        const spawnResult = await requestEntrypoint(stack.routerHttpUrl, oldTest, signal);
        assert.ok(
          spawnResult.ok,
          `spawn package-test Host result failed: ${spawnResult.status} ${spawnResult.body}`,
        );
        const pinnedStream = await openEntrypointStream(
          stack.routerHttpUrl,
          oldStream,
          signal,
          {
            startMarker: 'old-0-stream-start',
            endMarker: 'old-0-stream-end',
            forbiddenMarker: 'new-2-stream',
          },
        );
        assert.match(pinnedStream.firstChunk, /old-0-stream-start/);
        assert.equal(pinnedStream.isOpen(), true, 'A stream ended before cutover began');
        await waitForInFlightRequest(stack.controlUrl, signal);

        const rejected = await activate(
          stack.controlUrl,
          fixtures.tampered.candidate.assembly,
          1,
          'smoke-tampered',
          environment,
          signal,
          { expectFailure: true },
        );
        assert.ok(!rejected.ok, 'tampered candidate must be rejected');
        const stateAfterReject = JSON.parse(await readFile(
          path.join(stack.artifactRoot, 'environments', environment, 'activation.json'),
          'utf8',
        ));
        assert.equal(stateAfterReject.committed.generation, 1);
        assert.deepEqual(stateAfterReject.committed.assembly, fixtures.old.candidate.assembly);
        assert.equal(stateAfterReject.pending, null);
        const oldAfterReject = await requestEntrypoint(stack.routerHttpUrl, oldUnary, signal);
        assert.ok(oldAfterReject.ok, 'old Host result must survive activation abort');
        assertHostMarker(oldAfterReject, 'old-0');
        assert.equal(
          pinnedStream.isOpen(),
          true,
          'candidate abort must not terminate the A stream',
        );

        const newActivation = await activate(
          stack.controlUrl,
          fixtures.new.candidate.assembly,
          1,
          'smoke-new',
          environment,
          signal,
        );
        const newHealth = await waitForRuntimeEvidence(stack.controlUrl, {
          capabilityCount: replicaCount,
          registrationCount: replicaCount,
          generation: 2,
          assemblyIdentity: fixtures.new.candidate.assembly.assemblyIdentity,
        }, signal);
        assert.deepEqual(capabilityConnectionIds(newHealth), coldCapabilities);
        assert.equal(pinnedStream.isOpen(), true, 'A stream must remain open after B commits');
        assert.ok(
          inFlightRequestCount(newHealth) > 0,
          'router must still observe the generation-pinned A stream in flight after B commits',
        );
        const newUnary = fixtureEntrypoint(fixtures.new, 'unary');
        const finalResult = await requestEntrypoint(stack.routerHttpUrl, newUnary, signal);
        assert.ok(finalResult.ok, `new Host result failed: ${finalResult.status} ${finalResult.body}`);
        assertHostMarker(finalResult, 'new-2');
        pinnedStream.resume();
        const oldStreamResult = await pinnedStream.completion;
        assert.equal(oldStreamResult.containsStart, true);
        assert.equal(oldStreamResult.containsEnd, true);
        assert.equal(oldStreamResult.containsForbidden, false);

        let replicaFailover = false;
        if (extraReplicas.length > 0) {
          const before = await routerHealth(stack.controlUrl, signal);
          assert.equal(committedReplicaIds(before, {
            generation: 2,
            assemblyIdentity: fixtures.new.candidate.assembly.assemblyIdentity,
          }).length, replicaCount);
          await stopChild(extraReplicas[0]);
          await waitForRuntimeEvidence(stack.controlUrl, {
            capabilityCount: 1,
            registrationCount: 1,
            generation: 2,
            assemblyIdentity: fixtures.new.candidate.assembly.assemblyIdentity,
          }, signal);
          const failover = await requestEntrypoint(stack.routerHttpUrl, newUnary, signal);
          assert.ok(failover.ok, 'Host request must fail over to the surviving replica');
          assertHostMarker(failover, 'new-2');
          replicaFailover = true;
        }
        const health = await routerHealth(stack.controlUrl, signal);
        assert.equal(health.pendingActivation ?? null, null);
        return {
          status: 'PASS',
          probe: 'skiff-cutover',
          replicas: replicaCount,
          activationId: 'smoke-new',
          generation: newActivation.body.committed?.generation ?? 2,
          assembly: fixtures.new.candidate.assembly.assemblyIdentity,
          replicaIds: committedReplicaIds(health, {
            generation: 2,
            assemblyIdentity: fixtures.new.candidate.assembly.assemblyIdentity,
          }),
          hostResult: {
            status: finalResult.status,
            sha256: createHash('sha256').update(finalResult.body).digest('hex'),
          },
          capabilitiesHandshake: coldCapabilities.length === replicaCount,
          coldStartupRegistration: coldRegistrations.length === replicaCount,
          binaryActivationRoundTrip: {
            transportCodec: 'production-owned',
            prepareCommitRegisterObserved: true,
            rejectAbortRollbackObserved: true,
          },
          spawnTypedResponse: true,
          oldGenerationStreamPin: {
            firstChunkBeforeActivation: true,
            remainedOpenAfterCommit: true,
            completedOnAssembly: fixtures.old.candidate.assembly.assemblyIdentity,
            sha256: oldStreamResult.sha256,
          },
          activationAbortRollback: true,
          pendingCleared: true,
          replicaFailover,
          temporaryRuntimeHomes: true,
          oldActivation: oldActivation.body.committed ?? null,
        };
      } finally {
        await Promise.all(extraReplicas.map(stopChild));
      }
    },
  });
  const [commit, tree] = await Promise.all([
    commandText('git', ['rev-parse', 'HEAD'], { cwd: checkout }),
    commandText('git', ['rev-parse', 'HEAD^{tree}'], { cwd: checkout }),
  ]);
  return { ...result, checkout, commit: commit.trim(), tree: tree.trim() };
}

async function publishFixtures({ checkout, artifactRoot, environment, env, signal }) {
  const fixtureRoot = path.join(path.dirname(artifactRoot), 'ecosystem-smoke-fixtures');
  const results = {};
  for (const [index, label] of ['old', 'tampered', 'new'].entries()) {
    const packageRoot = path.join(fixtureRoot, label);
    await writePackageFixture(packageRoot, label, index);
    results[label] = await commandJson('cargo', [
      'run',
      '--quiet',
      '--manifest-path',
      path.join(checkout, 'test-runner', 'Cargo.toml'),
      '--bin',
      'skiff-package-service-smoke-fixture',
      '--',
      packageRoot,
      '--artifact-root',
      artifactRoot,
      '--environment',
      environment,
      ...(label === 'old' ? ['--initialize-environment'] : []),
    ], { cwd: checkout, env, signal });
  }
  await writeFile(
    path.join(artifactRoot, results.tampered.candidate.overlayRecordPath),
    '{}',
  );
  return results;
}

async function writePackageFixture(root, label, index) {
  const pressureChunk = `${label}-${index}-`.padEnd(64 * 1024, 'x');
  const pressureEmits = Array.from({ length: 256 }, () => '  emit(chunk)');
  await mkdir(root, { recursive: true });
  await writeFile(path.join(root, 'package.yml'), `${JSON.stringify({
    id: `test.skiff/ecosystem-smoke-${label}`,
    version: '1.0.0',
  }, null, 2)}\n`);
  await writeFile(
    path.join(root, 'api.yml'),
    'marker: main.marker\nevents: main.events\n',
  );
  await writeFile(
    path.join(root, 'main.skiff'),
    [
      `function marker() -> string { return "${label}-${index}" }`,
      '',
      'function events() -> Stream<string> {',
      `  emit("${label}-${index}-stream-start")`,
      `  const chunk = ${JSON.stringify(pressureChunk)}`,
      ...pressureEmits,
      `  emit("${label}-${index}-stream-end")`,
      '  return',
      '}',
      '',
    ].join('\n'),
  );
  await writeFile(
    path.join(root, 'main.test.skiff'),
    [
      `function typedSpawn${index}(value: string) -> void {`,
      '  return',
      '}',
      '',
      `test "${label}" {`,
      `  spawn typedSpawn${index}("${label}")`,
      '  assert true',
      '}',
      '',
    ].join('\n'),
  );
}

async function startExtraReplicas({
  checkout,
  count,
  environment,
  isolatedEnv,
  signal,
  stack,
}) {
  const children = [];
  const paths = devRuntimePaths({ devHome: stack.devHome, env: isolatedEnv });
  for (let index = 0; index < count; index += 1) {
    signal.throwIfAborted();
    const replicaRoot = path.join(stack.tempRoot, `runtime-replica-${index + 2}`);
    const runtimeHome = path.join(replicaRoot, 'home');
    const configPath = path.join(replicaRoot, 'runtime.yml');
    await mkdir(runtimeHome, { recursive: true });
    await writeFile(configPath, [
      `router: ${JSON.stringify(`ws://127.0.0.1:${stack.ports[1]}/runtime`)}`,
      `environment: ${JSON.stringify(environment)}`,
      `runtime-home: ${JSON.stringify(runtimeHome)}`,
      'artifactRoots:',
      `  - ${JSON.stringify(stack.artifactRoot)}`,
      '',
    ].join('\n'));
    const child = spawn(paths.runtimeBinary, [configPath], {
      cwd: checkout,
      env: isolatedEnv,
      stdio: 'inherit',
    });
    child.once('error', () => undefined);
    children.push(child);
  }
  return children;
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exit = new Promise((resolve) => child.once('exit', resolve));
  child.kill('SIGTERM');
  if (await Promise.race([exit.then(() => true), delay(5_000).then(() => false)])) return;
  child.kill('SIGKILL');
  await exit;
}

function commandJson(command, args, options) {
  return commandText(command, args, options).then((stdout) => JSON.parse(stdout));
}

function commandText(command, args, { cwd, env = process.env, signal } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      env,
      signal,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on('data', (chunk) => stdout.push(chunk));
    child.stderr.on('data', (chunk) => stderr.push(chunk));
    child.once('error', reject);
    child.once('exit', (code, childSignal) => {
      if (code === 0) {
        resolve(Buffer.concat(stdout).toString('utf8'));
      } else {
        reject(new Error(
          `${command} exited code=${code} signal=${childSignal ?? 'none'}: ${Buffer.concat(stderr).toString('utf8')}`,
        ));
      }
    });
  });
}
