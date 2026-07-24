import { isAbsolute } from 'node:path';

export function renderRouterConfig({
  profile,
  host,
  environment,
  artifactsPath,
  ecosystemStoreCliPath,
  identityCliPath,
  devReload,
  releaseMode,
  requestTimeoutMs = 20000,
  httpPort,
  runtimePort,
  runtimePath = '/runtime',
  serviceDbMongoUrl,
  telemetryEndpoint,
  rewrite = [],
}) {
  if (typeof environment !== 'string' || environment.length === 0) {
    throw new Error('router environment is required');
  }
  if (typeof ecosystemStoreCliPath !== 'string' || ecosystemStoreCliPath.trim().length === 0) {
    throw new Error('router ecosystemStoreCliPath is required');
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
  const lines = [
    `profile: ${profile}`,
    `host: ${host}`,
    `environment: ${quoteYamlString(environment)}`,
    `artifactsPath: ${quoteYamlString(artifactsPath)}`,
    `ecosystemStoreCliPath: ${quoteYamlString(ecosystemStoreCliPath)}`,
    `identityCliPath: ${quoteYamlString(identityCliPath)}`,
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
    '',
    'runtime:',
    `  port: ${runtimePort}`,
    `  path: ${runtimePath}`,
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
  environment,
  serviceDbEncryptionKeyringFile,
  httpResponseMaxBytes,
}) {
  if (typeof environment !== 'string' || environment.trim().length === 0) {
    throw new Error('runtime environment is required');
  }
  const lines = [
    `router: ${quoteYamlString(routerUrl)}`,
    `runtime-home: ${quoteYamlString(runtimeHome)}`,
    `environment: ${quoteYamlString(environment)}`,
  ];
  if (serviceDbEncryptionKeyringFile !== undefined) {
    lines.push(
      'serviceDb:',
      '  encryption:',
      `    keyringFile: ${quoteYamlString(serviceDbEncryptionKeyringFile)}`,
    );
  }
  if (httpResponseMaxBytes !== undefined) {
    lines.push(
      'http:',
      '  response:',
      `    maxBytes: ${httpResponseMaxBytes}`,
    );
  }
  lines.push('');
  return lines.join('\n');
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
