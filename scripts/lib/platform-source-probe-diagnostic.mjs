import { createHash } from 'node:crypto';

export const HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES = 512;

const UNKNOWN_CONTEXT = Object.freeze({ phase: 'unknown', subject: 'unknown' });
const CANONICAL_RESULT_LINE = /^test result: ok\. \d+ passed; \d+ failed$/;
const HTTP_BODY = /\b((?:http|request|response)\s+body\s*[:=])[^\r\n]*/gi;
const LABELED_SECRET = /\b(authorization|api[-_ ]?key|password|secret|token)\s*[:=]\s*(?:"[^"]*"|'[^']*'|\S+)/gi;
const BEARER_SECRET = /\bbearer\s+\S+/gi;
const SECRET_SENTINEL = /\b(?!REDACTED_SECRET\b)[a-z0-9._-]*secret[a-z0-9._-]*\b/gi;
const URL = /\b(?:file|https?|wss?):\/\/[^\s"'<>]+/gi;
const WINDOWS_PATH = /\b[a-z]:\\(?:[^\s"'<>:;,()[\]{}=]+\\)*[^\s"'<>:;,()[\]{}=]+/gi;
const POSIX_PATH = /(?<![\w.-])\/(?:[^/\s"'<>:;,()[\]{}=]+\/)*[^/\s"'<>:;,()[\]{}=]+/g;
const HOME_PATH = /(?:^|\s)~\/(?:[^\s"'<>]+\/)*[^\s"'<>]+/g;

export function captureHostDiagnostic(outcome) {
  const stdout = text(outcome?.stdout);
  const stderr = text(outcome?.stderr);
  const selected = selectDiagnostic(stdout, stderr, outcome?.error);
  const failed = outcome?.error != null || outcome?.signal != null || outcome?.code !== 0;
  const diagnostic = selected ?? (failed ? syntheticOutcomeDiagnostic(outcome) : null);
  return {
    phase: diagnostic?.phase ?? 'unknown',
    subject: diagnostic?.subject ?? 'unknown',
    stdoutBytes: Buffer.byteLength(stdout),
    stderrBytes: Buffer.byteLength(stderr),
    firstDiagnostic: diagnostic === null ? null : publicDiagnostic(diagnostic),
  };
}

export function assertHostDiagnosticMatchesOutcome(attempt, outcome) {
  const expected = captureHostDiagnostic(outcome);
  const actual = {
    phase: attempt?.phase,
    subject: attempt?.subject,
    stdoutBytes: attempt?.stdoutBytes,
    stderrBytes: attempt?.stderrBytes,
    firstDiagnostic: attempt?.firstDiagnostic,
  };
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error('Host bounded diagnostic does not match the original command outcome');
  }
  assertDiagnosticShape(actual);
}

export function sanitizeHostDiagnostic(value) {
  return text(value)
    .replace(HTTP_BODY, '$1 <REDACTED_HTTP_BODY>')
    .replace(LABELED_SECRET, '$1=<REDACTED_SECRET>')
    .replace(BEARER_SECRET, '<REDACTED_SECRET>')
    .replace(SECRET_SENTINEL, '<REDACTED_SECRET>')
    .replace(URL, '<URL>')
    .replace(WINDOWS_PATH, '<PATH>')
    .replace(HOME_PATH, (match) => `${match.startsWith(' ') ? ' ' : ''}<PATH>`)
    .replace(POSIX_PATH, '<PATH>');
}

function selectDiagnostic(stdout, stderr, spawnError) {
  let context = UNKNOWN_CONTEXT;
  const candidates = [];
  for (const [stream, contents] of [['stdout', stdout], ['stderr', stderr]]) {
    for (const originalLine of contents.split(/\r?\n/)) {
      const line = originalLine.trim();
      if (line.length === 0) continue;
      const marker = phaseMarker(line);
      if (marker !== null) {
        context = marker;
        continue;
      }
      if (isRoutineOutput(line)) continue;
      candidates.push({
        ...context,
        kind: diagnosticKind(line),
        stream,
        originalLine,
      });
    }
  }
  if (spawnError != null) {
    const originalLine = spawnError instanceof Error ? spawnError.message : String(spawnError);
    candidates.push({
      ...context,
      kind: 'error',
      stream: 'none',
      originalLine,
    });
  }
  return candidates.find((entry) => entry.kind !== 'diagnostic')
    ?? candidates.find((entry) => entry.stream === 'stderr')
    ?? candidates[0]
    ?? null;
}

function phaseMarker(line) {
  if (/^\[skiff-test\] isolated runtime (?:control|workspace):/.test(line)) {
    return { phase: 'startup', subject: 'isolated-runtime' };
  }
  if (/^\[skiff-tests\] preparing package-service-host:/.test(line)) {
    return { phase: 'host-prepare', subject: 'package-service-host' };
  }
  if (/^\[skiff-tests\] running package-service-host:/.test(line)) {
    return { phase: 'host-runner', subject: 'package-service-host' };
  }
  const std = /^\[skiff-tests\] running ([a-z][a-z0-9-]{0,63}):/.exec(line);
  if (std !== null) return { phase: 'std', subject: std[1] };
  if (line.startsWith('[skiff-test]') || line.startsWith('[skiff-tests]')) {
    return UNKNOWN_CONTEXT;
  }
  return null;
}

function diagnosticKind(line) {
  if (/\bpanic(?:ked)?\b/i.test(line)) return 'panic';
  if (/^(?:error|fatal)\b|\berror:/i.test(line)) return 'error';
  if (/\b(?:failed|failure)\b/i.test(line)) return 'failure';
  if (line.startsWith('test result:')) return 'invalid-result';
  return 'diagnostic';
}

function isRoutineOutput(line) {
  return CANONICAL_RESULT_LINE.test(line)
    || line.startsWith('PASS ')
    || /^\[skiff-tests\] passed \d+ canonical source test entr(?:y|ies)$/.test(line)
    || /^(?:Blocking|Checking|Compiling|Dirty|Downloaded|Downloading|Finished|Fresh|Locking|Running)\b/.test(line);
}

function syntheticOutcomeDiagnostic(outcome) {
  const originalLine = `child outcome: ${outcome?.signal ?? outcome?.code ?? 'spawn'}`;
  return {
    ...UNKNOWN_CONTEXT,
    kind: 'command-outcome',
    stream: 'none',
    originalLine,
  };
}

function publicDiagnostic(diagnostic) {
  const sanitized = sanitizeHostDiagnostic(diagnostic.originalLine);
  const bounded = boundedUtf8(sanitized, HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES);
  return {
    kind: diagnostic.kind,
    stream: diagnostic.stream,
    sanitizedExcerpt: bounded.value,
    originalLineSha256: sha256(diagnostic.originalLine),
    truncated: bounded.truncated,
  };
}

function boundedUtf8(value, maximumBytes) {
  if (Buffer.byteLength(value) <= maximumBytes) return { value, truncated: false };
  let bounded = '';
  for (const character of value) {
    if (Buffer.byteLength(bounded + character) > maximumBytes) break;
    bounded += character;
  }
  return { value: bounded, truncated: true };
}

function assertDiagnosticShape(evidence) {
  if (!['startup', 'std', 'host-prepare', 'host-runner', 'unknown'].includes(evidence.phase)) {
    throw new Error('Host bounded diagnostic has an invalid phase');
  }
  if (typeof evidence.subject !== 'string' || evidence.subject.length === 0) {
    throw new Error('Host bounded diagnostic has an invalid subject');
  }
  if (!Number.isInteger(evidence.stdoutBytes) || evidence.stdoutBytes < 0
    || !Number.isInteger(evidence.stderrBytes) || evidence.stderrBytes < 0) {
    throw new Error('Host bounded diagnostic has invalid stream byte counts');
  }
  if (evidence.firstDiagnostic === null) return;
  const diagnostic = evidence.firstDiagnostic;
  if (!['stdout', 'stderr', 'none'].includes(diagnostic.stream)
    || typeof diagnostic.kind !== 'string'
    || diagnostic.kind.length === 0
    || typeof diagnostic.sanitizedExcerpt !== 'string'
    || Buffer.byteLength(diagnostic.sanitizedExcerpt) > HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES
    || !/^[a-f0-9]{64}$/.test(diagnostic.originalLineSha256)
    || typeof diagnostic.truncated !== 'boolean') {
    throw new Error('Host bounded diagnostic has an invalid first diagnostic');
  }
}

function text(value) {
  return typeof value === 'string' ? value : '';
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
