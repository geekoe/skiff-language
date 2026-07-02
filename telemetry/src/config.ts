import { access, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { parse } from 'yaml';

import {
  InMemoryTelemetryStore,
  MongoTelemetryStore,
  type TelemetryStore
} from './mongoStore.js';

export const DEFAULT_TELEMETRY_HOST = '127.0.0.1';
export const DEFAULT_TELEMETRY_PORT = 4002;
export const DEFAULT_TELEMETRY_PATH = '/telemetry';
export const DEFAULT_TELEMETRY_CONFIG_PATH = 'telemetry.yml';
export const DEFAULT_TELEMETRY_DB = 'skiff_telemetry';

export interface LoadedTelemetryConfig {
  host: string;
  port: number;
  path: string;
  store: TelemetryStore;
}

export interface TelemetryConfigOverrides {
  configPath?: string;
  host?: string;
  port?: string;
  path?: string;
  memory?: boolean;
}

interface RawTelemetryConfig {
  telemetry?: unknown;
  mongo?: unknown;
  memory?: unknown;
  inMemory?: unknown;
}

interface RawTelemetryServerConfig {
  host?: unknown;
  port?: unknown;
  path?: unknown;
}

interface RawMongoConfig {
  url?: unknown;
  database?: unknown;
  db?: unknown;
  ttlDays?: unknown;
}

export async function loadTelemetryConfig(
  overrides: TelemetryConfigOverrides = {},
  env: NodeJS.ProcessEnv = process.env
): Promise<LoadedTelemetryConfig> {
  const raw = await loadRawTelemetryConfig(overrides.configPath ?? env.SKIFF_TELEMETRY_CONFIG);
  const telemetry = optionalRecord(raw.telemetry, 'telemetry') as RawTelemetryServerConfig;
  const mongo = optionalRecord(raw.mongo, 'mongo') as RawMongoConfig;
  const memory =
    overrides.memory ??
    readOptionalBoolean(env.SKIFF_TELEMETRY_IN_MEMORY, 'SKIFF_TELEMETRY_IN_MEMORY') ??
    readOptionalBoolean(raw.memory ?? raw.inMemory, 'memory') ??
    false;
  const host =
    overrides.host ??
    env.SKIFF_TELEMETRY_HOST ??
    readOptionalString(telemetry.host, 'telemetry.host') ??
    DEFAULT_TELEMETRY_HOST;
  const port = readPort(
    overrides.port ?? env.SKIFF_TELEMETRY_PORT ?? telemetry.port,
    'telemetry.port',
    DEFAULT_TELEMETRY_PORT
  );
  const telemetryPath = normalizePath(
    overrides.path ??
      env.SKIFF_TELEMETRY_PATH ??
      readOptionalString(telemetry.path, 'telemetry.path') ??
      DEFAULT_TELEMETRY_PATH
  );

  if (memory) {
    return {
      host,
      port,
      path: telemetryPath,
      store: new InMemoryTelemetryStore()
    };
  }

  const mongoUrl =
    env.SKIFF_TELEMETRY_MONGO_URL ??
    env.MONGO_URL ??
    readOptionalString(mongo.url, 'mongo.url');
  if (!mongoUrl) {
    throw new Error(
      'mongo.url, SKIFF_TELEMETRY_MONGO_URL, or MONGO_URL is required unless memory is true'
    );
  }
  const ttlDays = readOptionalPositiveNumber(
    env.SKIFF_TELEMETRY_TTL_DAYS ?? mongo.ttlDays,
    'mongo.ttlDays'
  );

  return {
    host,
    port,
    path: telemetryPath,
    store: new MongoTelemetryStore({
      mongoUrl,
      databaseName:
        env.SKIFF_TELEMETRY_DB ??
        readOptionalString(mongo.database ?? mongo.db, 'mongo.database') ??
        DEFAULT_TELEMETRY_DB,
      ...(ttlDays !== undefined ? { ttlDays } : {})
    })
  };
}

async function loadRawTelemetryConfig(configPath: string | undefined): Promise<RawTelemetryConfig> {
  if (configPath !== undefined) {
    return readTelemetryConfigFile(configPath, true);
  }
  if (await fileExists(DEFAULT_TELEMETRY_CONFIG_PATH)) {
    return readTelemetryConfigFile(DEFAULT_TELEMETRY_CONFIG_PATH, true);
  }
  return {};
}

async function readTelemetryConfigFile(
  configPath: string,
  required: boolean
): Promise<RawTelemetryConfig> {
  const absolutePath = resolve(configPath);
  let text: string;
  try {
    text = await readFile(absolutePath, 'utf8');
  } catch (error) {
    if (!required) {
      return {};
    }
    throw new Error(`failed to read telemetry config ${absolutePath}`, { cause: error });
  }
  const parsed = parse(text) as unknown;
  if (parsed === undefined || parsed === null) {
    return {};
  }
  if (!isRecord(parsed)) {
    throw new Error(`telemetry config ${absolutePath} must be a YAML object`);
  }
  return parsed as RawTelemetryConfig;
}

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

function optionalRecord(value: unknown, name: string): Record<string, unknown> {
  if (value === undefined || value === null) {
    return {};
  }
  if (!isRecord(value)) {
    throw new Error(`telemetry config ${name} must be an object`);
  }
  return value;
}

function readOptionalString(value: unknown, name: string): string | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`telemetry config ${name} must be a non-empty string`);
  }
  return value.trim();
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
    if (normalized === 'true' || normalized === '1') {
      return true;
    }
    if (normalized === 'false' || normalized === '0') {
      return false;
    }
  }
  throw new Error(`telemetry config ${name} must be a boolean`);
}

function readPort(value: unknown, name: string, fallback: number): number {
  if (value === undefined || value === null) {
    return fallback;
  }
  const parsed = typeof value === 'number' ? value : Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0 || parsed > 65535) {
    throw new Error(`telemetry config ${name} must be a TCP port`);
  }
  return parsed;
}

function readOptionalPositiveNumber(value: unknown, name: string): number | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  const parsed = typeof value === 'number' ? value : Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`telemetry config ${name} must be a positive number`);
  }
  return parsed;
}

function normalizePath(value: string): string {
  return value.startsWith('/') ? value : `/${value}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
