// Strict normalization for differential comparison (plan §9).
//
// Only four normalization kinds exist: uuid, timestamp, port (ephemeral
// leased ports), and logOrder (semantically independent log lines). A
// scenario must declare every normalization it applies, including the exact
// observation path. Nothing else is normalized; undeclared value differences
// are reported as differential failures.

export const NORMALIZATION_KINDS = Object.freeze([
  'uuid',
  'timestamp',
  'port',
  'logOrder',
]);

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const ISO_TIMESTAMP_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/;
const EPOCH_MILLIS_PATTERN = /^\d{10,13}$/;

export function assertNormalizationKind(kind) {
  if (!NORMALIZATION_KINDS.includes(kind)) {
    throw new Error(
      `unsupported normalization kind ${JSON.stringify(kind)}; `
      + `allowed kinds are ${NORMALIZATION_KINDS.join(', ')}`,
    );
  }
}

export function normalizeValue(value, { kind, ports = [] } = {}) {
  assertNormalizationKind(kind);
  if (typeof value === 'string') {
    switch (kind) {
      case 'uuid':
        return UUID_PATTERN.test(value) ? '<uuid>' : value;
      case 'timestamp':
        return ISO_TIMESTAMP_PATTERN.test(value) || EPOCH_MILLIS_PATTERN.test(value)
          ? '<timestamp>'
          : value;
      case 'port':
        return ports.includes(Number(value)) ? '<port>' : value;
      case 'logOrder':
        return value.split(/\r?\n/).filter((line) => line.length > 0).sort().join('\n');
      default:
        return value;
    }
  }
  if (Array.isArray(value)) {
    return value.map((entry) => normalizeValue(entry, { kind, ports }));
  }
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, entry]) => [
        key,
        normalizeValue(entry, { kind, ports }),
      ]),
    );
  }
  return value;
}

export function normalizeObservationPath(observation, path, { kind, ports = [] }) {
  const segments = path.split('.').filter((segment) => segment.length > 0);
  return walkAndNormalize(observation, segments, { kind, ports, skipMissing: false });
}

function walkAndNormalize(value, segments, options) {
  if (segments.length === 0) {
    return normalizeValue(value, options);
  }
  const [head, ...rest] = segments;
  if (head === '*') {
    if (!Array.isArray(value)) {
      throw new Error(`normalization path wildcard requires an array at ${head}`);
    }
    return value.map((entry) =>
      walkAndNormalize(entry, rest, { ...options, skipMissing: true }));
  }
  if (Array.isArray(value)) {
    const index = Number(head);
    if (!Number.isInteger(index) || index < 0 || index >= value.length) {
      throw new Error(`normalization array index ${head} is out of range`);
    }
    const next = [...value];
    next[index] = walkAndNormalize(value[index], rest, options);
    return next;
  }
  if (value === null || typeof value !== 'object') {
    throw new Error(`normalization path ${head} cannot descend into ${typeof value}`);
  }
  if (!Object.hasOwn(value, head)) {
    if (options.skipMissing) {
      return value;
    }
    throw new Error(`normalization path member ${head} is missing`);
  }
  return {
    ...value,
    [head]: walkAndNormalize(value[head], rest, options),
  };
}

export function isUuid(value) {
  return typeof value === 'string' && UUID_PATTERN.test(value);
}

export function isIsoTimestamp(value) {
  return typeof value === 'string' && ISO_TIMESTAMP_PATTERN.test(value);
}
