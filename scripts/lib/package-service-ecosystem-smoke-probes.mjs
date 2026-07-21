import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import http from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';

export async function waitForCanonicalRouter({ controlUrl, supervisor, signal }) {
  const startedAt = Date.now();
  let lastError;
  while (Date.now() - startedAt < 120_000) {
    signal.throwIfAborted();
    if (supervisor.exitCode !== null || supervisor.signalCode !== null) {
      throw new Error(
        `isolated supervisor exited before router readiness with ${supervisor.signalCode ?? supervisor.exitCode}`,
      );
    }
    try {
      const health = await routerHealth(controlUrl, signal);
      if (health.ok === true) return;
    } catch (error) {
      lastError = error;
    }
    await delay(100, undefined, { signal });
  }
  throw new Error(
    `canonical isolated router was not ready at ${controlUrl}: ${lastError?.message ?? 'timeout'}`,
  );
}

// F03B alignment hook: health must keep capability handshakes separate from
// exact committed-generation registrations. T05 deliberately reads no runtime
// socket internals and does not duplicate the binary frame codec.
export async function waitForRuntimeEvidence(controlUrl, expected, signal) {
  const startedAt = Date.now();
  let observedCapabilities = [];
  let observedRegistrations = [];
  while (Date.now() - startedAt < 30_000) {
    signal.throwIfAborted();
    const health = await routerHealth(controlUrl, signal);
    observedCapabilities = capabilityConnectionIds(health);
    observedRegistrations = committedReplicaIds(health, expected);
    if (
      observedCapabilities.length === expected.capabilityCount
      && observedRegistrations.length === expected.registrationCount
    ) {
      return health;
    }
    await delay(100, undefined, { signal });
  }
  throw new Error(
    `expected ${expected.capabilityCount} capability connection(s) and ${expected.registrationCount} committed registration(s) for generation ${expected.generation}/${expected.assemblyIdentity}; observed capabilities=${observedCapabilities.join(',') || 'none'} registrations=${observedRegistrations.join(',') || 'none'}`,
  );
}

export function capabilityConnectionIds(health) {
  assert.ok(
    Array.isArray(health.capabilityConnections),
    'F03B health alignment required: capabilityConnections must be an array',
  );
  const ids = health.capabilityConnections
    .filter((entry) => entry?.connected !== false)
    .map((entry) => {
      assert.equal(
        typeof entry?.runtimeId,
        'string',
        'F03B health alignment required: capabilityConnections[].runtimeId',
      );
      return entry.runtimeId;
    });
  return [...new Set(ids)].sort();
}

export function committedReplicaIds(health, { generation, assemblyIdentity }) {
  assert.ok(
    Array.isArray(health.replicas),
    'F03B health alignment required: replicas must be an array',
  );
  return health.replicas
    .filter((entry) => (
      entry?.connected !== false
      && entry?.state === 'healthy'
      && entry?.generation === generation
      && entry?.assemblyIdentity === assemblyIdentity
    ))
    .map((entry) => {
      assert.equal(typeof entry.replicaId, 'string');
      return entry.replicaId;
    })
    .sort();
}

export function inFlightRequestCount(health) {
  assert.ok(Array.isArray(health.replicas));
  return health.replicas.reduce((total, entry) => {
    assert.ok(
      Number.isSafeInteger(entry.inFlightCount) && entry.inFlightCount >= 0,
      'F03B health alignment required: replicas[].inFlightCount',
    );
    return total + entry.inFlightCount;
  }, 0);
}

export async function waitForInFlightRequest(controlUrl, signal) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < 30_000) {
    signal.throwIfAborted();
    const health = await routerHealth(controlUrl, signal);
    if (inFlightRequestCount(health) > 0) return health;
    await delay(50, undefined, { signal });
  }
  throw new Error('generation-pinned server stream never became observable as in flight');
}

export function fixtureEntrypoint(fixture, kind) {
  const matches = fixture.candidate.entrypoints.filter((entrypoint) => entrypoint.kind === kind);
  assert.equal(matches.length, 1, `fixture must expose one ${kind} entrypoint`);
  return matches[0];
}

export function assertHostMarker(result, marker) {
  assert.ok(
    result.body.includes(marker),
    `Host result must come from ${marker}; received ${result.body.slice(0, 200)}`,
  );
}

export function openEntrypointStream(
  routerHttpUrl,
  entrypoint,
  signal,
  { startMarker, endMarker, forbiddenMarker },
) {
  const url = new URL(entrypoint.path, routerHttpUrl);
  let open = true;
  let completionResolve;
  let completionReject;
  const completion = new Promise((resolve, reject) => {
    completionResolve = resolve;
    completionReject = reject;
  });
  completion.catch(() => undefined);
  return new Promise((resolve, reject) => {
    let awaitingStart = true;
    let carry = '';
    let containsStart = false;
    let containsEnd = false;
    let containsForbidden = false;
    const hash = createHash('sha256');
    const request = http.request({
      host: url.hostname,
      port: url.port,
      path: url.pathname,
      method: entrypoint.method ?? 'GET',
      headers: { host: entrypoint.host },
      signal,
    }, (incoming) => {
      if ((incoming.statusCode ?? 500) >= 400) {
        reject(new Error(`server stream returned HTTP ${incoming.statusCode}`));
        incoming.resume();
        return;
      }
      incoming.on('data', (chunk) => {
        hash.update(chunk);
        const text = carry + chunk.toString('utf8');
        containsStart ||= text.includes(startMarker);
        containsEnd ||= text.includes(endMarker);
        containsForbidden ||= text.includes(forbiddenMarker);
        carry = text.slice(-128);
        if (awaitingStart && containsStart) {
          awaitingStart = false;
          incoming.pause();
          resolve({
            firstChunk: text,
            isOpen: () => open,
            resume: () => incoming.resume(),
            completion,
          });
        }
      });
      incoming.once('end', () => {
        open = false;
        const result = {
          containsStart,
          containsEnd,
          containsForbidden,
          sha256: hash.digest('hex'),
        };
        if (awaitingStart) {
          reject(new Error('server stream ended without an observable first chunk'));
        }
        completionResolve(result);
      });
      incoming.once('aborted', () => {
        const error = new Error('server stream response was aborted');
        open = false;
        if (awaitingStart) reject(error);
        completionReject(error);
      });
      incoming.once('error', (error) => {
        open = false;
        if (awaitingStart) reject(error);
        completionReject(error);
      });
    });
    request.once('error', (error) => {
      open = false;
      reject(error);
      completionReject(error);
    });
    request.end();
  });
}

export async function activate(
  controlUrl,
  assembly,
  expectedGeneration,
  activationId,
  environment,
  signal,
  { expectFailure = false } = {},
) {
  const response = await fetch(`${controlUrl}/__skiff/activate-assembly`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      schemaVersion: 'skiff-assembly-activation-request-v1',
      environment,
      activationId,
      expectedGeneration,
      assembly,
    }),
    signal,
  });
  const text = await response.text();
  const body = text.length === 0 ? null : JSON.parse(text);
  if (!expectFailure) {
    assert.ok(response.ok, `activation ${activationId} failed: HTTP ${response.status} ${text}`);
  }
  return { ok: response.ok, status: response.status, body };
}

export async function routerHealth(controlUrl, signal) {
  const response = await fetch(`${controlUrl}/__router/health`, { signal });
  const text = await response.text();
  if (!response.ok) throw new Error(`router health HTTP ${response.status}: ${text}`);
  return JSON.parse(text);
}

export function requestEntrypoint(routerHttpUrl, entrypoint, signal) {
  const url = new URL(entrypoint.path, routerHttpUrl);
  return new Promise((resolve, reject) => {
    const request = http.request({
      host: url.hostname,
      port: url.port,
      path: url.pathname,
      method: entrypoint.method ?? 'POST',
      headers: { host: entrypoint.host },
      signal,
    }, (response) => {
      const chunks = [];
      response.on('data', (chunk) => chunks.push(chunk));
      response.on('end', () => {
        const body = Buffer.concat(chunks).toString('utf8');
        resolve({
          ok: (response.statusCode ?? 500) < 400,
          status: response.statusCode ?? 500,
          body,
        });
      });
    });
    request.once('error', reject);
    request.end();
  });
}
