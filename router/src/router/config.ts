import { readFile } from 'node:fs/promises';
import { dirname, isAbsolute, resolve } from 'node:path';

import { parse } from 'yaml';

import {
  TELEMETRY_PROTOCOL,
  TELEMETRY_TOPICS,
  type FileBackendControlConfig,
  type RouterBootstrapActivation,
  type RouterBootstrapEnvelope,
  type RuntimeServiceDbConfigInput,
  type TelemetryControlConfig,
  type TelemetryTopic
} from '../protocol/envelope.js';
import { DEFAULT_ACTIVATION_PREPARE_TIMEOUT_MS } from './activationTimeout.js';
import { readRewriteRules, type RouterRewriteRule } from './rewrite.js';

const DEFAULT_TELEMETRY_QUEUE_MAX_EVENTS = 10000;
const DEFAULT_TELEMETRY_BATCH_MAX_EVENTS = 200;
const DEFAULT_TELEMETRY_BATCH_MAX_BYTES = 262144;
const DEFAULT_TELEMETRY_FLUSH_INTERVAL_MS = 1000;
export interface RouterConfig {
  activationPrepareTimeoutMs: number;
  artifactsPath: string;
  devReload?: boolean;
  environment?: string;
  host: string;
  httpMaxRequestBytes: number;
  httpMaxResponseBytes: number;
  httpPort: number;
  manifests: string[];
  profile: string;
  releaseMode?: boolean;
  requestTimeoutMs: number;
  rewrite: RouterRewriteRule[];
  runtimePath: string;
  runtimePort: number;
  runtimeMaxConcurrency: number;
  fileBackend?: FileBackendControlConfig;
  serviceDb: RuntimeServiceDbConfigInput;
  telemetry?: TelemetryControlConfig;
  websocketPath: string;
}

export interface RouterConfigOverrides {
  activationPrepareTimeoutMs?: string;
  artifactsPath?: string;
  devReload?: boolean;
  environment?: string;
  host?: string;
  httpMaxRequestBytes?: string;
  httpMaxResponseBytes?: string;
  httpPort?: string;
  manifest?: string;
  profile?: string;
  releaseMode?: boolean;
  requestTimeoutMs?: string;
  runtimePath?: string;
  runtimePort?: string;
  websocketPath?: string;
}

export function runtimeBootstrapForRouterConfig(
  config: RouterConfig,
  activation: RouterBootstrapActivation
): Omit<RouterBootstrapEnvelope, 'type'> {
  return {
    artifactsPath: config.artifactsPath,
    serviceDb: config.serviceDb,
    http: { maxResponseBytes: config.httpMaxResponseBytes },
    activation
  };
}

interface RawRouterConfig {
  activation?: unknown;
  artifactsPath?: unknown;
  artifactRoots?: unknown;
  devReload?: unknown;
  environment?: unknown;
  host?: unknown;
  hosts?: unknown;
  http?: {
    bodyLimitBytes?: unknown;
    maxRequestBytes?: unknown;
    maxResponseBytes?: unknown;
    port?: unknown;
  };
  httpPort?: unknown;
  fileBackend?: unknown;
  manifest?: unknown;
  manifests?: unknown;
  profile?: unknown;
  releaseMode?: unknown;
  requestTimeoutMs?: unknown;
  rewrite?: unknown;
  runtime?: {
    maxConcurrency?: unknown;
    path?: unknown;
    port?: unknown;
  };
  runtimePath?: unknown;
  runtimePort?: unknown;
  serviceDb?: unknown;
  telemetry?: unknown;
  values?: unknown;
  websocket?: {
    path?: unknown;
  };
}

export async function loadRouterConfig(
  configPath: string,
  overrides: RouterConfigOverrides = {}
): Promise<RouterConfig> {
  const absoluteConfigPath = resolve(configPath);
  let text: string;
  try {
    text = await readFile(absoluteConfigPath, 'utf8');
  } catch (error) {
    throw new Error(
      `failed to read router config ${absoluteConfigPath}; copy router.example.yml to router.yml first`,
      { cause: error }
    );
  }

  const parsed = parse(text) as unknown;
  if (!isRecord(parsed)) {
    throw new Error(`router config ${absoluteConfigPath} must be a YAML object`);
  }

  const raw = parsed as RawRouterConfig;
  if (
    raw.http !== undefined &&
    Object.prototype.hasOwnProperty.call(raw.http, 'bodyLimitBytes')
  ) {
    throw new Error('router config http.bodyLimitBytes is no longer supported');
  }
  const configDir = dirname(absoluteConfigPath);
  const manifests = readManifests(overrides.manifest ?? raw.manifests ?? raw.manifest);
  rejectRemovedArtifactRootConfig(raw);
  const artifactsPath = resolve(
    configDir,
    readRequiredString(overrides.artifactsPath ?? raw.artifactsPath, 'artifactsPath')
  );
  const devReload = readOptionalBoolean(overrides.devReload ?? raw.devReload, 'devReload');
  const releaseMode = readOptionalBoolean(
    overrides.releaseMode ?? raw.releaseMode,
    'releaseMode'
  );
  rejectRemovedValuesConfig(raw.values);
  const rawProfile = readRequiredProfile(raw.profile, 'profile');
  const profile = readRequiredProfile(overrides.profile ?? rawProfile, 'profile');
  rejectRemovedHosts(raw.hosts);

  const config: RouterConfig = {
    activationPrepareTimeoutMs: readActivationPrepareTimeout(
      raw.activation,
      overrides.activationPrepareTimeoutMs
    ),
    artifactsPath,
    host: readString(overrides.host ?? raw.host, 'host', '127.0.0.1'),
    httpMaxRequestBytes: readRequiredPositiveInteger(
      overrides.httpMaxRequestBytes ?? raw.http?.maxRequestBytes,
      'http.maxRequestBytes'
    ),
    httpMaxResponseBytes: readRequiredPositiveInteger(
      overrides.httpMaxResponseBytes ?? raw.http?.maxResponseBytes,
      'http.maxResponseBytes'
    ),
    httpPort: readPort(overrides.httpPort ?? raw.httpPort ?? raw.http?.port, 'http.port', 4000),
    manifests: manifests.map((manifest) => resolveConfigPath(configDir, manifest)),
    profile,
    requestTimeoutMs: readPositiveInteger(
      overrides.requestTimeoutMs ?? raw.requestTimeoutMs,
      'requestTimeoutMs',
      20000
    ),
    rewrite: readRewriteRules(raw.rewrite),
    runtimePath: readPath(
      overrides.runtimePath ?? raw.runtimePath ?? raw.runtime?.path,
      'runtime.path',
      '/runtime'
    ),
    runtimeMaxConcurrency: readRequiredPositiveConfigInteger(
      raw.runtime?.maxConcurrency,
      'runtime.maxConcurrency'
    ),
    runtimePort: readPort(
      overrides.runtimePort ?? raw.runtimePort ?? raw.runtime?.port,
      'runtime.port',
      4001
    ),
    serviceDb: readServiceDbConfig(raw.serviceDb),
    websocketPath: readPath(
      overrides.websocketPath ?? raw.websocket?.path,
      'websocket.path',
      '/ws'
    )
  };
  const environment = readOptionalNonEmptyString(
    overrides.environment ?? raw.environment,
    'environment'
  );
  if (environment !== undefined) {
    if (!/^[A-Za-z0-9._-]{1,200}$/.test(environment) || environment === '.' || environment === '..') {
      throw new Error('router config environment is invalid');
    }
    config.environment = environment;
  }
  if (devReload !== undefined) {
    config.devReload = devReload;
  }
  if (releaseMode !== undefined) {
    config.releaseMode = releaseMode;
  }
  const fileBackend = readFileBackendConfig(raw.fileBackend, configDir);
  if (fileBackend !== undefined) {
    config.fileBackend = fileBackend;
  }
  const telemetry = readTelemetryConfig(raw.telemetry);
  if (telemetry !== undefined) {
    config.telemetry = telemetry;
  }
  return config;
}

function readServiceDbConfig(value: unknown): RuntimeServiceDbConfigInput {
  if (!isRecord(value)) {
    throw new Error('router config serviceDb must be an object');
  }
  if (Object.prototype.hasOwnProperty.call(value, 'storageNamespace')) {
    throw new Error('router config serviceDb.storageNamespace is no longer supported');
  }
  return {
    mongoUrl: readRequiredString(value.mongoUrl, 'serviceDb.mongoUrl')
  };
}

function readFileBackendConfig(
  value: unknown,
  configDir: string
): FileBackendControlConfig | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (!isRecord(value)) {
    throw new Error('router config fileBackend must be an object');
  }
  const local = readFileBackendLocalConfig(value.local, configDir);
  const oss = readFileBackendOssConfig(value.oss);
  if (local === undefined && oss === undefined) {
    throw new Error('router config fileBackend must configure local or oss');
  }
  return {
    ...(local !== undefined ? { local } : {}),
    ...(oss !== undefined ? { oss } : {})
  };
}

function readFileBackendLocalConfig(
  value: unknown,
  configDir: string
): FileBackendControlConfig['local'] | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (!isRecord(value)) {
    throw new Error('router config fileBackend.local must be an object');
  }
  return {
    root: resolveConfigPath(
      configDir,
      readRequiredString(value.root, 'fileBackend.local.root')
    )
  };
}

function readFileBackendOssConfig(
  value: unknown
): FileBackendControlConfig['oss'] | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (!isRecord(value)) {
    throw new Error('router config fileBackend.oss must be an object');
  }

  const accessKeyId = readOptionalNonEmptyString(
    value.accessKeyId,
    'fileBackend.oss.accessKeyId'
  );
  const accessKeySecret = readOptionalNonEmptyString(
    value.accessKeySecret,
    'fileBackend.oss.accessKeySecret'
  );
  const accessKeyIdEnv = readOptionalNonEmptyString(
    value.accessKeyIdEnv,
    'fileBackend.oss.accessKeyIdEnv'
  );
  const accessKeySecretEnv = readOptionalNonEmptyString(
    value.accessKeySecretEnv,
    'fileBackend.oss.accessKeySecretEnv'
  );
  const region = readOptionalNonEmptyString(value.region, 'fileBackend.oss.region');

  if (accessKeyId === undefined && accessKeyIdEnv === undefined) {
    throw new Error(
      'router config fileBackend.oss requires accessKeyIdEnv or accessKeyId'
    );
  }
  if (accessKeySecret === undefined && accessKeySecretEnv === undefined) {
    throw new Error(
      'router config fileBackend.oss requires accessKeySecretEnv or accessKeySecret'
    );
  }

  return {
    endpoint: readRequiredString(value.endpoint, 'fileBackend.oss.endpoint'),
    bucket: readRequiredString(value.bucket, 'fileBackend.oss.bucket'),
    ...(region !== undefined ? { region } : {}),
    ...(accessKeyId !== undefined ? { accessKeyId } : {}),
    ...(accessKeySecret !== undefined ? { accessKeySecret } : {}),
    ...(accessKeyIdEnv !== undefined ? { accessKeyIdEnv } : {}),
    ...(accessKeySecretEnv !== undefined ? { accessKeySecretEnv } : {})
  };
}

function rejectRemovedArtifactRootConfig(raw: RawRouterConfig): void {
  if (Object.prototype.hasOwnProperty.call(raw, 'artifactRoot')) {
    throw new Error('router config artifactRoot is no longer supported; use artifactsPath');
  }
  if (Object.prototype.hasOwnProperty.call(raw, 'artifactRoots')) {
    throw new Error('router config artifactRoots is no longer supported; use artifactsPath');
  }
  if (Object.prototype.hasOwnProperty.call(raw, 'artifacts')) {
    throw new Error('router config artifacts is no longer supported; use artifactsPath');
  }
}

function readRequiredString(value: unknown, name: string): string {
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`router config ${name} must be a non-empty string`);
  }
  return value.trim();
}

function readOptionalNonEmptyString(value: unknown, name: string): string | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  return readRequiredString(value, name);
}

function readManifests(value: unknown): string[] {
  if (value === undefined || value === null) {
    return ['fixtures/hello/manifest.json'];
  }
  if (typeof value === 'string') {
    return [readString(value, 'manifest', 'fixtures/hello/manifest.json')];
  }
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error('router config manifests must be a non-empty string array');
  }
  return value.map((item, index) => {
    if (typeof item !== 'string' || item.trim().length === 0) {
      throw new Error(`router config manifests[${index}] must be a non-empty string`);
    }
    return item.trim();
  });
}

function resolveConfigPath(configDir: string, value: string): string {
  return isAbsolute(value) ? value : resolve(configDir, value);
}

function readRequiredProfile(value: unknown, name: string): string {
  if (value === undefined || value === null) {
    throw new Error(`router config ${name} is required`);
  }
  const profile = readString(value, name, String(value));
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(profile)) {
    throw new Error(
      `router config ${name} must match [A-Za-z_][A-Za-z0-9_]* so it can be used in config.<profile>.yml`
    );
  }
  return profile;
}

function readOptionalBoolean(value: unknown, name: string): boolean | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (normalized === 'true') {
      return true;
    }
    if (normalized === 'false') {
      return false;
    }
  }
  throw new Error(`router config ${name} must be a boolean`);
}

function readTelemetryConfig(value: unknown): TelemetryControlConfig | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (!isRecord(value)) {
    throw new Error('router config telemetry must be an object');
  }

  const enabled = readOptionalBoolean(value.enabled, 'telemetry.enabled') ?? true;
  if (!enabled) {
    return undefined;
  }

  if (value.endpoint === undefined || value.endpoint === null) {
    return undefined;
  }

  return {
    endpoint: readString(value.endpoint, 'telemetry.endpoint', String(value.endpoint)),
    protocol: readTelemetryProtocol(value.protocol),
    topics: readTelemetryTopics(value.topics),
    queueMaxEvents: readPositiveInteger(
      value.queueMaxEvents,
      'telemetry.queueMaxEvents',
      DEFAULT_TELEMETRY_QUEUE_MAX_EVENTS
    ),
    batchMaxEvents: readPositiveInteger(
      value.batchMaxEvents,
      'telemetry.batchMaxEvents',
      DEFAULT_TELEMETRY_BATCH_MAX_EVENTS
    ),
    batchMaxBytes: readPositiveInteger(
      value.batchMaxBytes,
      'telemetry.batchMaxBytes',
      DEFAULT_TELEMETRY_BATCH_MAX_BYTES
    ),
    flushIntervalMs: readPositiveInteger(
      value.flushIntervalMs,
      'telemetry.flushIntervalMs',
      DEFAULT_TELEMETRY_FLUSH_INTERVAL_MS
    ),
    enabled
  };
}

function readTelemetryProtocol(value: unknown): typeof TELEMETRY_PROTOCOL {
  if (value === undefined || value === null) {
    return TELEMETRY_PROTOCOL;
  }
  if (value !== TELEMETRY_PROTOCOL) {
    throw new Error(`router config telemetry.protocol must be ${TELEMETRY_PROTOCOL}`);
  }
  return TELEMETRY_PROTOCOL;
}

function readTelemetryTopics(value: unknown): TelemetryTopic[] {
  if (value === undefined || value === null) {
    return [...TELEMETRY_TOPICS];
  }
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error('router config telemetry.topics must be a non-empty array');
  }
  const topics: TelemetryTopic[] = [];
  const seen = new Set<TelemetryTopic>();
  for (let index = 0; index < value.length; index += 1) {
    const topic = value[index];
    if (typeof topic !== 'string' || !isTelemetryTopic(topic)) {
      throw new Error(
        `router config telemetry.topics[${index}] must be one of ${TELEMETRY_TOPICS.join(', ')}`
      );
    }
    if (seen.has(topic)) {
      throw new Error('router config telemetry.topics must not contain duplicates');
    }
    seen.add(topic);
    topics.push(topic);
  }
  return topics;
}

function isTelemetryTopic(value: string): value is TelemetryTopic {
  return (TELEMETRY_TOPICS as readonly string[]).includes(value);
}

function rejectRemovedHosts(value: unknown): void {
  if (value === undefined || value === null) {
    return;
  }
  throw new Error(
    'router config hosts is no longer supported; declare RuntimeAssembly globalIngress Hosts'
  );
}

function rejectRemovedValuesConfig(value: unknown): void {
  if (value === undefined || value === null) {
    return;
  }
  if (isRecord(value) && Object.prototype.hasOwnProperty.call(value, 'profile')) {
    throw new Error(
      'router config values.profile is no longer supported; set top-level profile instead'
    );
  }
  throw new Error(
    'router config values is no longer supported; set top-level profile and keep runtime config in config*.yml'
  );
}

function readString(value: unknown, name: string, fallback: string): string {
  if (value === undefined || value === null) {
    return fallback;
  }
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`router config ${name} must be a non-empty string`);
  }
  return value.trim();
}

function readPath(value: unknown, name: string, fallback: string): string {
  const path = readString(value, name, fallback);
  if (!path.startsWith('/')) {
    throw new Error(`router config ${name} must start with /`);
  }
  return path;
}

function readPort(value: unknown, name: string, fallback: number): number {
  const port = readPositiveInteger(value, name, fallback);
  if (port > 65535) {
    throw new Error(`router config ${name} must be <= 65535`);
  }
  return port;
}

function readPositiveInteger(value: unknown, name: string, fallback: number): number {
  if (value === undefined || value === null) {
    return fallback;
  }
  return readRequiredPositiveInteger(value, name);
}

function readOptionalPositiveInteger(value: unknown, name: string): number | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  return readRequiredPositiveInteger(value, name);
}

function readRequiredPositiveInteger(value: unknown, name: string): number {
  const numberValue = typeof value === 'string' ? Number(value) : value;
  if (!Number.isSafeInteger(numberValue) || Number(numberValue) <= 0) {
    throw new Error(`router config ${name} must be a positive integer`);
  }
  return Number(numberValue);
}

function readRequiredPositiveConfigInteger(value: unknown, name: string): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`router config ${name} must be a positive integer`);
  }
  return value;
}

function readActivationPrepareTimeout(
  value: unknown,
  override: string | undefined
): number {
  if (override !== undefined) {
    return readRequiredPositiveInteger(
      override,
      'activation.prepareTimeoutMs'
    );
  }
  if (value === undefined || value === null) {
    return DEFAULT_ACTIVATION_PREPARE_TIMEOUT_MS;
  }
  if (!isRecord(value)) {
    throw new Error(
      'router config activation.prepareTimeoutMs must be a positive integer'
    );
  }
  const prepareTimeoutMs = value.prepareTimeoutMs;
  if (prepareTimeoutMs === undefined || prepareTimeoutMs === null) {
    return DEFAULT_ACTIVATION_PREPARE_TIMEOUT_MS;
  }
  if (
    typeof prepareTimeoutMs !== 'number' ||
    !Number.isSafeInteger(prepareTimeoutMs) ||
    prepareTimeoutMs <= 0
  ) {
    throw new Error(
      'router config activation.prepareTimeoutMs must be a positive integer'
    );
  }
  return prepareTimeoutMs;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
