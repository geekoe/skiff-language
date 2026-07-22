import { createHash } from 'node:crypto';

export const HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES = 512;
export const HOST_DIAGNOSTIC_MAX_ENTRIES = 3;
export const HOST_DIAGNOSTIC_TOTAL_EXCERPT_MAX_BYTES = 1536;

const UNKNOWN_CONTEXT = Object.freeze({ phase: 'unknown', subject: 'unknown' });
const PHASE_RANK = Object.freeze({ startup: 0, std: 1, 'host-prepare': 2, 'host-runner': 3 });
const DIAGNOSTIC_PRIORITY = Object.freeze({
  panic: 0,
  error: 1,
  'invalid-result': 2,
  failure: 3,
  diagnostic: 4,
  'command-outcome': 5,
});
const STREAM_PRIORITY = Object.freeze({ none: 0, stderr: 1, stdout: 2 });
const CANONICAL_RESULT_LINE = /^test result: ok\. \d+ passed; \d+ failed$/;
const SECONDARY_STARTUP_SHUTDOWN = /^\[skiff-instance\] stopping after startup failure$/i;
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
  const selected = selectDiagnostics(stdout, stderr, outcome?.error);
  const failed = outcome?.error != null || outcome?.signal != null || outcome?.code !== 0;
  if (selected.candidates.length === 0 && failed) {
    selected.candidates.push(syntheticOutcomeDiagnostic(outcome));
  }
  const ranked = [...selected.candidates].sort(compareDiagnosticCandidates);
  const diagnostics = ranked
    .slice(0, HOST_DIAGNOSTIC_MAX_ENTRIES)
    .map(publicDiagnostic);
  return {
    ...mostAdvancedContext(selected.markers),
    stdoutBytes: Buffer.byteLength(stdout),
    stderrBytes: Buffer.byteLength(stderr),
    diagnostics,
    diagnosticOmittedCount: ranked.length - diagnostics.length,
  };
}

export function assertHostDiagnosticMatchesOutcome(attempt, outcome) {
  const expected = captureHostDiagnostic(outcome);
  const actual = {
    phase: attempt?.phase,
    subject: attempt?.subject,
    stdoutBytes: attempt?.stdoutBytes,
    stderrBytes: attempt?.stderrBytes,
    diagnostics: attempt?.diagnostics,
    diagnosticOmittedCount: attempt?.diagnosticOmittedCount,
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

function selectDiagnostics(stdout, stderr, spawnError) {
  const candidates = [];
  const markers = [];
  for (const [stream, contents] of [['stdout', stdout], ['stderr', stderr]]) {
    for (const [lineIndex, originalLine] of contents.split(/\r?\n/).entries()) {
      const line = originalLine.trim();
      if (line.length === 0) continue;
      const marker = phaseMarker(line);
      if (marker !== null) {
        if (marker.phase !== 'unknown') markers.push(marker);
        continue;
      }
      if (isRoutineOutput(line)) continue;
      candidates.push({
        kind: diagnosticKind(line),
        stream,
        lineIndex,
        originalLine,
      });
    }
  }
  if (spawnError != null) {
    const originalLine = spawnError instanceof Error ? spawnError.message : String(spawnError);
    candidates.push({
      kind: 'error',
      stream: 'none',
      lineIndex: 0,
      originalLine,
    });
  }
  return { candidates, markers };
}

function phaseMarker(line) {
  if (line === '[skiff-tests] phase startup: isolated-runtime') {
    return { phase: 'startup', subject: 'isolated-runtime' };
  }
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
  if (/^\[skiff-instance\] supervisor failure:/i.test(line)) return 'error';
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
    kind: 'command-outcome',
    stream: 'none',
    lineIndex: 0,
    originalLine,
  };
}

function compareDiagnosticCandidates(left, right) {
  // Stream preference is a deterministic causal heuristic, never a merged-stream timestamp.
  return diagnosticPriority(left) - diagnosticPriority(right)
    || streamPriority(left.stream) - streamPriority(right.stream)
    || left.lineIndex - right.lineIndex;
}

function diagnosticPriority(candidate) {
  if (SECONDARY_STARTUP_SHUTDOWN.test(candidate.originalLine.trim())) return 6;
  return DIAGNOSTIC_PRIORITY[candidate.kind] ?? DIAGNOSTIC_PRIORITY.diagnostic;
}

function streamPriority(stream) {
  return STREAM_PRIORITY[stream] ?? 3;
}

function mostAdvancedContext(markers) {
  if (markers.length === 0) return UNKNOWN_CONTEXT;
  // Markers are monotonic phase declarations; diagnostics remain unordered across streams.
  const maximumRank = Math.max(...markers.map((marker) => PHASE_RANK[marker.phase] ?? -1));
  const advanced = markers.filter((marker) => PHASE_RANK[marker.phase] === maximumRank);
  const contexts = new Map(advanced.map((marker) => [
    `${marker.phase}\0${marker.subject}`,
    marker,
  ]));
  return contexts.size === 1 ? contexts.values().next().value : UNKNOWN_CONTEXT;
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
  if (!Array.isArray(evidence.diagnostics)
    || evidence.diagnostics.length > HOST_DIAGNOSTIC_MAX_ENTRIES
    || !Number.isInteger(evidence.diagnosticOmittedCount)
    || evidence.diagnosticOmittedCount < 0) {
    throw new Error('Host bounded diagnostic has an invalid diagnostic collection');
  }
  let totalExcerptBytes = 0;
  for (const diagnostic of evidence.diagnostics) {
    totalExcerptBytes += Buffer.byteLength(diagnostic?.sanitizedExcerpt ?? '');
    if (!['stdout', 'stderr', 'none'].includes(diagnostic?.stream)
      || !Object.hasOwn(DIAGNOSTIC_PRIORITY, diagnostic?.kind)
      || typeof diagnostic.sanitizedExcerpt !== 'string'
      || Buffer.byteLength(diagnostic.sanitizedExcerpt) > HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES
      || !/^[a-f0-9]{64}$/.test(diagnostic.originalLineSha256)
      || typeof diagnostic.truncated !== 'boolean') {
      throw new Error('Host bounded diagnostic has an invalid diagnostic entry');
    }
  }
  if (totalExcerptBytes > HOST_DIAGNOSTIC_TOTAL_EXCERPT_MAX_BYTES) {
    throw new Error('Host bounded diagnostic exceeded the aggregate excerpt limit');
  }
}

function text(value) {
  return typeof value === 'string' ? value : '';
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
