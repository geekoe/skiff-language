import { isAbsolute } from 'node:path';

export const DEFAULT_GENERATED_ROUTER_RUNTIME_MAX_CONCURRENCY = 128;

export function renderRouterConfig({
  profile,
  host,
  artifactsPath,
  devReload,
  releaseMode,
  requestTimeoutMs = 20000,
  httpPort,
  httpMaxRequestBytes,
  httpMaxResponseBytes,
  runtimePort,
  runtimePath = '/runtime',
  runtimeMaxConcurrency = DEFAULT_GENERATED_ROUTER_RUNTIME_MAX_CONCURRENCY,
  serviceDbMongoUrl,
  telemetryEndpoint,
  rewrite = [],
  ecosystemStoreCliPath,
}) {
  if (ecosystemStoreCliPath !== undefined) {
    throw new Error('router config ecosystemStoreCliPath is not supported');
  }
  if (typeof profile !== 'string' || profile.length === 0) {
    throw new Error('router profile is required');
  }
  if (
    !/^[A-Za-z0-9._-]{1,200}$/.test(profile)
    || profile === '.'
    || profile === '..'
  ) {
    throw new Error('router profile must be a canonical ASCII token');
  }
  if (
    typeof artifactsPath !== 'string'
    || artifactsPath.trim().length === 0
    || !isAbsolute(artifactsPath)
  ) {
    throw new Error('router artifactsPath must be an absolute path');
  }
  if (typeof serviceDbMongoUrl !== 'string' || serviceDbMongoUrl.trim().length === 0) {
    throw new Error('router serviceDb.mongoUrl is required');
  }
  if (typeof host !== 'string' || host.trim().length === 0) {
    throw new Error('router host must be a non-empty string');
  }
  requirePort(httpPort, 'router http.port');
  requirePort(runtimePort, 'router runtime.port');
  if (typeof runtimePath !== 'string' || !runtimePath.startsWith('/')) {
    throw new Error('router runtime.path must start with /');
  }
  requirePositiveSafeInteger(
    requestTimeoutMs,
    'router requestTimeoutMs must be a positive safe integer',
  );
  requirePositiveSafeInteger(httpMaxRequestBytes, 'router http.maxRequestBytes');
  requirePositiveSafeInteger(httpMaxResponseBytes, 'router http.maxResponseBytes');
  requirePositiveSafeInteger(
    runtimeMaxConcurrency,
    'router runtime.maxConcurrency',
  );
  if (devReload !== undefined && typeof devReload !== 'boolean') {
    throw new Error('router devReload must be a boolean');
  }
  if (releaseMode !== undefined && typeof releaseMode !== 'boolean') {
    throw new Error('router releaseMode must be a boolean');
  }
  if (
    telemetryEndpoint !== undefined
    && (typeof telemetryEndpoint !== 'string' || telemetryEndpoint.trim().length === 0)
  ) {
    throw new Error('router telemetry.endpoint must be a non-empty string');
  }
  validateRewrite(rewrite);
  const lines = [
    `profile: ${profile}`,
    `host: ${host}`,
    `artifactsPath: ${quoteYamlString(artifactsPath)}`,
  ];
  if (releaseMode !== undefined) {
    lines.push(`releaseMode: ${releaseMode ? 'true' : 'false'}`);
  }
  lines.push(
    `devReload: ${devReload ? 'true' : 'false'}`,
    `requestTimeoutMs: ${requestTimeoutMs}`,
    '',
    'http:',
    `  port: ${httpPort}`,
    `  maxRequestBytes: ${httpMaxRequestBytes}`,
    `  maxResponseBytes: ${httpMaxResponseBytes}`,
    '',
    'runtime:',
    `  port: ${runtimePort}`,
    `  path: ${runtimePath}`,
    `  maxConcurrency: ${runtimeMaxConcurrency}`,
  );
  lines.push(
    '',
    'serviceDb:',
    `  mongoUrl: ${quoteYamlString(serviceDbMongoUrl)}`,
  );
  if (telemetryEndpoint !== undefined) {
    lines.push(
      '',
      'telemetry:',
      `  endpoint: ${quoteYamlString(telemetryEndpoint)}`,
    );
  }
  if (rewrite.length > 0) {
    lines.push('', 'rewrite:');
    for (const item of rewrite) {
      lines.push(
        `  - host: ${item.host}`,
        `    service: ${item.service}`,
        `    version: ${item.version}`,
      );
    }
  }
  lines.push('');
  return lines.join('\n');
}

export function renderRuntimeConfig({
  routerUrl,
  runtimeHome,
  serviceDbEncryptionKeyringFile,
}) {
  const lines = [
    `router: ${quoteYamlString(routerUrl)}`,
    `runtime-home: ${quoteYamlString(runtimeHome)}`,
  ];
  if (serviceDbEncryptionKeyringFile !== undefined) {
    lines.push(
      'serviceDb:',
      '  encryption:',
      `    keyringFile: ${quoteYamlString(serviceDbEncryptionKeyringFile)}`,
    );
  }
  lines.push('');
  return lines.join('\n');
}

function requirePositiveSafeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${label} must be a positive safe integer`);
  }
}

function requirePort(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0 || value > 65535) {
    throw new Error(`${label} must be a TCP port`);
  }
}

function validateRewrite(rewrite) {
  if (!Array.isArray(rewrite)) {
    throw new Error('router rewrite must be an array');
  }
  for (let index = 0; index < rewrite.length; index += 1) {
    const item = rewrite[index];
    const label = `router rewrite[${index}]`;
    if (!isRecord(item)) {
      throw new Error(`${label} must be an object`);
    }
    if (typeof item.host !== 'string' || item.host.trim().length === 0) {
      throw new Error(`${label}.host must be a non-empty string`);
    }
    if (typeof item.service !== 'string' || item.service.trim().length === 0) {
      throw new Error(`${label}.service must be a non-empty string`);
    }
    if (item.path !== undefined && (typeof item.path !== 'string' || !item.path.startsWith('/'))) {
      throw new Error(`${label}.path must start with /`);
    }
    if (
      item.version !== undefined
      && (typeof item.version !== 'string' || item.version.trim().length === 0)
    ) {
      throw new Error(`${label}.version must be a non-empty string`);
    }
  }
}

function isRecord(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function renderTelemetryConfig({
  host,
  port,
  path,
  memory,
  emitMemory,
  mongo,
}) {
  const lines = [
    'telemetry:',
    `  host: ${host}`,
    `  port: ${port}`,
    `  path: ${path}`,
  ];
  if (emitMemory) {
    lines.push('', `memory: ${memory ? 'true' : 'false'}`);
  }
  if (mongo !== undefined) {
    lines.push(
      '',
      'mongo:',
      `  url: ${quoteYamlString(mongo.url)}`,
      `  database: ${quoteYamlString(mongo.database)}`,
      ...(mongo.ttlDays ? [`  ttlDays: ${mongo.ttlDays}`] : []),
    );
  }
  lines.push('');
  return lines.join('\n');
}

export function quoteYamlString(value) {
  return JSON.stringify(String(value));
}
