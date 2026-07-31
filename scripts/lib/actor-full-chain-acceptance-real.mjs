import assert from 'node:assert/strict';
import { request as requestHttp } from 'node:http';
import { join } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { requestAssemblyActivation } from './package-service-authoring.mjs';
import { packageServiceEcosystemSmokeFixtureCargoArgs } from './package-service-ecosystem-smoke-real.mjs';

const FIXTURE_ROOT = join('test-runner', 'fixtures', 'actor-full-chain-acceptance');

export async function runActorFullChainAcceptance({
  checkout,
  environment = 'actor-full-chain',
}) {
  return runInIsolatedTestRuntime({
    skiffRoot: checkout,
    environment,
    runtimeReplicas: 2,
    runTest: async (isolatedEnv, signal, stack) => {
      const outcome = await captureCheckedCommand(
        'cargo',
        packageServiceEcosystemSmokeFixtureCargoArgs({
          checkout,
          fixtureRoot: join(checkout, FIXTURE_ROOT),
          artifactRoot: stack.artifactRoot,
          environment,
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
      const activation = await requestAssemblyActivation({
        activationUrl: `${stack.controlUrl}/__skiff/activate-assembly`,
        expectedGeneration: 0,
        environment,
        assembly: receipt.candidate.assembly,
        configSnapshot: receipt.candidate.configSnapshot,
        signal,
      });
      assert.equal(activation.response.ok, true);
      const generation = activation.response.activeAssembly.generation;
      await waitForTwoActiveReplicas({
        controlUrl: stack.controlUrl,
        environment,
        generation,
        assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
        signal,
      });

      const first = await invokeUnary(stack.routerHttpUrl, unary, signal);
      const second = await invokeUnary(stack.routerHttpUrl, unary, signal);
      assert.equal(first, 'actor-count-1');
      assert.equal(second, 'actor-count-next');

      const health = await readHealth(stack.controlUrl, signal);
      const replicas = health.replicas.filter(
        (replica) =>
          replica.connected === true
          && replica.state === 'healthy'
          && replica.generation === generation,
      );
      assert.equal(new Set(replicas.map((replica) => replica.replicaId)).size, 2);
      return {
        status: 'PASS',
        fixture: FIXTURE_ROOT,
        assemblyIdentity: receipt.candidate.assembly.assemblyIdentity,
        generation,
        replicas: replicas.map((replica) => replica.replicaId).sort(),
        results: [first, second],
      };
    },
  });
}

async function invokeUnary(routerHttpUrl, entrypoint, signal) {
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
    request.end('null');
  });
  const chunks = [];
  for await (const chunk of response) chunks.push(Buffer.from(chunk));
  const body = Buffer.concat(chunks);
  assert.equal(
    response.statusCode,
    200,
    `Actor unary failed: ${body.toString('utf8')}`
  );
  return JSON.parse(body.toString('utf8'));
}

async function waitForTwoActiveReplicas({
  controlUrl,
  environment,
  generation,
  assemblyIdentity,
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
        && replica.environment === environment
        && replica.generation === generation
        && replica.assemblyIdentity === assemblyIdentity,
    );
    if (new Set(replicas.map((replica) => replica.replicaId)).size === 2) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
  }
  throw new Error('Actor acceptance assembly did not activate on two Runtime replicas');
}

async function readHealth(controlUrl, signal) {
  const response = await fetch(`${controlUrl}/__router/health`, { signal });
  assert.equal(response.ok, true);
  return response.json();
}
