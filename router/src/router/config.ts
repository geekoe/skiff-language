import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';

import { parse } from 'yaml';

import {
  TELEMETRY_PROTOCOL,
  TELEMETRY_TOPICS,
  type FileBackendControlConfig,
  type RuntimeServiceDbActivationPayload,
  type TelemetryControlConfig,
  type TelemetryTopic
} from '../protocol/envelope.js';
import { readRewriteRules, type RouterRewriteRule } from './rewrite.js';

const DEFAULT_TELEMETRY_QUEUE_MAX_EVENTS = 10000;
const DEFAULT_TELEMETRY_BATCH_MAX_EVENTS = 200;
const DEFAULT_TELEMETRY_BATCH_MAX_BYTES = 262144;
const DEFAULT_TELEMETRY_FLUSH_INTERVAL_MS = 1000;
const IDENTITY_CLI_ENV = 'SKIFF_ARTIFACT_IDENTITY_CLI';
const IDENTITY_CLI_BINARY = process.platform === 'win32'
  ? 'skiff-artifact-identity.exe'
  : 'skiff-artifact-identity';

export interface RouterConfig {
  artifactRoots?: string[];
  devReload?: boolean;
  host: string;
  httpBodyLimitBytes?: number;
  httpPort: number;
  identityCliPath?: string;
  manifests: string[];
  profile: string;
  releaseMode?: boolean;
  requestTimeoutMs: number;
  rewrite: RouterRewriteRule[];
  runtimePath: string;
  runtimePort: number;
  fileBackend?: FileBackendControlConfig;
  serviceDb?: RuntimeServiceDbActivationPayload;
  telemetry?: TelemetryControlConfig;
  websocketPath: string;
}

export interface RouterConfigOverrides {
  artifactRoots?: string[];
  devReload?: boolean;
  host?: string;
  httpBodyLimitBytes?: string;
  httpPort?: string;
  identityCliPath?: string;
  manifest?: string;
  profile?: string;
  releaseMode?: boolean;
  requestTimeoutMs?: string;
  runtimePath?: string;
  runtimePort?: string;
  websocketPath?: string;
}

interface RawRouterConfig {
  artifactRoots?: unknown;
  devReload?: unknown;
  host?: unknown;
  hosts?: unknown;
  http?: {
    bodyLimitBytes?: unknown;
    port?: unknown;
  };
  httpPort?: unknown;
  identityCliPath?: unknown;
  fileBackend?: unknown;
  manifest?: unknown;
  manifests?: unknown;
  profile?: unknown;
  releaseMode?: unknown;
  requestTimeoutMs?: unknown;
  rewrite?: unknown;
  runtime?: {
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
  const configDir = dirname(absoluteConfigPath);
  const manifests = readManifests(overrides.manifest ?? raw.manifests ?? raw.manifest);
  const artifactRoots = readOptionalArtifactRoots(
    overrides.artifactRoots ?? raw.artifactRoots,
    configDir
  );
  const devReload = readOptionalBoolean(overrides.devReload ?? raw.devReload, 'devReload');
  const releaseMode = readOptionalBoolean(
    overrides.releaseMode ?? raw.releaseMode,
    'releaseMode'
  );
  const identityCliPath = readIdentityCliPath({
    raw: raw.identityCliPath,
    configDir,
    ...(overrides.identityCliPath !== undefined
      ? { override: overrides.identityCliPath }
      : {}),
    ...(releaseMode !== undefined ? { releaseMode } : {}),
  });
  rejectRemovedValuesConfig(raw.values);
  rejectRemovedArtifactRootConfig(raw);
  const rawProfile = readRequiredProfile(raw.profile, 'profile');
  const profile = readRequiredProfile(overrides.profile ?? rawProfile, 'profile');
  rejectRemovedHosts(raw.hosts);

  const config: RouterConfig = {
    host: readString(overrides.host ?? raw.host, 'host', '127.0.0.1'),
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
    runtimePort: readPort(
      overrides.runtimePort ?? raw.runtimePort ?? raw.runtime?.port,
      'runtime.port',
      4001
    ),
    websocketPath: readPath(
      overrides.websocketPath ?? raw.websocket?.path,
      'websocket.path',
      '/ws'
    )
  };
  if (artifactRoots !== undefined) {
    config.artifactRoots = artifactRoots;
  }
  if (devReload !== undefined) {
    config.devReload = devReload;
  }
  if (releaseMode !== undefined) {
    config.releaseMode = releaseMode;
  }
  const httpBodyLimitBytes = readOptionalPositiveInteger(
    overrides.httpBodyLimitBytes ?? raw.http?.bodyLimitBytes,
    'http.bodyLimitBytes'
  );
  if (httpBodyLimitBytes !== undefined) {
    config.httpBodyLimitBytes = httpBodyLimitBytes;
  }
  if (identityCliPath !== undefined) {
    config.identityCliPath = identityCliPath;
  }
  const fileBackend = readFileBackendConfig(raw.fileBackend, configDir);
  if (fileBackend !== undefined) {
    config.fileBackend = fileBackend;
  }
  const telemetry = readTelemetryConfig(raw.telemetry);
  if (telemetry !== undefined) {
    config.telemetry = telemetry;
  }
  const serviceDb = readServiceDbConfig(raw.serviceDb);
  if (serviceDb !== undefined) {
    config.serviceDb = serviceDb;
  }
  return config;
}

function readServiceDbConfig(value: unknown): RuntimeServiceDbActivationPayload | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
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
    throw new Error('router config artifactRoot is no longer supported; use artifactRoots');
  }
  if (Object.prototype.hasOwnProperty.call(raw, 'artifacts')) {
    throw new Error('router config artifacts is no longer supported; use artifactRoots');
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

function readOptionalArtifactRoots(value: unknown, configDir: string): string[] | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (Array.isArray(value)) {
    if (value.length === 0) {
      throw new Error('router config artifactRoots must be a non-empty string array');
    }
    return value.map((item, index) =>
      resolveConfigPath(
        configDir,
        readString(item, `artifactRoots[${index}]`, String(item))
      )
    );
  }
  throw new Error('router config artifactRoots must be a non-empty string array');
}

function readIdentityCliPath(input: {
  override?: string;
  raw: unknown;
  configDir: string;
  releaseMode?: boolean;
}): string | undefined {
  if (input.override !== undefined) {
    return resolveProcessPath(readString(input.override, 'identityCliPath', input.override));
  }
  if (input.raw !== undefined && input.raw !== null) {
    return resolveConfigPath(
      input.configDir,
      readString(input.raw, 'identityCliPath', String(input.raw))
    );
  }
  const envPath = process.env[IDENTITY_CLI_ENV];
  if (envPath !== undefined && envPath.trim().length > 0) {
    return resolveProcessPath(envPath);
  }
  if (input.releaseMode === true) {
    return undefined;
  }
  return defaultDevIdentityCliPath();
}

function defaultDevIdentityCliPath(): string {
  const devHome =
    process.env.SKIFF_DEV_HOME && process.env.SKIFF_DEV_HOME.trim().length > 0
      ? process.env.SKIFF_DEV_HOME
      : join(process.env.HOME || process.env.USERPROFILE || homedir(), '.skiff', 'dev');
  return join(resolve(devHome), 'bin', IDENTITY_CLI_BINARY);
}

function resolveProcessPath(value: string): string {
  return isAbsolute(value) ? value : resolve(value);
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
    'router config hosts is no longer supported; use top-level rewrite rules'
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
  if (!Number.isInteger(numberValue) || Number(numberValue) <= 0) {
    throw new Error(`router config ${name} must be a positive integer`);
  }
  return Number(numberValue);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
