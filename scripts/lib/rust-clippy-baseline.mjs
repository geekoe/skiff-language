import { isAbsolute, relative, resolve, sep } from 'node:path';

export const TOO_MANY_LINES_LINT = 'clippy::too_many_lines';
export const TOO_MANY_LINES_BASELINE_VERSION = 1;

const HARD_DIAGNOSTIC_LEVELS = new Set(['error', 'failure-note']);
const ITEM_PATTERN = /^fn [A-Za-z_][A-Za-z0-9_]*$/;

export function analyzeClippyRun(outcome, { root }) {
  assertSuccessfulCargoOutcome(outcome);
  const messages = parseCargoJsonMessages(outcome.stdout);
  const hardDiagnostics = compilerDiagnostics(messages)
    .filter(({ message }) => HARD_DIAGNOSTIC_LEVELS.has(message.level));
  if (hardDiagnostics.length > 0) {
    throw new Error([
      'cargo clippy emitted hard diagnostic(s) despite a successful exit:',
      ...hardDiagnostics.map(formatDiagnostic),
    ].join('\n'));
  }

  const findings = collectTooManyLinesFindings(messages, { root });
  const advisoryCounts = advisoryDiagnosticCounts(messages);
  return {
    messages,
    findings,
    advisoryCounts,
    advisoryCount: advisoryCounts.reduce((total, entry) => total + entry.count, 0),
  };
}

export function assertSuccessfulCargoOutcome(outcome) {
  if (!outcome || typeof outcome !== 'object') {
    throw new Error('cargo clippy returned an invalid execution outcome');
  }
  if (outcome.error) {
    throw new Error(`cargo clippy failed to spawn: ${outcome.error.message ?? String(outcome.error)}`);
  }
  if (outcome.signal) {
    throw new Error(`cargo clippy terminated by signal ${outcome.signal}${stderrSuffix(outcome.stderr)}`);
  }
  if (outcome.code !== 0) {
    const status = outcome.code === null || outcome.code === undefined
      ? 'without an exit code'
      : `with exit code ${outcome.code}`;
    throw new Error(`cargo clippy exited ${status}${hardDiagnosticSuffix(outcome.stdout)}${stderrSuffix(outcome.stderr)}`);
  }
}

export function parseCargoJsonMessages(stdout) {
  if (typeof stdout !== 'string') {
    throw new Error('cargo clippy stdout must be a string');
  }
  const messages = [];
  const lines = stdout.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index].trim();
    if (line.length === 0) {
      continue;
    }
    let message;
    try {
      message = JSON.parse(line);
    } catch (error) {
      throw new Error(
        `cargo clippy emitted invalid JSON on stdout line ${index + 1}: ${error.message}`,
      );
    }
    if (!message || typeof message !== 'object' || Array.isArray(message)) {
      throw new Error(`cargo clippy emitted a non-object JSON message on stdout line ${index + 1}`);
    }
    messages.push(message);
  }
  return messages;
}

export function collectTooManyLinesFindings(messages, { root }) {
  if (!root) {
    throw new Error('too_many_lines identity extraction requires the repository root');
  }
  const findingsByIdentity = new Map();
  for (const diagnostic of compilerDiagnostics(messages)) {
    if (diagnostic.message.code?.code !== TOO_MANY_LINES_LINT) {
      continue;
    }
    const finding = findingFromDiagnostic(diagnostic.message, root);
    const identity = findingIdentity(finding);
    const existing = findingsByIdentity.get(identity);
    if (existing === undefined) {
      findingsByIdentity.set(identity, finding);
      continue;
    }
    if (existing.span !== finding.span) {
      throw new Error(
        `too_many_lines identity collision for ${identity}: ${existing.location} and ${finding.location}`,
      );
    }
  }
  return [...findingsByIdentity.values()]
    .map(({ path, item }) => ({ path, item }))
    .sort(compareFindings);
}

export function parseTooManyLinesBaseline(value) {
  let baseline = value;
  if (typeof value === 'string') {
    try {
      baseline = JSON.parse(value);
    } catch (error) {
      throw new Error(`invalid too_many_lines baseline JSON: ${error.message}`);
    }
  }
  if (!baseline || typeof baseline !== 'object' || Array.isArray(baseline)) {
    throw new Error('too_many_lines baseline must be an object');
  }
  if (baseline.version !== TOO_MANY_LINES_BASELINE_VERSION) {
    throw new Error(
      `too_many_lines baseline version must be ${TOO_MANY_LINES_BASELINE_VERSION}`,
    );
  }
  if (baseline.lint !== TOO_MANY_LINES_LINT) {
    throw new Error(`too_many_lines baseline lint must be ${TOO_MANY_LINES_LINT}`);
  }
  if (!Array.isArray(baseline.entries)) {
    throw new Error('too_many_lines baseline entries must be an array');
  }

  const entries = baseline.entries.map((entry, index) => validateBaselineEntry(entry, index));
  const identities = entries.map(findingIdentity);
  if (new Set(identities).size !== identities.length) {
    throw new Error('too_many_lines baseline entries must have unique path + item identities');
  }
  const sorted = [...entries].sort(compareFindings);
  if (!entries.every((entry, index) => findingIdentity(entry) === findingIdentity(sorted[index]))) {
    throw new Error('too_many_lines baseline entries must be sorted by path + item');
  }
  return { version: baseline.version, lint: baseline.lint, entries };
}

export function compareTooManyLinesBaseline(actualFindings, baselineEntries) {
  const actual = new Map(actualFindings.map((finding) => [findingIdentity(finding), finding]));
  const baseline = new Map(baselineEntries.map((finding) => [findingIdentity(finding), finding]));
  return {
    unexpected: [...actual]
      .filter(([identity]) => !baseline.has(identity))
      .map(([, finding]) => finding)
      .sort(compareFindings),
    stale: [...baseline]
      .filter(([identity]) => !actual.has(identity))
      .map(([, finding]) => finding)
      .sort(compareFindings),
  };
}

export function assertTooManyLinesBaselineMatches(actualFindings, baselineEntries) {
  const difference = compareTooManyLinesBaseline(actualFindings, baselineEntries);
  if (difference.unexpected.length === 0 && difference.stale.length === 0) {
    return;
  }
  throw new Error([
    'clippy::too_many_lines baseline mismatch:',
    ...(difference.unexpected.length === 0 ? [] : [
      'unexpected finding(s):',
      ...difference.unexpected.map((finding) => `+ ${formatFinding(finding)}`),
    ]),
    ...(difference.stale.length === 0 ? [] : [
      'stale baseline entry/entries (remove them):',
      ...difference.stale.map((finding) => `- ${formatFinding(finding)}`),
    ]),
  ].join('\n'));
}

export function createTooManyLinesBaseline(entries) {
  return {
    version: TOO_MANY_LINES_BASELINE_VERSION,
    lint: TOO_MANY_LINES_LINT,
    identity: 'repository-relative source path + Rust function item name from the primary span',
    entries: entries.map(({ path, item }) => ({ path, item })).sort(compareFindings),
  };
}

function compilerDiagnostics(messages) {
  return messages.filter((entry) =>
    entry.reason === 'compiler-message'
    && entry.message
    && typeof entry.message === 'object'
    && !Array.isArray(entry.message));
}

function findingFromDiagnostic(message, root) {
  const primarySpans = Array.isArray(message.spans)
    ? message.spans.filter((span) => span?.is_primary === true)
    : [];
  if (primarySpans.length !== 1) {
    throw new Error(
      `clippy::too_many_lines diagnostic must have exactly one primary span, found ${primarySpans.length}`,
    );
  }
  const span = primarySpans[0];
  const path = repositoryRelativeDiagnosticPath(span.file_name, root);
  const source = Array.isArray(span.text)
    ? span.text.map((line) => line?.text ?? '').join('\n')
    : '';
  const match = source.match(/\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\b/);
  if (!match) {
    throw new Error(
      `cannot extract a Rust function item from clippy::too_many_lines primary span at ${path}:${span.line_start ?? '?'}`,
    );
  }
  const item = `fn ${match[1]}`;
  const lineStart = integerOrUnknown(span.line_start);
  const columnStart = integerOrUnknown(span.column_start);
  const lineEnd = integerOrUnknown(span.line_end);
  const columnEnd = integerOrUnknown(span.column_end);
  return {
    path,
    item,
    location: `${path}:${lineStart}:${columnStart}`,
    span: `${path}:${lineStart}:${columnStart}:${lineEnd}:${columnEnd}`,
  };
}

function repositoryRelativeDiagnosticPath(fileName, root) {
  if (typeof fileName !== 'string' || fileName.length === 0) {
    throw new Error('clippy::too_many_lines primary span requires a source path');
  }
  const absoluteRoot = resolve(root);
  const absolutePath = isAbsolute(fileName) ? resolve(fileName) : resolve(absoluteRoot, fileName);
  const relativePath = relative(absoluteRoot, absolutePath);
  if (
    relativePath.length === 0
    || relativePath === '..'
    || relativePath.startsWith(`..${sep}`)
    || isAbsolute(relativePath)
  ) {
    throw new Error(`clippy::too_many_lines source path is outside the repository: ${fileName}`);
  }
  return relativePath.split(sep).join('/');
}

function advisoryDiagnosticCounts(messages) {
  const counts = new Map();
  for (const { message } of compilerDiagnostics(messages)) {
    if (message.level !== 'warning' || message.code?.code === TOO_MANY_LINES_LINT) {
      continue;
    }
    const code = message.code?.code ?? '<uncoded-warning>';
    counts.set(code, (counts.get(code) ?? 0) + 1);
  }
  return [...counts]
    .map(([code, count]) => ({ code, count }))
    .sort((left, right) => left.code.localeCompare(right.code));
}

function validateBaselineEntry(entry, index) {
  if (!entry || typeof entry !== 'object' || Array.isArray(entry)) {
    throw new Error(`too_many_lines baseline entry ${index} must be an object`);
  }
  const keys = Object.keys(entry).sort();
  if (keys.length !== 2 || keys[0] !== 'item' || keys[1] !== 'path') {
    throw new Error(`too_many_lines baseline entry ${index} must contain only path and item`);
  }
  if (
    typeof entry.path !== 'string'
    || entry.path.length === 0
    || entry.path.includes('\\')
    || isAbsolute(entry.path)
    || entry.path === '..'
    || entry.path.startsWith('../')
  ) {
    throw new Error(`too_many_lines baseline entry ${index} has an invalid repository-relative path`);
  }
  if (typeof entry.item !== 'string' || !ITEM_PATTERN.test(entry.item)) {
    throw new Error(`too_many_lines baseline entry ${index} has an invalid Rust function item`);
  }
  return { path: entry.path, item: entry.item };
}

function findingIdentity(finding) {
  return `${finding.path} :: ${finding.item}`;
}

function compareFindings(left, right) {
  return findingIdentity(left).localeCompare(findingIdentity(right));
}

function formatFinding(finding) {
  return findingIdentity(finding);
}

function formatDiagnostic({ message }) {
  const code = message.code?.code ?? '<uncoded>';
  const primary = Array.isArray(message.spans)
    ? message.spans.find((span) => span?.is_primary === true)
    : undefined;
  const location = primary?.file_name
    ? ` at ${primary.file_name}:${primary.line_start ?? '?'}`
    : '';
  return `- ${code}: ${message.message ?? '<missing message>'}${location}`;
}

function hardDiagnosticSuffix(stdout) {
  if (typeof stdout !== 'string' || stdout.length === 0) {
    return '';
  }
  try {
    const hard = compilerDiagnostics(parseCargoJsonMessages(stdout))
      .filter(({ message }) => HARD_DIAGNOSTIC_LEVELS.has(message.level));
    return hard.length === 0 ? '' : `\n${hard.map(formatDiagnostic).join('\n')}`;
  } catch {
    return '';
  }
}

function stderrSuffix(stderr) {
  if (typeof stderr !== 'string' || stderr.trim().length === 0) {
    return '';
  }
  const trimmed = stderr.trim();
  const maximumLength = 4_000;
  const detail = trimmed.length <= maximumLength
    ? trimmed
    : `…${trimmed.slice(-maximumLength)}`;
  return `\nstderr:\n${detail}`;
}

function integerOrUnknown(value) {
  return Number.isInteger(value) ? value : '?';
}
