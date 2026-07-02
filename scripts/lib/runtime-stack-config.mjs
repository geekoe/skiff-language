export function renderRouterConfig({
  profile,
  host,
  artifactRoots,
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
  const lines = [
    `profile: ${profile}`,
    `host: ${host}`,
    'artifactRoots:',
    ...artifactRoots.map((artifactRoot) => `  - ${quoteYamlString(artifactRoot)}`),
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
  if (serviceDbMongoUrl !== undefined) {
    lines.push(
      '',
      'serviceDb:',
      `  mongoUrl: ${quoteYamlString(serviceDbMongoUrl)}`,
    );
  }
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
  artifactRoots,
  httpResponseMaxBytes,
}) {
  const lines = [
    `router: ${quoteYamlString(routerUrl)}`,
    `runtime-home: ${quoteYamlString(runtimeHome)}`,
  ];
  if ((artifactRoots?.length ?? 0) > 0) {
    lines.push(
      'artifactRoots:',
      ...artifactRoots.map((artifactRoot) => `  - ${quoteYamlString(artifactRoot)}`),
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
