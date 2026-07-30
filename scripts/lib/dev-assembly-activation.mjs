import { randomUUID } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';

import {
  defaultAssemblyActivationUrl,
  maxExpectedAssemblyGeneration,
  requestAssemblyActivation,
} from './package-service-authoring.mjs';

const runtimeAssemblyIdentityPattern =
  /^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$/;
const runtimeConfigSnapshotIdPattern =
  /^skiff-runtime-config-snapshot-v1:[0-9a-f]{32}$/;

export async function activateDevAssembly({
  fetchImpl = fetch,
  activationUrl = defaultAssemblyActivationUrl,
  activationId = `skiff-dev-${randomUUID()}`,
  environment,
  assembly,
  configSnapshot,
  wait = delay,
  maxAttempts = 5,
}) {
  if (!Number.isSafeInteger(maxAttempts) || maxAttempts <= 0) {
    throw new Error('dev activation maxAttempts must be a positive integer');
  }
  let lastError;
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    const active = await readRouterActivationState({ fetchImpl, activationUrl });
    assertMatchingRouterEnvironment(active, environment);
    if (matchesActivationTarget(active, { environment, assembly, configSnapshot })) {
      return alreadyCommittedActivation({
        activationId,
        environment,
        assembly,
        configSnapshot,
        active,
      });
    }
    try {
      return await requestAssemblyActivation({
        fetchImpl,
        activationUrl,
        activationId,
        expectedGeneration: active.generation,
        environment,
        assembly,
        configSnapshot,
      });
    } catch (error) {
      lastError = error;
      const observed = await readRouterActivationState({ fetchImpl, activationUrl });
      assertMatchingRouterEnvironment(observed, environment);
      if (matchesActivationTarget(observed, { environment, assembly, configSnapshot })) {
        return alreadyCommittedActivation({
          activationId,
          environment,
          assembly,
          configSnapshot,
          active: observed,
        });
      }
      if (!isActivationConflict(error) || attempt + 1 >= maxAttempts) {
        throw error;
      }
      await wait(Math.min(50 * (2 ** attempt), 800));
    }
  }
  throw lastError ?? new Error('dev activation exhausted its retry budget');
}

export async function readRouterActivationState({
  fetchImpl = fetch,
  activationUrl = defaultAssemblyActivationUrl,
}) {
  const healthUrl = new URL('/__router/health', activationUrl).toString();
  let response;
  try {
    response = await fetchImpl(healthUrl, {
      method: 'GET',
      headers: { accept: 'application/json' },
    });
  } catch (error) {
    throw new Error(`router health request failed for ${healthUrl}: ${formatError(error)}`);
  }
  const text = await response.text();
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    throw new Error(`router health ${healthUrl} returned invalid JSON`);
  }
  if (!response.ok) {
    throw new Error(`router health ${healthUrl} rejected with HTTP ${response.status}`);
  }
  const active = body?.activeAssembly;
  if (
    body?.ok !== true
    || !isPlainObject(active)
    || typeof active.environment !== 'string'
    || !Number.isSafeInteger(active.generation)
    || active.generation < 0
    || active.generation > maxExpectedAssemblyGeneration
    || typeof active.assemblyIdentity !== 'string'
    || !runtimeAssemblyIdentityPattern.test(active.assemblyIdentity)
    || typeof active.configSnapshotId !== 'string'
    || !runtimeConfigSnapshotIdPattern.test(active.configSnapshotId)
  ) {
    throw new Error(`router health ${healthUrl} did not return an exact active assembly tuple`);
  }
  return {
    environment: active.environment,
    generation: active.generation,
    assembly: { assemblyIdentity: active.assemblyIdentity },
    configSnapshot: { snapshotId: active.configSnapshotId },
  };
}

function alreadyCommittedActivation({
  activationId,
  environment,
  assembly,
  configSnapshot,
  active,
}) {
  return {
    request: {
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment,
      activationId,
      expectedGeneration: active.generation,
      assembly,
      configSnapshot,
    },
    response: {
      ok: true,
      committed: {
        environment,
        generation: active.generation,
        assembly,
        configSnapshot,
      },
      activeAssembly: {
        environment,
        generation: active.generation,
        assemblyIdentity: assembly.assemblyIdentity,
        configSnapshotId: configSnapshot.snapshotId,
      },
      idempotent: true,
    },
  };
}

function assertMatchingRouterEnvironment(active, environment) {
  if (active.environment !== environment) {
    throw new Error(
      `router coordinates environment ${active.environment}, not requested ${environment}`,
    );
  }
}

function matchesActivationTarget(active, target) {
  return active.environment === target.environment
    && active.assembly.assemblyIdentity === target.assembly.assemblyIdentity
    && active.configSnapshot.snapshotId === target.configSnapshot.snapshotId;
}

function isActivationConflict(error) {
  return /assembly activation rejected with HTTP 409(?:\b|:)/.test(formatError(error));
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function formatError(error) {
  return error instanceof Error ? error.message : String(error);
}
