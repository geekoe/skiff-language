import { createHash } from 'node:crypto';

export const FIXTURE_CARGO_DIAGNOSTIC_SCHEMA_VERSION =
  'skiff-package-service-ecosystem-smoke-fixture-cargo-diagnostic-v1';
export const FIXTURE_CARGO_DIAGNOSTIC_PROPERTY = 'fixtureCargoDiagnostic';
export const FIXTURE_CARGO_DIAGNOSTIC_EXCERPT_MAX_BYTES = 512;
export const FIXTURE_CARGO_DIAGNOSTIC_MAX_ENTRIES = 3;
export const FIXTURE_CARGO_DIAGNOSTIC_TOTAL_EXCERPT_MAX_BYTES = 1536;

const FIXTURE_CARGO_PHASE = 'fixture-cargo';
const ANSI_OSC = /\u001B\][^\u0007]*(?:\u0007|\u001B\\)/g;
const ANSI_CSI = /(?:\u001B\[|\u009B)[0-?]*[ -/]*[@-~]/g;
const ANSI_SINGLE = /\u001B[@-_]/g;
const CONTROL_CHARACTER = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F]/g;
const HTTP_BODY = /\b((?:http|request|response)\s+body\s*[:=])[^\r\n]*/gi;
const LABELED_SECRET =
  /\b(authorization|api[-_ ]?key|password|secret|token)\s*[:=]\s*(?:"[^"]*"|'[^']*'|\S+)/gi;
const BEARER_SECRET = /\bbearer\s+\S+/gi;
const COMMON_SECRET_TOKEN =
  /\b(?:AKIA[A-Z0-9]{16}|gh[pousr]_[a-zA-Z0-9]{20,}|sk-[a-zA-Z0-9_-]{16,})\b/g;
const SECRET_SENTINEL = /\b(?!REDACTED_SECRET\b)[a-z0-9._-]*secret[a-z0-9._-]*\b/gi;
const URL = /\b(?:file|https?|wss?):\/\/[^\s"'<>]+/gi;
const WINDOWS_PATH = /\b[a-z]:\\(?:[^\s"'<>:;,()[\]{}=]+\\)*[^\s"'<>:;,()[\]{}=]+/gi;
const POSIX_PATH = /(?<![\w.-])\/(?:[^/\s"'<>:;,()[\]{}=]+\/)*[^/\s"'<>:;,()[\]{}=]+/g;
const HOME_PATH = /(?:^|\s)~\/(?:[^\s"'<>]+\/)*[^\s"'<>]+/g;

export function retainFixtureCargoDiagnostic(error) {
  if ((typeof error !== 'object' && typeof error !== 'function') || error === null) {
    return error;
  }
  if (Object.hasOwn(error, FIXTURE_CARGO_DIAGNOSTIC_PROPERTY)) {
    return error;
  }
  Object.defineProperty(error, FIXTURE_CARGO_DIAGNOSTIC_PROPERTY, {
    value: captureFixtureCargoDiagnostic(error),
    enumerable: true,
    writable: false,
    configurable: false,
  });
  return error;
}

export function captureFixtureCargoDiagnostic(error) {
  const stdout = errorStream(error, 'stdout');
  const stderr = errorStream(error, 'stderr');
  const selected = [];
  let diagnosticCount = 0;
  for (const [stream, contents] of [['stderr', stderr], ['stdout', stdout]]) {
    visitNonemptyLines(contents, (originalLine) => {
      diagnosticCount += 1;
      if (selected.length < FIXTURE_CARGO_DIAGNOSTIC_MAX_ENTRIES) {
        selected.push(publicDiagnostic(stream, originalLine));
      }
    });
  }
  if (diagnosticCount === 0) {
    diagnosticCount = 1;
    selected.push(publicDiagnostic(
      'none',
      `cargo exited with ${commandSignal(error) ?? commandCode(error) ?? 'unknown'}`,
    ));
  }

  return Object.freeze({
    schemaVersion: FIXTURE_CARGO_DIAGNOSTIC_SCHEMA_VERSION,
    command: 'cargo',
    phase: FIXTURE_CARGO_PHASE,
    code: commandCode(error),
    signal: commandSignal(error),
    stdoutBytes: Buffer.byteLength(stdout),
    stdoutSha256: sha256(stdout),
    stderrBytes: Buffer.byteLength(stderr),
    stderrSha256: sha256(stderr),
    diagnostics: Object.freeze(selected),
    diagnosticOmittedCount: diagnosticCount - selected.length,
  });
}

export function sanitizeFixtureCargoDiagnostic(value) {
  return value
    .replace(ANSI_OSC, '')
    .replace(ANSI_CSI, '')
    .replace(ANSI_SINGLE, '')
    .replace(CONTROL_CHARACTER, '')
    .replace(HTTP_BODY, '$1 <REDACTED_HTTP_BODY>')
    .replace(LABELED_SECRET, '$1=<REDACTED_SECRET>')
    .replace(BEARER_SECRET, '<REDACTED_SECRET>')
    .replace(COMMON_SECRET_TOKEN, '<REDACTED_SECRET>')
    .replace(SECRET_SENTINEL, '<REDACTED_SECRET>')
    .replace(URL, '<URL>')
    .replace(WINDOWS_PATH, '<PATH>')
    .replace(HOME_PATH, (match) => `${match.startsWith(' ') ? ' ' : ''}<PATH>`)
    .replace(POSIX_PATH, '<PATH>');
}

function publicDiagnostic(stream, originalLine) {
  const bounded = boundedUtf8(
    sanitizeFixtureCargoDiagnostic(originalLine),
    FIXTURE_CARGO_DIAGNOSTIC_EXCERPT_MAX_BYTES,
  );
  return Object.freeze({
    stream,
    sanitizedExcerpt: bounded.value,
    originalLineSha256: sha256(originalLine),
    truncated: bounded.truncated,
  });
}

function visitNonemptyLines(value, visitor) {
  let start = 0;
  while (start <= value.length) {
    const newline = value.indexOf('\n', start);
    const end = newline === -1 ? value.length : newline;
    const lineEnd = end > start && value[end - 1] === '\r' ? end - 1 : end;
    const line = value.slice(start, lineEnd);
    if (line.trim() !== '') visitor(line);
    if (newline === -1) return;
    start = newline + 1;
  }
}

function boundedUtf8(value, maximumBytes) {
  const originalBytes = Buffer.byteLength(value);
  if (originalBytes <= maximumBytes) {
    return { value, truncated: false };
  }
  let byteCount = 0;
  let bounded = '';
  for (const character of value) {
    const characterBytes = Buffer.byteLength(character);
    if (byteCount + characterBytes > maximumBytes) break;
    bounded += character;
    byteCount += characterBytes;
  }
  return { value: bounded, truncated: true };
}

function errorStream(error, property) {
  return typeof error?.[property] === 'string' ? error[property] : '';
}

function commandCode(error) {
  return typeof error?.code === 'number' || typeof error?.code === 'string'
    ? error.code
    : null;
}

function commandSignal(error) {
  return typeof error?.signal === 'string' ? error.signal : null;
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
