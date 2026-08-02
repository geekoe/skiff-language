// Implementation-neutral actor full-chain HTTP driver + frame projection
// (plan §7 E-actor-parity, §8 router-live:actor, §9 differential).
//
// The driver sends the exact same real HTTP unary sequence (trusted
// service/version selectors) to each isolated Router side; both sides run two
// real Runtime replicas through test-only relays. The captured observation
// contains:
// - `http.steps`: deterministic per-step status + parsed body (failure steps
//   carry the platform error code/message and normalized detail ids);
// - `frameEvents`: projected semantic frame sequences per replica (ephemeral
//   correlation ids tokenized per key, timestamps normalized, payloads hashed,
//   handshake/health frames excluded);
// - `health`, `mongo`, `terminal` (same shape as the shared harness).

import { request as requestHttp } from 'node:http';

import {
  ACTOR_PARITY_EXCLUDED_FRAME_TYPES,
  ACTOR_PARITY_POLL_STEPS,
  ACTOR_PARITY_STEPS,
  ACTOR_PARITY_TIMESTAMP_KEYS,
  ACTOR_PARITY_TOKEN_KEYS,
} from './actor_parity_constants.mjs';

const ISO_TIMESTAMP_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/;
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const LONG_TOKEN_PATTERN = /^[0-9a-f]{16,128}$/i;

const TOKEN_KEYS = new Set(ACTOR_PARITY_TOKEN_KEYS);
const TIMESTAMP_KEYS = new Set(ACTOR_PARITY_TIMESTAMP_KEYS);

// request.start fields kept in the compared projection; all other fields
// (ephemeral url/trace/deadline detail) are recordOnly raw evidence.
const REQUEST_START_FIELD_PATHS = [
  ['mode'],
  ['caller', 'kind'],
  ['routing', 'kind'],
  ['routing', 'assemblyIdentity'],
  ['routing', 'assemblyGeneration'],
  ['routing', 'deployment', 'serviceId'],
  ['routing', 'deployment', 'contractVersion'],
  ['routing', 'gatewayEntryIdentity'],
  ['routing', 'ingress', 'protocol'],
  ['routing', 'ingress', 'method'],
  ['routing', 'ingress', 'path'],
  ['requestId'],
  ['deadline', 'timeoutMs'],
  ['httpRequest', 'method'],
  ['httpRequest', 'path'],
];

export async function runActorParityFullChain({
  httpPort,
  entrypoints,
  deployment,
  requestTimeoutMs = 30_000,
}) {
  const serviceId = deployment.serviceId;
  const contractVersion = deployment.contractVersion;
  const invoke = async (entrypoint, bodyValue = null) => {
    const response = await new Promise((resolve, reject) => {
      const request = requestHttp(
        `http://127.0.0.1:${httpPort}${entrypoint.path}`,
        {
          method: entrypoint.method ?? 'POST',
          headers: {
            'x-skiff-service': serviceId,
            'x-skiff-version': contractVersion,
            'content-type': 'application/json',
          },
        },
        resolve,
      );
      request.setTimeout(requestTimeoutMs, () => {
        request.destroy(new Error(`request timeout for ${entrypoint.path}`));
      });
      request.once('error', reject);
      request.end(JSON.stringify(bodyValue));
    });
    const chunks = [];
    for await (const chunk of response) {
      chunks.push(Buffer.from(chunk));
    }
    const body = Buffer.concat(chunks).toString('utf8');
    return { status: response.statusCode, body };
  };

  const steps = [];
  const timings = {};
  const record = (name, result, extra = {}) => {
    const normalized = normalizeErrorBody(result.body);
    steps.push({
      name,
      status: result.status,
      body: normalized.errorBody ?? parseBody(result.body),
      ...extra,
    });
  };

  for (const step of ACTOR_PARITY_STEPS) {
    const entrypoint = requireEntrypoint(entrypoints, step.entrypoint);
    const startedAt = Date.now();
    const result = await invoke(entrypoint);
    const elapsedMs = Date.now() - startedAt;
    if (result.status !== step.expectStatus) {
      throw new Error(
        `actor parity step ${step.name} expected HTTP ${step.expectStatus}, got ${result.status}: ${result.body}`,
      );
    }
    const body = parseBody(result.body);
    if (body !== step.expectBody) {
      throw new Error(
        `actor parity step ${step.name} expected body ${JSON.stringify(step.expectBody)}, got ${JSON.stringify(body)}`,
      );
    }
    if (step.minElapsedMs !== undefined && elapsedMs < step.minElapsedMs) {
      throw new Error(
        `actor parity step ${step.name} returned before ${step.minElapsedMs}ms (${elapsedMs}ms)`,
      );
    }
    if (step.maxElapsedMs !== undefined && elapsedMs > step.maxElapsedMs) {
      throw new Error(
        `actor parity step ${step.name} waited for the target (${elapsedMs}ms > ${step.maxElapsedMs}ms)`,
      );
    }
    timings[step.name] = elapsedMs;
    record(step.name, result);
  }

  // Concurrent gets for one fresh id dedup onto a single activation and both
  // wait for the same create (frozen TS/Rust full-chain semantics).
  const dedupEntrypoint = requireEntrypoint(entrypoints, 'slowDedup');
  const dedupStartedAt = Date.now();
  const [dedupLeft, dedupRight] = await Promise.all([
    invoke(dedupEntrypoint),
    invoke(dedupEntrypoint),
  ]);
  const dedupElapsedMs = Date.now() - dedupStartedAt;
  for (const result of [dedupLeft, dedupRight]) {
    if (result.status !== 200 || parseBody(result.body) !== 'slow-get-ok') {
      throw new Error(
        `actor parity step slow-dedup failed: ${result.status} ${result.body}`,
      );
    }
  }
  if (dedupElapsedMs < 200) {
    throw new Error(
      `actor parity concurrent gets did not wait for one create: ${dedupElapsedMs}ms`,
    );
  }
  timings['slow-dedup'] = dedupElapsedMs;
  record('slow-dedup', { status: 200, body: '"slow-get-ok"' }, {
    concurrent: 2,
  });

  // Create failure surfaces on get; the retained entry keeps failing.
  const flakyEntrypoint = requireEntrypoint(entrypoints, 'flakyGet');
  for (const name of ['flaky-get-1', 'flaky-get-2']) {
    const result = await invoke(flakyEntrypoint);
    if (result.status === 200) {
      throw new Error(`actor parity step ${name} must fail, got 200 ${result.body}`);
    }
    const errorBody = parseErrorBody(result.body);
    if (!/UnhandledServiceError|InternalError|ProviderUnavailable/.test(
      `${errorBody.code} ${errorBody.message}`,
    )) {
      throw new Error(
        `actor parity step ${name} failure must be a platform error, got ${result.body}`,
      );
    }
    record(name, { status: result.status, body: result.body });
  }

  // Poll steps wait for deterministic actor values produced by spawns.
  for (const poll of ACTOR_PARITY_POLL_STEPS) {
    const entrypoint = requireEntrypoint(entrypoints, poll.entrypoint);
    const startedAt = Date.now();
    let last;
    for (;;) {
      const result = await invoke(entrypoint);
      if (result.status !== 200) {
        throw new Error(
          `actor parity poll ${poll.name} failed with HTTP ${result.status}: ${result.body}`,
        );
      }
      last = parseBody(result.body);
      if (last === poll.expected) {
        break;
      }
      if (Date.now() - startedAt > (poll.timeoutMs ?? 15_000)) {
        throw new Error(
          `actor parity poll ${poll.name} expected ${JSON.stringify(poll.expected)}, `
          + `last ${JSON.stringify(last)} within ${poll.timeoutMs ?? 15_000}ms`,
        );
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    record(poll.name, { status: 200, body: JSON.stringify(last) });
  }

  return { steps, timings };
}

export function projectActorParityFrameEvents(relays) {
  const perReplica = new Map();
  for (const { replica, records } of relays) {
    const events = [];
    const tokenCounters = new Map();
    for (const record of records) {
      if (typeof record.type !== 'string') {
        continue;
      }
      if (ACTOR_PARITY_EXCLUDED_FRAME_TYPES.has(record.type)) {
        continue;
      }
      const fields = record.type === 'request.start'
        ? projectRequestStart(record.header)
        : projectGeneric(record.header, tokenCounters);
      events.push({
        direction: record.direction,
        type: record.type,
        replica,
        payloadSha256: record.payloadSha256 ?? null,
        fields,
      });
    }
    perReplica.set(replica, events);
  }
  return Object.fromEntries(perReplica);
}

function projectRequestStart(header) {
  const fields = {};
  for (const path of REQUEST_START_FIELD_PATHS) {
    const value = readPath(header, path);
    if (value !== undefined) {
      writePath(fields, path, tokenizeValue(path[path.length - 1], value));
    }
  }
  return fields;
}

function projectGeneric(header, tokenCounters) {
  return walk(header, tokenCounters, ['type']);
}

function walk(value, tokenCounters, droppedKeys = []) {
  if (Array.isArray(value)) {
    return value.map((entry) => walk(entry, tokenCounters, droppedKeys));
  }
  if (value === null || typeof value !== 'object') {
    return value;
  }
  const result = {};
  for (const [key, entry] of Object.entries(value)) {
    if (droppedKeys.includes(key) || key === 'schemaVersion' || key === 'observedAt') {
      continue;
    }
    if (TOKEN_KEYS.has(key)) {
      result[key] = tokenize(key, entry, tokenCounters);
      continue;
    }
    if (TIMESTAMP_KEYS.has(key) && typeof entry === 'string') {
      result[key] = '<timestamp>';
      continue;
    }
    if (key === 'httpRequest') {
      result[key] = pick(entry, ['method', 'path']);
      continue;
    }
    if (key === 'trace') {
      result[key] = pick(entry, ['traceId', 'spanId', 'parentSpanId']);
      continue;
    }
    result[key] = walk(entry, tokenCounters, droppedKeys);
  }
  return result;
}

function tokenize(key, value, tokenCounters) {
  if (typeof value === 'string') {
    const counter = tokenCounters.get(key) ?? 0;
    tokenCounters.set(key, counter + 1);
    return `<${key}-${counter + 1}>`;
  }
  return value;
}

function tokenizeValue(key, value) {
  if (typeof value === 'string' && TOKEN_KEYS.has(key)) {
    return `<${key}-token>`;
  }
  if (typeof value === 'string' && TIMESTAMP_KEYS.has(key)) {
    return '<timestamp>';
  }
  return value;
}

function pick(object, keys) {
  if (object === null || typeof object !== 'object') {
    return object;
  }
  const result = {};
  for (const key of keys) {
    if (Object.hasOwn(object, key)) {
      result[key] = object[key];
    }
  }
  return result;
}

function readPath(value, segments) {
  let current = value;
  for (const segment of segments) {
    if (current === null || typeof current !== 'object') {
      return undefined;
    }
    if (!Object.hasOwn(current, segment)) {
      return undefined;
    }
    current = current[segment];
  }
  return current;
}

function writePath(object, segments, value) {
  let current = object;
  for (const segment of segments.slice(0, -1)) {
    current[segment] ??= {};
    current = current[segment];
  }
  current[segments.at(-1)] = value;
}

function requireEntrypoint(entrypoints, key) {
  const entrypoint = entrypoints[key];
  if (entrypoint === undefined) {
    throw new Error(`actor parity entrypoint ${key} is missing`);
  }
  return entrypoint;
}

function parseBody(text) {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function normalizeErrorBody(text) {
  const parsed = parseErrorBody(text);
  if (parsed === undefined) {
    return {};
  }
  return {
    errorBody: {
      code: parsed.code,
      message: parsed.message,
      details: normalizeDetails(parsed.details),
    },
  };
}

function parseErrorBody(text) {
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch {
    return undefined;
  }
  const error = parsed?.error;
  if (error === null || typeof error !== 'object') {
    return undefined;
  }
  return {
    code: typeof error.code === 'string' ? error.code : String(error.code),
    message: typeof error.message === 'string' ? error.message : '',
    details: error.details,
  };
}

function normalizeDetails(details) {
  if (typeof details === 'string') {
    return normalizeOpaque(details);
  }
  if (Array.isArray(details)) {
    return details.map(normalizeDetails);
  }
  if (details !== null && typeof details === 'object') {
    return Object.fromEntries(
      Object.entries(details).map(([key, value]) => [
        key,
        key === 'traceId' || key === 'errorId' ? normalizeOpaque(value) : normalizeDetails(value),
      ]),
    );
  }
  return details;
}

function normalizeOpaque(value) {
  if (typeof value !== 'string') {
    return value;
  }
  if (UUID_PATTERN.test(value) || LONG_TOKEN_PATTERN.test(value) || ISO_TIMESTAMP_PATTERN.test(value)) {
    return '<opaque>';
  }
  return value;
}
