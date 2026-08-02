// Differential extension: real HTTP traffic through trusted selectors into
// the real Runtime (`differential_ext_http_*` scenarios, plan §9).
//
// Runs a fixed, ordered case suite against each side's public HTTP port and
// returns a deterministic `httpTraffic` observation. The scenario inventory
// decides which indices are compared (equal), which are recorded as
// evidence (recordOnly), and which known non-blocking divergences are never
// asserted (release-conflict: TS 201 vs Rust 400, see differential docs).

import {
  RELEASE_HEADER,
  requestFull,
  selectorHeaders,
} from '../http_live_client.mjs';

export const EXT_HTTP_SERVICE_ID = 'test.skiff/router-rust-differential-ext-http';
export const EXT_HTTP_VERSION = '0.1.0';
export const EXT_HTTP_RELEASE_CONFLICT = '9.9.9';

const CORS_ORIGIN = 'https://caller.example';

function serviceHeaders() {
  return selectorHeaders({ service: EXT_HTTP_SERVICE_ID, version: EXT_HTTP_VERSION });
}

function corsHeaders() {
  return {
    ...serviceHeaders(),
    origin: CORS_ORIGIN,
    'access-control-request-method': 'POST',
  };
}

function releaseConflictHeaders() {
  return {
    ...serviceHeaders(),
    [RELEASE_HEADER]: EXT_HTTP_RELEASE_CONFLICT,
  };
}

function errorShape(body) {
  let parsed;
  try {
    parsed = JSON.parse(body.toString('utf8'));
  } catch {
    return { errorCode: null, errorMessage: null };
  }
  return {
    errorCode: parsed?.error?.code ?? null,
    errorMessage: parsed?.error?.message ?? null,
  };
}

export async function captureDifferentialExtHttp({ side }) {
  const traffic = [];
  const record = (name, response, extra = {}) => {
    const entry = {
      name,
      status: response.status,
      ...errorShape(response.body),
      ...extra,
    };
    if (entry.errorCode === null) {
      delete entry.errorCode;
      delete entry.errorMessage;
    }
    traffic.push(entry);
  };

  const unary = await requestFull({
    port: side.httpPort,
    method: 'POST',
    path: '/unary',
    headers: serviceHeaders(),
    body: Buffer.from('diff-payload', 'utf8'),
  });
  record('unary', unary, { body: unary.body.toString('utf8') });

  const typed = await requestFull({
    port: side.httpPort,
    method: 'POST',
    path: '/typed-unary',
    headers: serviceHeaders(),
    body: Buffer.from('"diff"', 'utf8'),
  });
  record('typed-unary', typed, { body: typed.body.toString('utf8') });

  const stream = await requestFull({
    port: side.httpPort,
    method: 'POST',
    path: '/stream',
    headers: serviceHeaders(),
    body: Buffer.from('middle', 'utf8'),
  });
  record('stream', stream, { body: stream.body.toString('utf8') });

  const serviceError = await requestFull({
    port: side.httpPort,
    method: 'GET',
    path: '/error',
    headers: serviceHeaders(),
  });
  record('service-error', serviceError);

  const missingSelector = await requestFull({
    port: side.httpPort,
    method: 'POST',
    path: '/unary',
    headers: {},
    body: Buffer.from('x', 'utf8'),
  });
  record('missing-selector', missingSelector);

  const wrongPath = await requestFull({
    port: side.httpPort,
    method: 'GET',
    path: '/missing',
    headers: serviceHeaders(),
  });
  record('wrong-path', wrongPath);

  const corsPreflight = await requestFull({
    port: side.httpPort,
    method: 'OPTIONS',
    path: '/unary',
    headers: corsHeaders(),
  });
  record('cors-preflight', corsPreflight, {
    allowOrigin: corsPreflight.headers['access-control-allow-origin'] ?? null,
    allowMethods: corsPreflight.headers['access-control-allow-methods'] ?? null,
  });

  const serviceManagedCors = await requestFull({
    port: side.httpPort,
    method: 'OPTIONS',
    path: '/cors',
    headers: corsHeaders(),
  });
  record('service-managed-cors', serviceManagedCors, {
    allowOrigin: serviceManagedCors.headers['access-control-allow-origin'] ?? null,
    allowMethods: serviceManagedCors.headers['access-control-allow-methods'] ?? null,
  });

  // Known non-blocking divergence: TS assembly gateway does not implement
  // the Release alias/conflict rule (201) while the Rust gateway freezes the
  // legacy manifest semantics (400). Recorded as evidence only; never equal.
  const releaseConflict = await requestFull({
    port: side.httpPort,
    method: 'POST',
    path: '/unary',
    headers: releaseConflictHeaders(),
    body: Buffer.from('x', 'utf8'),
  });
  record('release-conflict', releaseConflict);

  return { httpTraffic: traffic };
}
