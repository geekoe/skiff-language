import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import { sanitizeFixtureCargoDiagnostic } from './package-service-ecosystem-smoke-diagnostic.mjs';

export const ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY = 'isolatedRuntimeLogEvidence';
export const ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES = 4096;
export const ISOLATED_RUNTIME_LOG_EVIDENCE_SCHEMA_VERSION =
  'skiff-isolated-runtime-failure-log-evidence-v1';

const LOG_FILES = Object.freeze([
  ['router', 'stdout', 'router.log'],
  ['router', 'stderr', 'router.err.log'],
  ['runtime', 'stdout', 'runtime.log'],
  ['runtime', 'stderr', 'runtime.err.log'],
]);

export async function retainIsolatedRuntimeLogEvidence(error, tempRoot, {
  read = readFile,
} = {}) {
  if (!isObject(error) || Object.hasOwn(error, ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY)) {
    return error;
  }
  const evidence = await captureIsolatedRuntimeLogEvidence(tempRoot, { read });
  Object.defineProperty(error, ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY, {
    value: evidence,
    enumerable: true,
    writable: false,
    configurable: false,
  });
  return error;
}

export async function captureIsolatedRuntimeLogEvidence(tempRoot, {
  read = readFile,
} = {}) {
  read ??= readFile;
  const logs = [];
  for (const [component, stream, filename] of LOG_FILES) {
    const path = join(tempRoot, 'instance', 'logs', filename);
    logs.push(await captureLog(component, stream, path, read));
  }
  return Object.freeze({
    schemaVersion: ISOLATED_RUNTIME_LOG_EVIDENCE_SCHEMA_VERSION,
    logs: Object.freeze(logs),
  });
}

export function renderIsolatedRuntimeLogEvidence(error) {
  const evidence = error?.[ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY];
  if (
    evidence?.schemaVersion !== ISOLATED_RUNTIME_LOG_EVIDENCE_SCHEMA_VERSION
    || !Array.isArray(evidence.logs)
  ) {
    return '';
  }
  const rendered = evidence.logs.flatMap((log) => {
    if (
      typeof log?.component !== 'string'
      || typeof log?.stream !== 'string'
      || typeof log?.sanitizedTail !== 'string'
      || log.sanitizedTail.trim().length === 0
    ) {
      return [];
    }
    const suffix = log.truncated === true ? ' (tail, truncated)' : '';
    return [`[isolated ${log.component} ${log.stream}${suffix}]\n${log.sanitizedTail.trimEnd()}`];
  });
  return rendered.join('\n');
}

async function captureLog(component, stream, path, read) {
  let contents;
  try {
    contents = await read(path);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return Object.freeze({
        component,
        stream,
        missing: true,
        bytes: 0,
        sha256: sha256(Buffer.alloc(0)),
        truncated: false,
        sanitizedTail: '',
      });
    }
    return Object.freeze({
      component,
      stream,
      missing: false,
      bytes: null,
      sha256: null,
      truncated: false,
      sanitizedTail: '',
      readError: sanitizeFixtureCargoDiagnostic(error?.message || String(error)),
    });
  }
  const raw = Buffer.isBuffer(contents) ? contents : Buffer.from(contents);
  const sanitized = Buffer.from(sanitizeFixtureCargoDiagnostic(raw.toString('utf8')));
  const tail = utf8Tail(sanitized, ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES);
  return Object.freeze({
    component,
    stream,
    missing: false,
    bytes: raw.length,
    sha256: sha256(raw),
    truncated: raw.length > ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES,
    sanitizedTail: tail.toString('utf8'),
  });
}

function utf8Tail(raw, maximumBytes) {
  let start = Math.max(0, raw.length - maximumBytes);
  while (start < raw.length && (raw[start] & 0xc0) === 0x80) start += 1;
  return raw.subarray(start);
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

function isObject(value) {
  return (typeof value === 'object' || typeof value === 'function') && value !== null;
}
