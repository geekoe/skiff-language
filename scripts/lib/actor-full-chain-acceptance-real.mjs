import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { request as requestHttp } from 'node:http';
import { join } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { packageServiceEcosystemSmokeFixtureCargoArgs } from './package-service-ecosystem-smoke-real.mjs';
import { writeReleasePointerSeed } from './release-pointer-seed.mjs';

const FIXTURE_ROOT = join('test-runner', 'fixtures', 'actor-full-chain-acceptance');

export async function runActorFullChainAcceptance({
  checkout,
  profile = 'actor-full-chain',
}) {
  return runInIsolatedTestRuntime({
    skiffRoot: checkout,
    profile,
    runtimeReplicas: 2,
    runTest: async (isolatedEnv, signal, stack) => {
      const outcome = await captureCheckedCommand(
        'cargo',
        packageServiceEcosystemSmokeFixtureCargoArgs({
          checkout,
          fixtureRoot: join(checkout, FIXTURE_ROOT),
          artifactRoot: stack.artifactRoot,
          profile,
        }),
        { cwd: checkout, env: isolatedEnv, signal },
      );
      const receipt = JSON.parse(outcome.stdout);
      assert.equal(receipt.schemaVersion, 'skiff-package-service-smoke-fixture-v4');
      assert.equal(receipt.candidate.testService.packageId, 'test.skiff/actor-full-chain-acceptance');
      const unary = receipt.candidate.entrypoints.find(
        (entrypoint) => entrypoint.gatewayEntryKey === 'probe',
      );
      assert.ok(unary, 'Actor fixture must publish its real unary marker');
      const deployment = receipt.candidate.deployments[0];
      await writeReleasePointerSeed({
        artifactRoot: stack.artifactRoot,
        profile,
        deployment,
        recordPath: actorDeploymentRecordPath(deployment),
      });
      await waitForTwoActiveReplicas({
        controlUrl: stack.controlUrl,
        profile,
        signal,
      });

      const first = await invokeUnary(stack.routerHttpUrl, unary, signal);
      const second = await invokeUnary(stack.routerHttpUrl, unary, signal);
      assert.equal(first, 'actor-count-1');
      assert.equal(second, 'actor-count-next');

      const entrypoints = new Map(
        receipt.candidate.entrypoints.map((entrypoint) => [
          entrypoint.gatewayEntryKey,
          entrypoint,
        ]),
      );
      const slowGet = entrypoints.get('slowGet');
      const slowDedup = entrypoints.get('slowDedup');
      const slowIncrement = entrypoints.get('slowIncrement');
      const flakyGet = entrypoints.get('flakyGet');
      assert.ok(slowGet, 'Actor fixture must publish slowGet');
      assert.ok(slowDedup, 'Actor fixture must publish slowDedup');
      assert.ok(slowIncrement, 'Actor fixture must publish slowIncrement');
      assert.ok(flakyGet, 'Actor fixture must publish flakyGet');

      // get waits for create: a get-only probe with a 300ms create sleep must
      // not return before create completes.
      const slowStarted = Date.now();
      const slowGetBody = await invokeUnary(stack.routerHttpUrl, slowGet, signal);
      const slowElapsedMs = Date.now() - slowStarted;
      assert.equal(slowGetBody, 'slow-get-ok');
      assert.ok(
        slowElapsedMs >= 200,
        `get returned before create completed: ${slowElapsedMs}ms`,
      );
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, slowIncrement, signal),
        'slow-ok',
      );
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, slowIncrement, signal),
        'slow-ok',
      );

      // A regular actor method may synchronously call another method through
      // `self`. The nested invocation forces the outer continuation to release
      // the instance, then the outer method must resume on the nested method's
      // committed field value: 0 + 1 + 4 + 100 = 105.
      const synchronousSelfCall = entrypoints.get('synchronousSelfCall');
      const synchronousSelfCount = entrypoints.get('synchronousSelfCount');
      assert.ok(
        synchronousSelfCall,
        'Actor fixture must publish synchronousSelfCall',
      );
      assert.ok(
        synchronousSelfCount,
        'Actor fixture must publish synchronousSelfCount',
      );
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, synchronousSelfCall, signal),
        105,
      );
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, synchronousSelfCount, signal),
        105,
      );

      // Concurrent gets for one fresh id dedup onto a single activation and
      // both wait for the same create. The isolated acceptance profile
      // starts with an empty router registry, so the fixed id is a new entry.
      const dedupStarted = Date.now();
      const [dedupLeft, dedupRight] = await Promise.all([
        invokeUnary(stack.routerHttpUrl, slowDedup, signal),
        invokeUnary(stack.routerHttpUrl, slowDedup, signal),
      ]);
      const dedupElapsedMs = Date.now() - dedupStarted;
      assert.equal(dedupLeft, 'slow-get-ok');
      assert.equal(dedupRight, 'slow-get-ok');
      assert.ok(
        dedupElapsedMs >= 200,
        `concurrent gets did not wait for one create: ${dedupElapsedMs}ms`,
      );

      // Create failure surfaces on get; the retained entry keeps failing on
      // retry so the failure is observable again through the method path.
      const flakyFirst = await invokeUnaryRaw(stack.routerHttpUrl, flakyGet, signal);
      assert.notEqual(flakyFirst.status, 200, 'flaky get must fail');
      assert.match(
        flakyFirst.body,
        /UnhandledServiceError|InternalError|ProviderUnavailable/,
        `flaky get failure must surface as a platform error, got ${flakyFirst.body}`,
      );
      const flakyRetry = await invokeUnaryRaw(stack.routerHttpUrl, flakyGet, signal);
      assert.notEqual(flakyRetry.status, 200, 'retained flaky entry must keep failing');

      // spawn to an actor method from a plain function: the submit returns
      // before the target method (500ms record sleep) has run, and the call is
      // queued on the actor instance.
      const spawnExternal = entrypoints.get('spawnExternal');
      const externalCount = entrypoints.get('externalCount');
      const externalHistory = entrypoints.get('externalHistory');
      assert.ok(spawnExternal, 'Actor fixture must publish spawnExternal');
      assert.ok(externalCount, 'Actor fixture must publish externalCount');
      assert.ok(externalHistory, 'Actor fixture must publish externalHistory');
      const externalStarted = Date.now();
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, spawnExternal, signal),
        'external-submitted',
      );
      const externalSubmitElapsedMs = Date.now() - externalStarted;
      assert.ok(
        externalSubmitElapsedMs < 250,
        `spawn submit waited for the target method: ${externalSubmitElapsedMs}ms`,
      );
      await waitForActorValue({
        routerHttpUrl: stack.routerHttpUrl,
        entrypoint: externalCount,
        expected: 1,
        signal,
      });
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, externalHistory, signal),
        'x',
      );

      // spawn self.method inside an actor method: the self message advances the
      // same instance without nesting in the submitting method's call stack.
      const spawnSelfKick = entrypoints.get('spawnSelfKick');
      const selfKickCount = entrypoints.get('selfKickCount');
      const selfKickHistory = entrypoints.get('selfKickHistory');
      assert.ok(spawnSelfKick, 'Actor fixture must publish spawnSelfKick');
      assert.ok(selfKickCount, 'Actor fixture must publish selfKickCount');
      assert.ok(selfKickHistory, 'Actor fixture must publish selfKickHistory');
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, spawnSelfKick, signal),
        'kicked',
      );
      await waitForActorValue({
        routerHttpUrl: stack.routerHttpUrl,
        entrypoint: selfKickCount,
        expected: 1,
        signal,
      });
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, selfKickHistory, signal),
        's',
      );

      // Multiple spawned self messages queue serially on the same instance and
      // all run to completion.
      const spawnFanout = entrypoints.get('spawnFanout');
      const fanoutCount = entrypoints.get('fanoutCount');
      const fanoutHistory = entrypoints.get('fanoutHistory');
      assert.ok(spawnFanout, 'Actor fixture must publish spawnFanout');
      assert.ok(fanoutCount, 'Actor fixture must publish fanoutCount');
      assert.ok(fanoutHistory, 'Actor fixture must publish fanoutHistory');
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, spawnFanout, signal),
        'fanned',
      );
      await waitForActorValue({
        routerHttpUrl: stack.routerHttpUrl,
        entrypoint: fanoutCount,
        expected: 3,
        signal,
      });
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, fanoutHistory, signal),
        'abc',
      );

      // A chained spawn self.method: tick() spawns itself once per step for
      // 160 steps. Queued independent invocations complete all steps serially;
      // nesting the spawned call in the submitting method's stack would grow
      // programCallDepth past the runtime safety limit (128) and fail.
      const chainKick = entrypoints.get('chainKick');
      const chainSteps = entrypoints.get('chainSteps');
      const chainHistory = entrypoints.get('chainHistory');
      assert.ok(chainKick, 'Actor fixture must publish chainKick');
      assert.ok(chainSteps, 'Actor fixture must publish chainSteps');
      assert.ok(chainHistory, 'Actor fixture must publish chainHistory');
      const chainStarted = Date.now();
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, chainKick, signal),
        'chain-kicked',
      );
      const chainSubmitElapsedMs = Date.now() - chainStarted;
      assert.ok(
        chainSubmitElapsedMs < 250,
        `spawn self submit waited for the chained target: ${chainSubmitElapsedMs}ms`,
      );
      await waitForActorValue({
        routerHttpUrl: stack.routerHttpUrl,
        entrypoint: chainSteps,
        expected: 160,
        signal,
      });
      assert.equal(
        await invokeUnary(stack.routerHttpUrl, chainHistory, signal),
        'c'.repeat(160),
      );

      // A spawned actor method that throws a user exception must construct its
      // request-local exception from the caller's trace id. Without spawn
      // trace penetration the failure is reported as "request-local exception
      // requires a non-empty request trace id".
      const spawnThrow = entrypoints.get('spawnThrow');
      assert.ok(spawnThrow, 'Actor fixture must publish spawnThrow');
      const spawnThrowResponse = await invokeUnaryRaw(
        stack.routerHttpUrl,
        spawnThrow,
        signal,
        null
      );
      assert.equal(
        spawnThrowResponse.status,
        200,
        `spawn throw submit failed: ${spawnThrowResponse.body}`
      );
      assert.equal(JSON.parse(spawnThrowResponse.body), 'throw-spawned');
      await waitForSpawnThrowFailure({
        runtimeLogPaths: [
          join(stack.tempRoot, 'instance', 'logs', 'runtime.log'),
          join(stack.tempRoot, 'instance', 'logs', 'runtime.err.log'),
          join(stack.tempRoot, 'instance', 'logs', 'runtime-2.log'),
          join(stack.tempRoot, 'instance', 'logs', 'runtime-2.err.log'),
        ],
        signal,
      });

      const health = await readHealth(stack.controlUrl, signal);
      const replicas = health.replicas.filter(
        (replica) =>
          replica.connected === true
          && replica.state === 'healthy'
          && replica.profile === profile,
      );
      assert.equal(new Set(replicas.map((replica) => replica.replicaId)).size, 2);
      return {
        status: 'PASS',
        fixture: FIXTURE_ROOT,
        assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
        replicas: replicas.map((replica) => replica.replicaId).sort(),
        results: [first, second],
      };
    },
  });
}

async function invokeUnary(routerHttpUrl, entrypoint, signal) {
  const response = await invokeUnaryRaw(routerHttpUrl, entrypoint, signal);
  assert.equal(
    response.status,
    200,
    `Actor unary failed: ${response.body}`
  );
  return JSON.parse(response.body);
}

async function invokeUnaryRaw(routerHttpUrl, entrypoint, signal, bodyValue = null) {
  const { selector, deployment } = entrypoint;
  assert.equal(selector.protocol, 'http');
  const response = await new Promise((resolveResponse, rejectResponse) => {
    const request = requestHttp(`${routerHttpUrl}${selector.path}`, {
      method: selector.method,
      headers: {
        'x-skiff-service': deployment.serviceId,
        'x-skiff-version': deployment.contractVersion,
        'content-type': 'application/json',
      },
      signal,
    }, resolveResponse);
    request.once('error', rejectResponse);
    request.end(JSON.stringify(bodyValue));
  });
  const chunks = [];
  for await (const chunk of response) chunks.push(Buffer.from(chunk));
  const body = Buffer.concat(chunks);
  return { status: response.statusCode, body: body.toString('utf8') };
}

async function waitForTwoActiveReplicas({
  controlUrl,
  profile,
  signal,
}) {
  const started = Date.now();
  while (Date.now() - started < 120_000) {
    signal.throwIfAborted();
    const health = await readHealth(controlUrl, signal);
    const replicas = health.replicas.filter(
      (replica) =>
        replica.connected === true
        && replica.state === 'healthy'
        && replica.profile === profile,
    );
    if (new Set(replicas.map((replica) => replica.replicaId)).size === 2) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
  }
  throw new Error('Actor acceptance deployment did not reach two healthy Runtime replicas');
}

function actorDeploymentRecordPath(deployment) {
  const hex = deployment.deploymentArtifactIdentity.slice(
    deployment.deploymentArtifactIdentity.lastIndexOf(':') + 1,
  );
  return [
    'records',
    'service-deployments',
    deployment.serviceId.replaceAll('.', '~d').replaceAll('/', '~s'),
    deployment.contractVersion,
    deployment.deploymentRevision,
    `${hex}.json`,
  ].join('/');
}

async function waitForActorValue({
  routerHttpUrl,
  entrypoint,
  expected,
  signal,
}) {
  const started = Date.now();
  while (Date.now() - started < 15_000) {
    signal.throwIfAborted();
    const value = await invokeUnary(routerHttpUrl, entrypoint, signal);
    if (JSON.stringify(value) === JSON.stringify(expected)) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 50));
  }
  throw new Error(
    `actor value ${JSON.stringify(expected)} was not observed within 15s`,
  );
}

async function waitForSpawnThrowFailure({
  runtimeLogPaths,
  signal,
}) {
  const started = Date.now();
  while (Date.now() - started < 15_000) {
    signal.throwIfAborted();
    for (const runtimeLogPath of runtimeLogPaths) {
      let contents;
      try {
        contents = await readFile(runtimeLogPath, 'utf8');
      } catch (error) {
        if (error?.code === 'ENOENT') continue;
        throw error;
      }
      for (const failureLine of contents.split('\n')) {
        if (!failureLine.includes('runtime.actor_owner_invoke_failed')) continue;
        let parsed;
        try {
          parsed = JSON.parse(failureLine);
        } catch {
          continue;
        }
        const fields = parsed?.fields;
        if (fields?.event !== 'runtime.actor_owner_invoke_failed') continue;
        if (
          typeof fields.trace_id !== 'string'
          || fields.trace_id.trim().length === 0
        ) {
          continue;
        }
        assert.ok(
          !failureLine.includes('requires a non-empty request trace id'),
          `spawned actor method must not fail on a missing request trace id: ${failureLine}`
        );
        assert.ok(
          String(fields.error).includes('unhandled user exception'),
          `spawned actor method failure must surface the user exception: ${failureLine}`
        );
        return;
      }
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 50));
  }
  throw new Error(
    'spawned actor method failure with a non-empty trace id was not observed in the runtime logs within 15s'
  );
}

async function readHealth(controlUrl, signal) {
  const response = await fetch(`${controlUrl}/__router/health`, { signal });
  assert.equal(response.ok, true);
  return response.json();
}
