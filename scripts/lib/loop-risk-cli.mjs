const redactedPath = '<redacted-path>';

export function parseLoopRiskArgs(argv, spec) {
  const flags = new Set(spec.flags ?? []);
  const singletonValues = new Set(spec.singletonValues ?? []);
  const repeatableValues = new Set(spec.repeatableValues ?? []);
  assertDisjointSpec(flags, singletonValues, repeatableValues);

  const parsedFlags = new Set();
  const parsedValues = new Map();
  for (let index = 0; index < argv.length; index += 1) {
    const rawArg = argv[index];
    if (!rawArg.startsWith('--') || rawArg === '--') {
      throw new Error('unexpected positional argument');
    }

    const equalsIndex = rawArg.indexOf('=');
    const option = equalsIndex === -1 ? rawArg : rawArg.slice(0, equalsIndex);
    const name = option.slice(2);
    if (flags.has(name)) {
      if (equalsIndex !== -1) {
        throw new Error(`${option} does not accept a value`);
      }
      if (parsedFlags.has(name)) {
        throw new Error(`${option} was provided more than once`);
      }
      parsedFlags.add(name);
      continue;
    }

    const singleton = singletonValues.has(name);
    if (!singleton && !repeatableValues.has(name)) {
      throw new Error(`unknown option ${option}`);
    }

    const value = equalsIndex === -1 ? argv[index + 1] : rawArg.slice(equalsIndex + 1);
    if (!value || (equalsIndex === -1 && value.startsWith('--'))) {
      throw new Error(`${option} requires a value`);
    }
    if (singleton && parsedValues.has(name)) {
      throw new Error(`${option} was provided more than once`);
    }

    const values = parsedValues.get(name) ?? [];
    values.push(value);
    parsedValues.set(name, values);
    if (equalsIndex === -1) {
      index += 1;
    }
  }

  return Object.freeze({
    hasFlag(name) {
      return parsedFlags.has(name);
    },
    value(name) {
      return parsedValues.get(name)?.[0];
    },
    values(...names) {
      return names.flatMap((name) => parsedValues.get(name) ?? []);
    },
    list(...names) {
      const entries = [];
      for (const name of names) {
        for (const value of parsedValues.get(name) ?? []) {
          const items = value.split(',').map((item) => item.trim()).filter(Boolean);
          if (items.length === 0) {
            throw new Error(`--${name} requires a non-empty value`);
          }
          entries.push(...items);
        }
      }
      return entries;
    },
  });
}

export function readPositiveIntegerArg(args, name, fallback) {
  const raw = args.value(name);
  const value = raw === undefined ? fallback : Number(raw);
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(`--${name} must be a positive integer`);
  }
  return value;
}

export function readNonNegativeIntegerArg(args, name, fallback) {
  const raw = args.value(name);
  const value = raw === undefined ? fallback : Number(raw);
  if (!Number.isInteger(value) || value < 0) {
    throw new Error(`--${name} must be a non-negative integer`);
  }
  return value;
}

export function readNumberArg(args, name, fallback) {
  const raw = args.value(name);
  if (raw === undefined) {
    return fallback;
  }
  const value = Number(raw);
  if (!Number.isFinite(value)) {
    throw new Error(`--${name} must be a number`);
  }
  return value;
}

export function collectLoopRiskUrlArgs(argv, names) {
  const optionNames = new Set(names.map((name) => `--${name}`));
  const values = [];
  for (let index = 0; index < argv.length; index += 1) {
    const rawArg = argv[index];
    const equalsIndex = rawArg.indexOf('=');
    const option = equalsIndex === -1 ? rawArg : rawArg.slice(0, equalsIndex);
    if (!optionNames.has(option)) {
      continue;
    }
    const value = equalsIndex === -1 ? argv[index + 1] : rawArg.slice(equalsIndex + 1);
    if (value && (equalsIndex !== -1 || !value.startsWith('--'))) {
      values.push(value);
    }
    if (equalsIndex === -1) {
      index += 1;
    }
  }
  return unique(values);
}

export function redactLoopRiskUrl(rawUrl) {
  try {
    const parsed = new URL(rawUrl);
    if (!parsed.protocol || !parsed.host) {
      return `<redacted-url>/${redactedPath}`;
    }
    return `${parsed.protocol}//${parsed.host}/${redactedPath}`;
  } catch {
    return `<invalid-url>/${redactedPath}`;
  }
}

export function formatLoopRiskJson(value, rawUrls) {
  return redactKnownLoopRiskUrls(JSON.stringify(value, null, 2), rawUrls);
}

export function redactKnownLoopRiskUrls(text, rawUrls) {
  const replacements = [];
  for (const rawUrl of unique(rawUrls.filter(Boolean))) {
    const display = redactLoopRiskUrl(rawUrl);
    replacements.push([rawUrl, display]);
    try {
      const normalized = new URL(rawUrl).href;
      if (normalized !== rawUrl) {
        replacements.push([normalized, display]);
      }
    } catch {
      // The invalid raw value is still replaced exactly without trying to normalize it.
    }
  }

  let result = String(text);
  for (const [rawUrl, display] of uniquePairs(replacements).sort(
    ([left], [right]) => right.length - left.length,
  )) {
    result = result.replaceAll(rawUrl, display);
  }
  return result;
}

function assertDisjointSpec(...sets) {
  const seen = new Set();
  for (const values of sets) {
    for (const value of values) {
      if (!value || seen.has(value)) {
        throw new Error(`invalid loop-risk CLI option schema for ${value || '<empty>'}`);
      }
      seen.add(value);
    }
  }
}

function unique(values) {
  return Array.from(new Set(values));
}

function uniquePairs(pairs) {
  const seen = new Set();
  return pairs.filter(([raw]) => {
    if (seen.has(raw)) {
      return false;
    }
    seen.add(raw);
    return true;
  });
}
