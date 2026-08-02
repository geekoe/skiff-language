// E-http assertion suites (`router-live:http`, plan §7/§8).
//
// `runRollbackSuite` is executed once per TS→Rust→TS phase and asserts the
// same observable unary/stream behavior on every Router implementation.
// `runFullSuite` runs the complete E-http surface on the Rust phase: trusted
// selectors, service-scoped ingress, typed/raw payloads, unary/stream
// mapping and sequencing, cumulative ceiling, backpressure, disconnect/
// cancel/deadline, CORS preflight/service-managed and platform errors, with
// every race asserting one external terminal, at most one cancel frame and a
// successful follow-up unary (pending/permit/timer residue proxy).

import {
  HTTP_LIVE_SERVICE_ID,
  HTTP_LIVE_VERSION,
} from './http_live_fixture.mjs';
import {
  openHttpLiveStream,
  requestFull,
  selectorHeaders,
} from './http_live_client.mjs';
import { waitForCancelFrame } from './http_live_process.mjs';

const FIXED_ERROR_PATTERN = /^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$/;

export async function runRollbackSuite(ctx) {
  const evidence = [];
  await caseUnaryHappy(ctx, evidence);
  await caseTypedUnary(ctx, evidence);
  await caseMissingSelector(ctx, evidence);
  await caseWrongPath(ctx, evidence);
  await caseStreamRoundtrip(ctx, evidence);
  return evidence;
}

export async function runFullSuite(ctx) {
  const evidence = await runRollbackSuite(ctx);
  await caseVersionConflict(ctx, evidence);
  await caseUnknownService(ctx, evidence);
  await caseWrongMethod(ctx, evidence);
  await caseBodyTooLarge(ctx, evidence);
  await caseUnaryCeiling(ctx, evidence);
  await caseStreamCeiling(ctx, evidence);
  await caseServiceError(ctx, evidence);
  await caseAutomaticCorsPreflight(ctx, evidence);
  await caseServiceManagedCors(ctx, evidence);
  await caseDeadlineUnary(ctx, evidence);
  await caseDeadlineStream(ctx, evidence);
  await caseDisconnectStream(ctx, evidence);
  await caseBackpressure(ctx, evidence);
  return evidence;
}

async function caseUnaryHappy(ctx, evidence) {
  const body = Buffer.from(`rollback-${ctx.phase}`, 'utf8');
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'POST',
    path: '/unary',
    headers: serviceHeaders(ctx),
    body,
  });
  assertEqual(response.status, 201, `${ctx.phase} unary status`);
  assertEqual(
    response.body.toString('utf8'),
    `unary:rollback-${ctx.phase}`,
    `${ctx.phase} unary body`,
  );
  const records = newRecords(ctx.relay, before);
  const requestId = assertSingleDispatch(records, {
    path: '/unary',
    mode: 'unary',
    serviceId: ctx.serviceId,
    version: ctx.version,
  });
  assertCancelReasons(records, requestId, [], `${ctx.phase} unary cancels`);
  evidence.push({ phase: ctx.phase, name: 'unary-happy', status: response.status });
  return requestId;
}

async function caseTypedUnary(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'POST',
    path: '/typed-unary',
    headers: serviceHeaders(ctx),
    body: Buffer.from('"hello"', 'utf8'),
  });
  assertEqual(response.status, 200, `${ctx.phase} typed unary status`);
  assertEqual(
    JSON.parse(response.body.toString('utf8')),
    'typed:hello',
    `${ctx.phase} typed unary body`,
  );
  const records = newRecords(ctx.relay, before);
  assertSingleDispatch(records, {
    path: '/typed-unary',
    mode: 'unary',
    serviceId: ctx.serviceId,
    version: ctx.version,
  });
  evidence.push({ phase: ctx.phase, name: 'typed-unary', status: response.status });
}

async function caseMissingSelector(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'POST',
    path: '/unary',
    headers: {},
    body: Buffer.from('x', 'utf8'),
  });
  assertJsonError(response, 400, 'ServiceSelectorRequired', `${ctx.phase} missing selector`);
  assertNoDispatch(newRecords(ctx.relay, before), `${ctx.phase} missing selector dispatch`);
  evidence.push({ phase: ctx.phase, name: 'missing-selector', status: response.status });
}

async function caseVersionConflict(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'POST',
    path: '/unary',
    headers: {
      ...selectorHeaders({ service: ctx.serviceId, version: ctx.version }),
      'x-skiff-release': '9.9.9',
    },
    body: Buffer.from('x', 'utf8'),
  });
  assertJsonError(response, 400, 'InvalidVersionHeader', `${ctx.phase} version conflict`);
  assertNoDispatch(newRecords(ctx.relay, before), `${ctx.phase} version conflict dispatch`);
  evidence.push({ phase: ctx.phase, name: 'version-conflict', status: response.status });
}

async function caseWrongPath(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'GET',
    path: '/missing',
    headers: serviceHeaders(ctx),
  });
  assertJsonError(response, 404, 'AssemblyIngressNotFound', `${ctx.phase} wrong path`);
  assertNoDispatch(newRecords(ctx.relay, before), `${ctx.phase} wrong path dispatch`);
  evidence.push({ phase: ctx.phase, name: 'wrong-path', status: response.status });
}

async function caseStreamRoundtrip(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'POST',
    path: '/stream',
    headers: serviceHeaders(ctx),
    body: Buffer.from('middle', 'utf8'),
  });
  assertEqual(response.status, 206, `${ctx.phase} stream status`);
  assertEqual(
    response.body.toString('utf8'),
    'alpha|middle|omega',
    `${ctx.phase} stream body`,
  );
  const records = newRecords(ctx.relay, before);
  const requestId = assertSingleDispatch(records, {
    path: '/stream',
    mode: 'serverStream',
    serviceId: ctx.serviceId,
    version: ctx.version,
  });
  assertStreamFrames(records, requestId, `${ctx.phase} stream frames`);
  assertCancelReasons(records, requestId, [], `${ctx.phase} stream cancels`);
  evidence.push({ phase: ctx.phase, name: 'stream-roundtrip', status: response.status });
}

async function caseUnknownService(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'GET',
    path: '/unary',
    headers: selectorHeaders({ service: 'test.skiff/unknown', version: ctx.version }),
  });
  assertJsonError(response, 404, 'AssemblyIngressNotFound', 'unknown service');
  assertNoDispatch(newRecords(ctx.relay, before), 'unknown service dispatch');
  evidence.push({ phase: ctx.phase, name: 'unknown-service', status: response.status });
}

async function caseWrongMethod(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'GET',
    path: '/unary',
    headers: serviceHeaders(ctx),
  });
  assertJsonError(response, 404, 'AssemblyIngressNotFound', 'wrong method');
  assertNoDispatch(newRecords(ctx.relay, before), 'wrong method dispatch');
  evidence.push({ phase: ctx.phase, name: 'wrong-method', status: response.status });
}

async function caseBodyTooLarge(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'POST',
    path: '/unary',
    headers: serviceHeaders(ctx),
    body: Buffer.alloc(70_000, 'x'),
  });
  assertJsonError(response, 413, 'RequestTooLarge', 'request body limit');
  assertNoDispatch(newRecords(ctx.relay, before), 'request body limit dispatch');
  evidence.push({ phase: ctx.phase, name: 'body-too-large', status: response.status });
}

async function caseUnaryCeiling(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'POST',
    path: '/echo',
    headers: serviceHeaders(ctx),
    body: Buffer.alloc(8192, 'e'),
  });
  assertJsonError(response, 502, 'ResponseTooLarge', 'unary response ceiling');
  const records = newRecords(ctx.relay, before);
  const requestId = latestRequestId(records);
  assertCancelReasons(records, requestId, [], 'unary ceiling cancels');
  evidence.push({ phase: ctx.phase, name: 'unary-ceiling', status: response.status });
}

async function caseStreamCeiling(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const stream = await openHttpLiveStream({
    port: ctx.port,
    method: 'POST',
    path: '/echo-stream',
    headers: serviceHeaders(ctx),
    body: Buffer.alloc(8192, 's'),
  });
  assertEqual(stream.status, 200, 'stream ceiling head status');
  const chunks = [];
  while (true) {
    const chunk = await stream.readChunk();
    if (chunk === null) {
      break;
    }
    chunks.push(chunk);
  }
  const bodyLength = chunks.reduce((total, chunk) => total + chunk.length, 0);
  if (bodyLength >= 8192) {
    throw new Error(`stream ceiling body must be truncated, got ${bodyLength} bytes`);
  }
  const records = newRecords(ctx.relay, before);
  const requestId = latestRequestId(records);
  const reasons = cancelReasons(records, requestId);
  assert(
    reasons.length === 1 && reasons[0] === 'protocol_error',
    `stream ceiling must cancel exactly once with protocol_error, got ${JSON.stringify(reasons)}`,
  );
  evidence.push({ phase: ctx.phase, name: 'stream-ceiling', bodyLength });
}

async function caseServiceError(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'GET',
    path: '/error',
    headers: serviceHeaders(ctx),
  });
  assertEqual(response.status, 500, 'service error status');
  const body = parseJson(response.body);
  assertEqual(body?.error?.code, 'FixedServiceError', 'service error code');
  assertEqual(body?.error?.message, 'Service request failed', 'service error message');
  assert(
    typeof body?.error?.details?.traceId === 'string'
      && body.error.details.traceId.length > 0,
    'service error details must carry traceId',
  );
  assert(
    typeof body?.error?.details?.errorId === 'string'
      && body.error.details.errorId.length > 0,
    'service error details must carry errorId',
  );
  assertSingleDispatch(newRecords(ctx.relay, before), {
    path: '/error',
    mode: 'unary',
    serviceId: ctx.serviceId,
    version: ctx.version,
  });
  evidence.push({ phase: ctx.phase, name: 'service-error', status: response.status });
}

async function caseAutomaticCorsPreflight(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'OPTIONS',
    path: '/unary',
    headers: {
      ...serviceHeaders(ctx),
      origin: 'https://caller.example',
      'access-control-request-method': 'POST',
    },
  });
  assertEqual(response.status, 204, 'automatic preflight status');
  assertEqual(
    response.headers['access-control-allow-origin'],
    'https://caller.example',
    'automatic preflight allow-origin',
  );
  assert(
    String(response.headers['access-control-allow-methods'] ?? '').includes('OPTIONS'),
    'automatic preflight allow-methods must include OPTIONS',
  );
  assertNoDispatch(newRecords(ctx.relay, before), 'automatic preflight dispatch');
  evidence.push({ phase: ctx.phase, name: 'cors-preflight', status: response.status });
}

async function caseServiceManagedCors(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'OPTIONS',
    path: '/cors',
    headers: {
      ...serviceHeaders(ctx),
      origin: 'https://caller.example',
      'access-control-request-method': 'POST',
    },
  });
  assertEqual(response.status, 204, 'service-managed CORS status');
  assertEqual(
    response.headers['access-control-allow-origin'],
    'https://service.example',
    'service-managed CORS allow-origin',
  );
  assertSingleDispatch(newRecords(ctx.relay, before), {
    path: '/cors',
    mode: 'unary',
    serviceId: ctx.serviceId,
    version: ctx.version,
  });
  evidence.push({ phase: ctx.phase, name: 'service-managed-cors', status: response.status });
}

async function caseDeadlineUnary(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'GET',
    path: '/slow-unary',
    headers: serviceHeaders(ctx),
  });
  assertJsonError(response, 504, 'TimeoutError', 'unary deadline');
  const records = newRecords(ctx.relay, before);
  const requestId = latestRequestId(records);
  const reasons = await waitForCancelFrame(ctx.relay, before.index, requestId);
  assert(
    reasons.length === 1 && reasons[0] === 'timeout',
    `unary deadline must cancel exactly once with timeout, got ${JSON.stringify(reasons)}`,
  );
  await assertFollowUpUnary(ctx, 'unary-deadline');
  evidence.push({ phase: ctx.phase, name: 'deadline-unary', status: response.status });
}

async function caseDeadlineStream(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const response = await requestFull({
    port: ctx.port,
    method: 'GET',
    path: '/slow',
    headers: serviceHeaders(ctx),
  });
  assertJsonError(response, 504, 'TimeoutError', 'stream deadline');
  const records = newRecords(ctx.relay, before);
  const requestId = latestRequestId(records);
  const reasons = await waitForCancelFrame(ctx.relay, before.index, requestId);
  assert(
    reasons.length === 1 && reasons[0] === 'timeout',
    `stream deadline must cancel exactly once with timeout, got ${JSON.stringify(reasons)}`,
  );
  await assertFollowUpUnary(ctx, 'stream-deadline');
  evidence.push({ phase: ctx.phase, name: 'deadline-stream', status: response.status });
}

async function caseDisconnectStream(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const stream = await openHttpLiveStream({
    port: ctx.port,
    method: 'GET',
    path: '/slow-stream',
    headers: serviceHeaders(ctx),
  });
  assertEqual(stream.status, 200, 'disconnect head status');
  const first = await stream.readChunk();
  assertEqual(first?.toString('utf8'), 'first', 'disconnect first chunk');
  const records = newRecords(ctx.relay, before);
  const requestId = latestRequestId(records);
  stream.destroy();
  const reasons = await waitForCancelFrame(ctx.relay, before.index, requestId);
  assert(
    reasons.length === 1 && reasons[0] === 'client_disconnect',
    `client disconnect must cancel exactly once with client_disconnect, got ${JSON.stringify(reasons)}`,
  );
  await assertFollowUpUnary(ctx, 'client-disconnect');
  evidence.push({ phase: ctx.phase, name: 'disconnect-stream', cancel: reasons[0] });
}

async function caseBackpressure(ctx, evidence) {
  const before = snapshot(ctx.relay);
  const stream = await openHttpLiveStream({
    port: ctx.port,
    method: 'POST',
    path: '/burst',
    headers: serviceHeaders(ctx),
  });
  assertEqual(stream.status, 200, 'backpressure head status');
  // Deliberately do not read the body: the bounded stream channel fills and
  // the drain deadline produces the backpressure terminal.
  const records = newRecords(ctx.relay, before);
  const requestId = latestRequestId(records);
  const reasons = await waitForCancelFrame(ctx.relay, before.index, requestId, {
    timeoutMs: 35_000,
  });
  stream.destroy();
  assert(
    reasons.length === 1 && reasons[0] === 'backpressure',
    `backpressure must cancel exactly once with backpressure, got ${JSON.stringify(reasons)}`,
  );
  await assertFollowUpUnary(ctx, 'backpressure');
  evidence.push({ phase: ctx.phase, name: 'backpressure', cancel: reasons[0] });
}

async function assertFollowUpUnary(ctx, label) {
  const response = await requestFull({
    port: ctx.port,
    method: 'POST',
    path: '/unary',
    headers: serviceHeaders(ctx),
    body: Buffer.from(`after-${label}`, 'utf8'),
  });
  assertEqual(response.status, 201, `${label} follow-up unary status`);
  assertEqual(
    response.body.toString('utf8'),
    `unary:after-${label}`,
    `${label} follow-up unary body`,
  );
}

function assertSingleDispatch(records, { path, mode, serviceId, version }) {
  const starts = requestStartRecords(records);
  assert(
    starts.length === 1,
    `expected exactly one request.start for ${path}, got ${starts.length}`,
  );
  const header = starts[0].header;
  assertEqual(header.mode, mode, `${path} request mode`);
  assertEqual(header.routing?.deployment?.serviceId, serviceId, `${path} service id`);
  assertEqual(
    header.routing?.deployment?.contractVersion,
    version,
    `${path} contract version`,
  );
  assertEqual(header.routing?.ingress?.path, path, `${path} ingress path`);
  assert(
    FIXED_ERROR_PATTERN.test(header.routing?.gatewayEntryIdentity ?? ''),
    `${path} gateway entry identity shape`,
  );
  return header.requestId;
}

function assertNoDispatch(records, label) {
  assert(
    requestStartRecords(records).length === 0,
    `${label} must not dispatch (no request.start frame)`,
  );
}

function assertStreamFrames(records, requestId, label) {
  const frames = records.filter(
    (record) => (
      ['response.start', 'response.chunk', 'response.end'].includes(record.type)
      && record.header?.requestId === requestId
    ),
  );
  assertEqual(
    frames.map((frame) => frame.type),
    ['response.start', 'response.chunk', 'response.chunk', 'response.chunk', 'response.end'],
    `${label} response frame sequence`,
  );
  assertEqual(
    frames
      .filter((frame) => frame.type === 'response.chunk')
      .map((frame) => frame.header?.seq),
    [0, 1, 2],
    `${label} chunk sequence`,
  );
}

function assertCancelReasons(records, requestId, expected, label) {
  assertEqual(cancelReasons(records, requestId), expected, label);
}

function cancelReasons(records, requestId) {
  if (requestId === null || requestId === undefined) {
    return [];
  }
  return records
    .filter((record) => record.type === 'request.cancel' && record.header?.requestId === requestId)
    .map((record) => record.header?.reason);
}

function latestRequestId(records) {
  const starts = requestStartRecords(records);
  if (starts.length === 0) {
    return null;
  }
  return starts[starts.length - 1].header.requestId;
}

function requestStartRecords(records) {
  return records.filter(
    (record) => record.type === 'request.start' && record.header?.requestId,
  );
}

function snapshot(relay) {
  return { index: relay.records.length };
}

function newRecords(relay, before) {
  return relay.records.slice(before.index);
}

function serviceHeaders(ctx) {
  return selectorHeaders({ service: ctx.serviceId, version: ctx.version });
}

function assertJsonError(response, status, code, label) {
  assertEqual(response.status, status, `${label} status`);
  const body = parseJson(response.body);
  assertEqual(body?.error?.code, code, `${label} code`);
  assert(
    typeof body?.error?.message === 'string' && body.error.message.length > 0,
    `${label} message`,
  );
}

function parseJson(buffer) {
  try {
    return JSON.parse(buffer.toString('utf8'));
  } catch (error) {
    throw new Error(`expected JSON error body, got ${JSON.stringify(buffer.toString('utf8'))}`, {
      cause: error,
    });
  }
}

function assertEqual(actual, expected, label) {
  const same = Array.isArray(actual) && Array.isArray(expected)
    ? JSON.stringify(actual) === JSON.stringify(expected)
    : Object.is(actual, expected);
  if (!same) {
    throw new Error(
      `${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

export const httpLiveSuiteInternals = {
  HTTP_LIVE_SERVICE_ID,
  HTTP_LIVE_VERSION,
};
