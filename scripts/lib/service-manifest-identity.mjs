import { isMap, isScalar, parseDocument } from 'yaml';

import { assertServiceId } from './dev-registry-store.mjs';

const SERVICE_KINDS = new Set(['service', 'test']);

export function parseServiceManifestIdentity(source, label) {
  const document = parseDocument(source, {
    uniqueKeys: true,
    merge: false,
    schema: 'core',
    prettyErrors: false,
  });
  const problems = [...document.errors, ...document.warnings];
  if (problems.length > 0) {
    throw new Error(`${label} YAML parse error: ${normalizeYamlError(problems[0].message)}`);
  }
  if (!isMap(document.contents)) {
    throw new Error(`${label} root must be an object`);
  }

  const id = readStringField(document.contents, 'id', label, { required: true });
  assertServiceId(id, `${label} id`);

  const kind = readStringField(document.contents, 'kind', label, { required: false })
    ?? 'service';
  if (!SERVICE_KINDS.has(kind)) {
    throw new Error(`${label} kind must be service or test`);
  }
  return { id, kind };
}

function readStringField(mapping, field, label, { required }) {
  const value = mapping.get(field, true);
  if (value === undefined) {
    if (required) {
      throw new Error(`${label} ${field} is required`);
    }
    return undefined;
  }
  if (!isScalar(value) || typeof value.value !== 'string') {
    throw new Error(`${label} ${field} must be a string`);
  }
  return value.value;
}

function normalizeYamlError(message) {
  return message.replace(/\s+at line \d+, column \d+.*$/s, '').trim();
}
