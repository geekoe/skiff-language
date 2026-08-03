import { dirname, join } from 'node:path';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';

const BUILD_STATUS_SCHEMA_VERSION = 1;
const BUILD_STATUS_STATES = new Set(['ok', 'failed', 'building']);
const MAX_ERROR_SUMMARY_LENGTH = 240;

export function defaultBuildStatusPath(configPath) {
  return join(dirname(configPath), 'last-build.json');
}

export async function writeBuildStatus({
  path,
  state,
  updatedAt,
  nextRetryAt = null,
  error = null,
  attempt = null,
}) {
  if (!BUILD_STATUS_STATES.has(state)) {
    throw new Error(`invalid dev sync build status state ${state}`);
  }
  const payload = {
    schemaVersion: BUILD_STATUS_SCHEMA_VERSION,
    state,
    updatedAt,
    nextRetryAt,
    error,
    attempt,
  };
  await mkdir(dirname(path), { recursive: true });
  const temporaryPath = `${path}.tmp-${process.pid}-${Date.now()}`;
  await writeFile(temporaryPath, `${JSON.stringify(payload)}\n`, 'utf8');
  await rename(temporaryPath, path);
}

export async function readBuildStatus(path) {
  let text;
  try {
    text = await readFile(path, 'utf8');
  } catch {
    return null;
  }
  try {
    const parsed = JSON.parse(text);
    if (
      parsed === null
      || typeof parsed !== 'object'
      || parsed.schemaVersion !== BUILD_STATUS_SCHEMA_VERSION
      || !BUILD_STATUS_STATES.has(parsed.state)
    ) {
      return null;
    }
    return parsed;
  } catch {
    return null;
  }
}

export function summarizeBuildError(error) {
  const message = error instanceof Error
    ? (error.message ?? String(error))
    : String(error);
  const firstLine = message.split(/\r?\n/, 1)[0].trim();
  return firstLine.length > MAX_ERROR_SUMMARY_LENGTH
    ? `${firstLine.slice(0, MAX_ERROR_SUMMARY_LENGTH - 3)}...`
    : firstLine;
}

export function formatBuildStatusSuffix(status, now = Date.now()) {
  if (status === null) {
    return '';
  }
  if (status.state === 'ok') {
    return ' build=ok';
  }
  if (status.state === 'failed') {
    const retryInMs = status.nextRetryAt
      ? Math.max(0, new Date(status.nextRetryAt).getTime() - now)
      : null;
    const retry = retryInMs === null
      ? ''
      : ` retryIn=${Math.ceil(retryInMs / 1000)}s`;
    return ` build=failed${retry}`;
  }
  return ' build=building';
}
