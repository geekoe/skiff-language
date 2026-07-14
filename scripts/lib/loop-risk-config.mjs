import { access, readFile, stat } from 'node:fs/promises';
import { constants as fsConstants } from 'node:fs';
import { isAbsolute } from 'node:path';

import { redactLoopRiskUrl } from './loop-risk-cli.mjs';

export const LOOP_RISK_CONFIG_PROFILES = Object.freeze({
  HEALTH: 'health',
  STRESS: 'stress',
});

export async function loadLoopRiskConfig(configPath, {
  profile = LOOP_RISK_CONFIG_PROFILES.HEALTH,
  checkLogFiles = profile === LOOP_RISK_CONFIG_PROFILES.STRESS,
} = {}) {
  let raw;
  try {
    raw = await readFile(configPath, 'utf8');
  } catch (error) {
    throw new Error(`loop-risk config must be a readable file: ${configPath}`, { cause: error });
  }

  let value;
  try {
    value = JSON.parse(raw);
  } catch (error) {
    throw new Error(`loop-risk config must contain valid JSON: ${configPath}`, { cause: error });
  }
  const config = parseLoopRiskConfig(value, { profile });
  if (checkLogFiles && config.stress !== undefined) {
    await assertReadableRuntimeLogs(config.stress.runtimeLogs);
  }
  return config;
}

export function parseLoopRiskConfig(value, {
  profile = LOOP_RISK_CONFIG_PROFILES.HEALTH,
} = {}) {
  if (!Object.values(LOOP_RISK_CONFIG_PROFILES).includes(profile)) {
    throw new Error(`unknown loop-risk config profile ${profile}`);
  }
  assertPlainObject(value, 'loop-risk config');
  assertExactKeys(value, ['healthUrl', 'runtimeIds', 'stress'], 'loop-risk config', {
    optional: ['stress'],
  });

  const healthUrl = parseHealthUrl(value.healthUrl);
  const runtimeIds = parseUniqueStrings(value.runtimeIds, 'loop-risk config runtimeIds');
  let stress;
  if (value.stress !== undefined) {
    stress = parseStressConfig(value.stress);
  } else if (profile === LOOP_RISK_CONFIG_PROFILES.STRESS) {
    throw new Error('loop-risk stress config requires stress');
  }
  return deepFreeze({ healthUrl, runtimeIds, ...(stress === undefined ? {} : { stress }) });
}

export async function assertReadableRuntimeLogs(runtimeLogs) {
  const failures = [];
  for (const path of runtimeLogs) {
    try {
      const metadata = await stat(path);
      if (!metadata.isFile()) {
        failures.push(`runtime log must be a file: ${path}`);
        continue;
      }
      await access(path, fsConstants.R_OK);
    } catch {
      failures.push(`runtime log must be an existing readable file: ${path}`);
    }
  }
  if (failures.length > 0) {
    throw new Error(failures.join('; '));
  }
}

function parseStressConfig(value) {
  assertPlainObject(value, 'loop-risk config stress');
  assertExactKeys(
    value,
    ['wsUrl', 'runtimePids', 'runtimeLogs'],
    'loop-risk config stress',
  );
  return {
    wsUrl: parseWebSocketUrl(value.wsUrl),
    runtimePids: parseRuntimePids(value.runtimePids),
    runtimeLogs: parseRuntimeLogs(value.runtimeLogs),
  };
}

function parseHealthUrl(rawUrl) {
  const parsed = parseUrl(rawUrl, 'healthUrl');
  if (
    !['http:', 'https:'].includes(parsed.protocol)
    || parsed.username
    || parsed.password
    || parsed.pathname !== '/__router/health'
    || parsed.search !== '?detail=loop-risk'
    || parsed.hash
  ) {
    throw new Error(
      `loop-risk config healthUrl must target /__router/health?detail=loop-risk: ${redactLoopRiskUrl(rawUrl)}`,
    );
  }
  return rawUrl;
}

function parseWebSocketUrl(rawUrl) {
  const parsed = parseUrl(rawUrl, 'stress.wsUrl');
  if (!['ws:', 'wss:'].includes(parsed.protocol) || parsed.hash) {
    throw new Error(
      `loop-risk config stress.wsUrl must be ws:// or wss:// without a fragment: ${redactLoopRiskUrl(rawUrl)}`,
    );
  }
  return rawUrl;
}

function parseUrl(rawUrl, field) {
  if (typeof rawUrl !== 'string' || rawUrl.trim().length === 0) {
    throw new Error(`loop-risk config ${field} must be a non-empty URL string`);
  }
  try {
    const parsed = new URL(rawUrl);
    if (!parsed.host) {
      throw new Error('missing host');
    }
    return parsed;
  } catch {
    throw new Error(
      `loop-risk config ${field} is invalid: ${redactLoopRiskUrl(rawUrl)}`,
    );
  }
}

function parseRuntimePids(value) {
  if (
    !Array.isArray(value)
    || value.length === 0
    || !value.every((pid) => Number.isInteger(pid) && pid > 0)
    || new Set(value).size !== value.length
  ) {
    throw new Error('loop-risk config stress.runtimePids must be unique positive integers');
  }
  return [...value];
}

function parseRuntimeLogs(value) {
  const paths = parseUniqueStrings(value, 'loop-risk config stress.runtimeLogs');
  if (!paths.every(isAbsolute)) {
    throw new Error('loop-risk config stress.runtimeLogs must contain absolute paths');
  }
  return paths;
}

function parseUniqueStrings(value, field) {
  if (
    !Array.isArray(value)
    || value.length === 0
    || !value.every((entry) => typeof entry === 'string' && entry.trim().length > 0)
    || new Set(value).size !== value.length
  ) {
    throw new Error(`${field} must be a non-empty array of unique non-empty strings`);
  }
  return [...value];
}

function assertPlainObject(value, field) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${field} must be an object`);
  }
}

function assertExactKeys(value, allowed, field, { optional = [] } = {}) {
  const actual = Object.keys(value);
  const unexpected = actual.filter((key) => !allowed.includes(key));
  const required = allowed.filter((key) => !optional.includes(key));
  const missing = required.filter((key) => !Object.hasOwn(value, key));
  if (missing.length > 0 || unexpected.length > 0) {
    throw new Error([
      missing.length > 0 ? `${field} missing field(s): ${missing.join(', ')}` : '',
      unexpected.length > 0 ? `${field} unknown field(s): ${unexpected.join(', ')}` : '',
    ].filter(Boolean).join('; '));
  }
}

function deepFreeze(value) {
  for (const child of Object.values(value)) {
    if (child && typeof child === 'object') {
      deepFreeze(child);
    }
  }
  return Object.freeze(value);
}
