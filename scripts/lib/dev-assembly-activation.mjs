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
  profile,
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
    assertMatchingRouterProfile(active, profile);
    if (matchesActivationTarget(active, { profile, assembly, configSnapshot })) {
      return alreadyCommittedActivation({
        activationId,
        profile,
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
        profile,
        assembly,
        configSnapshot,
      });
    } catch (error) {
      lastError = error;
      const observed = await readRouterActivationState({ fetchImpl, activationUrl });
      assertMatchingRouterProfile(observed, profile);
      if (matchesActivationTarget(observed, { profile, assembly, configSnapshot })) {
        return alreadyCommittedActivation({
          activationId,
          profile,
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
    || typeof active.profile !== 'string'
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
    profile: active.profile,
    generation: active.generation,
    assembly: { assemblyIdentity: active.assemblyIdentity },
    configSnapshot: { snapshotId: active.configSnapshotId },
  };
}

function alreadyCommittedActivation({
  activationId,
  profile,
  assembly,
  configSnapshot,
  active,
}) {
  return {
    request: {
      schemaVersion: 'skiff-assembly-activation-request-v3',
      profile,
      activationId,
      expectedGeneration: active.generation,
      assembly,
      configSnapshot,
    },
    response: {
      ok: true,
      committed: {
        profile,
        generation: active.generation,
        assembly,
        configSnapshot: active.configSnapshot,
      },
      activeAssembly: {
        profile,
        generation: active.generation,
        assemblyIdentity: assembly.assemblyIdentity,
        configSnapshotId: active.configSnapshot.snapshotId,
      },
      idempotent: true,
    },
  };
}

function assertMatchingRouterProfile(active, profile) {
  if (active.profile !== profile) {
    throw new Error(
      `router coordinates profile ${active.profile}, not requested ${profile}`,
    );
  }
}

function matchesActivationTarget(active, target) {
  // Assembly identity is the authoritative "already deployed" signal: every
  // dev-sync rebuild re-derives the config snapshot id even for unchanged
  // content, so snapshot equality would never match across rebuilds.
  return active.profile === target.profile
    && active.assembly.assemblyIdentity === target.assembly.assemblyIdentity;
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
