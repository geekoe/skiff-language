import { parseDocument, isAlias, isMap, isScalar, isSeq, type Node } from 'yaml';

import { sha256Hex, stableStringify } from '../manifest/identity.js';

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

export type JsonObject = { [key: string]: JsonValue };
export type ConfigSourceClass = 'bundle' | 'secret';
export type ConfigShapeValueType = 'string' | 'number' | 'bool' | 'Json' | 'JsonObject';
export type ConfigShapeType = ConfigShapeValueType;

export interface ConfigSourceSpec {
  path: string;
  label: string;
  sourceClass: ConfigSourceClass;
}

export interface ConfigSource {
  sourceClass: ConfigSourceClass;
  label: string;
  value: JsonObject;
}

export interface ConfigShapeEntry {
  path: string;
  type: ConfigShapeValueType;
  required?: boolean;
}

export interface NormalizedConfigShapeEntry {
  path: string;
  type: ConfigShapeValueType;
  required: boolean;
}

export interface ConfigShape {
  schemaVersion: 'skiff-config-shape-v1';
  entries: NormalizedConfigShapeEntry[];
}

export interface ConfigProvenanceEntry {
  sourceClass: ConfigSourceClass;
  label: string;
}

export interface ResolvedConfig {
  resolvedConfig: JsonObject;
  redactedResolvedConfig: JsonObject;
  provenance: {
    leaves: Record<string, ConfigProvenanceEntry>;
    tombstones: Record<string, ConfigProvenanceEntry>;
  };
  redactionProjectionIdentity: string;
}

const CONFIG_KEY_PATTERN = /^[A-Za-z_][A-Za-z0-9_-]*$/;
export const CONFIG_SHAPE_VALUE_TYPES = ['string', 'number', 'bool', 'Json', 'JsonObject'] as const;

export function defaultConfigSourceSpecs(profile?: string): ConfigSourceSpec[] {
  const specs: ConfigSourceSpec[] = [
    { path: 'config.yml', label: 'config.yml', sourceClass: 'bundle' }
  ];
  if (profile !== undefined && profile.length > 0) {
    assertPathSegment(profile, `profile ${profile}`);
    specs.push({
      path: `config.${profile}.yml`,
      label: `config.${profile}.yml`,
      sourceClass: 'bundle'
    });
    specs.push({
      path: `config.${profile}.secret.yml`,
      label: `config.${profile}.secret.yml`,
      sourceClass: 'secret'
    });
  }
  return specs;
}

export function parseConfigYamlSource(
  text: string,
  metadata: { label: string; sourceClass: ConfigSourceClass }
): ConfigSource {
  const document = parseDocument(text, {
    uniqueKeys: true,
    merge: false,
    schema: 'core',
    prettyErrors: false
  });
  const parseProblems = [...document.errors, ...document.warnings];
  if (parseProblems.length > 0) {
    const message = parseProblems[0]?.message ?? 'invalid YAML';
    throw new Error(`${metadata.label} config YAML parse error: ${normalizeYamlError(message)}`);
  }
  if (document.contents === null) {
    throw new Error(`${metadata.label} config root must be an object`);
  }
  rejectUnsupportedYamlNodeFeatures(document.contents, metadata.label);
  if (!isMap(document.contents)) {
    throw new Error(`${metadata.label} config root must be an object`);
  }
  const value = yamlMapToJsonObject(document.contents, metadata.label, []);
  return {
    sourceClass: metadata.sourceClass,
    label: metadata.label,
    value
  };
}

export function readConfigShape(value: unknown, label: string): ConfigShape {
  if (value === undefined || value === null) {
    return emptyConfigShape();
  }
  if (!isRecord(value)) {
    throw new Error(`${label} must be an object`);
  }
  if (value.schemaVersion !== 'skiff-config-shape-v1') {
    throw new Error(`${label}.schemaVersion must be skiff-config-shape-v1`);
  }
  if (!Array.isArray(value.entries)) {
    throw new Error(`${label}.entries must be an array`);
  }
  return {
    schemaVersion: 'skiff-config-shape-v1',
    entries: validateConfigShapeEntries(value.entries, `${label}.entries`)
  };
}

export function emptyConfigShape(): ConfigShape {
  return {
    schemaVersion: 'skiff-config-shape-v1',
    entries: []
  };
}

export function validateConfigShapeEntries(
  entries: readonly unknown[],
  label: string
): NormalizedConfigShapeEntry[] {
  const byPath = new Map<string, NormalizedConfigShapeEntry>();
  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index];
    const entryLabel = `${label}[${index}]`;
    if (!isRecord(entry)) {
      throw new Error(`${entryLabel} must be an object`);
    }
    if (typeof entry.path !== 'string') {
      throw new Error(`${entryLabel}.path must be a string`);
    }
    if (typeof entry.type !== 'string' || !isConfigShapeValueType(entry.type)) {
      throw new Error(`${entryLabel} ${entry.path} type must be string, number, bool, Json, or JsonObject`);
    }
    if (
      Object.prototype.hasOwnProperty.call(entry, 'required') &&
      typeof entry.required !== 'boolean'
    ) {
      throw new Error(`${entryLabel} ${entry.path} required must be a boolean`);
    }
    if (Object.prototype.hasOwnProperty.call(entry, 'distribution')) {
      throw new Error(`${entryLabel} ${entry.path} must not declare distribution`);
    }
    if (Object.prototype.hasOwnProperty.call(entry, 'redact')) {
      throw new Error(`${entryLabel} ${entry.path} must not declare redact`);
    }
    validateDottedPath(entry.path, `${entryLabel} ${entry.path}`);
    const normalized: NormalizedConfigShapeEntry = {
      path: entry.path,
      type: entry.type,
      required: entry.required === true
    };
    const existing = byPath.get(entry.path);
    if (existing === undefined) {
      byPath.set(entry.path, normalized);
      continue;
    }
    if (existing.type !== normalized.type || existing.required !== normalized.required) {
      throw new Error(`${label} conflicting configShape entry for ${entry.path}`);
    }
  }
  return Array.from(byPath.values()).sort((left, right) => left.path.localeCompare(right.path));
}

export function buildResolvedConfig(input: {
  configShape: readonly ConfigShapeEntry[] | readonly NormalizedConfigShapeEntry[];
  sources: readonly ConfigSource[];
}): ResolvedConfig {
  const configShape = validateConfigShapeEntries(input.configShape, 'configShape');
  const resolvedConfig: JsonObject = {};
  const provenance: ResolvedConfig['provenance'] = {
    leaves: {},
    tombstones: {}
  };

  for (const source of input.sources) {
    overlaySource(resolvedConfig, provenance, source.value, source, []);
  }

  validateFinalResolvedConfig(resolvedConfig, configShape);
  const redactedResolvedConfig = redactResolvedConfig(resolvedConfig, provenance);
  const redactionProjectionIdentity = `skiff-config-redaction-v1:sha256:${sha256Hex(
    stableStringify({
      configShape,
      redactedResolvedConfig,
      provenance
    })
  )}`;
  return {
    resolvedConfig,
    redactedResolvedConfig,
    provenance,
    redactionProjectionIdentity
  };
}

function normalizeYamlError(message: string): string {
  return message.replace(/Map keys must be unique/i, 'duplicate key').split('\n')[0] ?? message;
}

function rejectUnsupportedYamlNodeFeatures(node: Node, label: string): void {
  const stack: Node[] = [node];
  while (stack.length > 0) {
    const current = stack.pop()!;
    if (isAlias(current)) {
      throw new Error(`${label} config YAML aliases are not supported`);
    }
    if (current.anchor !== undefined) {
      throw new Error(`${label} config YAML anchors are not supported`);
    }
    if (current.tag !== undefined) {
      throw new Error(`${label} config YAML tags are not supported`);
    }
    if (isMap(current)) {
      for (const item of current.items) {
        if (item.key) {
          stack.push(item.key as Node);
        }
        if (item.value) {
          stack.push(item.value as Node);
        }
      }
      continue;
    }
    if (isSeq(current)) {
      for (const item of current.items) {
        if (item) {
          stack.push(item as Node);
        }
      }
    }
  }
}

function yamlMapToJsonObject(map: Node, label: string, path: string[]): JsonObject {
  if (!isMap(map)) {
    throw new Error(`${label} config ${formatPath(path)} must be an object`);
  }
  const object: JsonObject = {};
  for (const item of map.items) {
    if (!isScalar(item.key) || typeof item.key.value !== 'string') {
      throw new Error(`${label} config key at ${formatPath(path)} must be a string`);
    }
    const key = item.key.value;
    const keySegments = normalizeConfigKeySegments(key, label, path);
    const nextPath = [...path, ...keySegments];
    const value =
      item.value === null ? null : yamlNodeToJsonValue(item.value as Node, label, nextPath);
    insertConfigPathValue(object, keySegments, value, label, nextPath);
  }
  return object;
}

function normalizeConfigKeySegments(key: string, label: string, path: string[]): string[] {
  if (key.length === 0) {
    throw new Error(`${label} invalid config key ${formatPath(path)}`);
  }
  if (key.includes('.')) {
    throw new Error(`${label} invalid config key ${[...path, key].join('.')}: dotted YAML keys are not supported`);
  }
  assertPathSegment(key, `${label} invalid config key ${[...path, key].join('.')}`);
  return [key];
}

function insertConfigPathValue(
  object: JsonObject,
  segments: readonly string[],
  value: JsonValue,
  label: string,
  fullPath: readonly string[]
): void {
  let cursor = object;
  for (let index = 0; index < segments.length - 1; index += 1) {
    const segment = segments[index]!;
    const currentPath = fullPath.slice(0, index + 1);
    const existing = cursor[segment];
    if (existing === undefined) {
      const next: JsonObject = {};
      cursor[segment] = next;
      cursor = next;
      continue;
    }
    if (!isJsonObject(existing) || Array.isArray(existing)) {
      throw new Error(
        `${label} config ${formatPath(currentPath)} cannot be both a value and an object parent`
      );
    }
    cursor = existing;
  }

  const key = segments[segments.length - 1]!;
  const existing = cursor[key];
  if (existing === undefined) {
    cursor[key] = value;
    return;
  }
  if (stableStringify(existing) === stableStringify(value)) {
    return;
  }
  throw new Error(`${label} config ${formatPath([...fullPath])} conflicts with another config key`);
}

function yamlNodeToJsonValue(node: Node, label: string, path: string[]): JsonValue {
  if (isMap(node)) {
    return yamlMapToJsonObject(node, label, path);
  }
  if (isSeq(node)) {
    return node.items.map((item, index) => {
      if (item === null) {
        return null;
      }
      return yamlNodeToJsonValue(item as Node, label, [...path, String(index)]);
    });
  }
  if (isScalar(node)) {
    const value = node.value;
    if (
      value === null ||
      typeof value === 'string' ||
      typeof value === 'boolean' ||
      (typeof value === 'number' && Number.isFinite(value))
    ) {
      return value;
    }
    throw new Error(`${label} config ${formatPath(path)} must be JSON-compatible`);
  }
  throw new Error(`${label} config ${formatPath(path)} must be JSON-compatible`);
}

function validateDottedPath(path: string, label: string): void {
  const segments = path.split('.');
  if (segments.length === 0 || path.length === 0) {
    throw new Error(`${label} path must not be empty`);
  }
  for (const segment of segments) {
    assertPathSegment(segment, `${label} invalid path segment ${segment}`);
  }
}

function assertPathSegment(segment: string, label: string): void {
  if (!CONFIG_KEY_PATTERN.test(segment)) {
    throw new Error(`${label}`);
  }
}

function overlaySource(
  target: JsonObject,
  provenance: ResolvedConfig['provenance'],
  value: JsonObject,
  source: ConfigSource,
  path: string[]
): void {
  for (const [key, child] of Object.entries(value)) {
    const nextPath = [...path, key];
    const pathString = nextPath.join('.');
    if (child === null) {
      delete target[key];
      clearProvenanceSubtree(provenance.leaves, pathString);
      provenance.tombstones[pathString] = {
        sourceClass: source.sourceClass,
        label: source.label
      };
      continue;
    }
    if (isJsonObject(child) && !Array.isArray(child)) {
      if (!isJsonObject(target[key]) || Array.isArray(target[key])) {
        clearProvenanceSubtree(provenance.leaves, pathString);
        clearProvenanceSubtree(provenance.tombstones, pathString);
        target[key] = {};
      }
      overlaySource(target[key] as JsonObject, provenance, child, source, nextPath);
      continue;
    }
    clearProvenanceSubtree(provenance.leaves, pathString);
    clearProvenanceSubtree(provenance.tombstones, pathString);
    target[key] = cloneJson(child);
    provenance.leaves[pathString] = {
      sourceClass: source.sourceClass,
      label: source.label
    };
  }
}

function clearProvenanceSubtree(
  entries: Record<string, ConfigProvenanceEntry>,
  path: string
): void {
  const prefix = `${path}.`;
  for (const key of Object.keys(entries)) {
    if (key === path || key.startsWith(prefix)) {
      delete entries[key];
    }
  }
}

function validateFinalResolvedConfig(
  resolvedConfig: JsonObject,
  configShape: readonly NormalizedConfigShapeEntry[]
): void {
  for (const entry of configShape) {
    const value = getPathValue(resolvedConfig, entry.path);
    if (value === undefined || value === null) {
      if (entry.required) {
        throw new Error(`final resolvedConfig ${entry.path} is required`);
      }
      continue;
    }
    if (!matchesConfigShapeType(value, entry.type)) {
      throw new Error(`final resolvedConfig ${entry.path} must be ${entry.type}`);
    }
  }
}

function matchesConfigShapeType(value: JsonValue, type: ConfigShapeValueType): boolean {
  switch (type) {
    case 'string':
      return typeof value === 'string';
    case 'number':
      return typeof value === 'number' && Number.isFinite(value);
    case 'bool':
      return typeof value === 'boolean';
    case 'Json':
      return true;
    case 'JsonObject':
      return isJsonObject(value) && !Array.isArray(value);
  }
}

function redactResolvedConfig(
  resolvedConfig: JsonObject,
  provenance: ResolvedConfig['provenance']
): JsonObject {
  const redactValue = (value: JsonValue, path: string[]): JsonValue => {
    if (Array.isArray(value)) {
      return shouldRedact(path.join('.'), provenance) ? '[REDACTED]' : cloneJson(value);
    }
    if (isJsonObject(value)) {
      const redacted: JsonObject = {};
      for (const [key, child] of Object.entries(value)) {
        redacted[key] = redactValue(child, [...path, key]);
      }
      return redacted;
    }
    return shouldRedact(path.join('.'), provenance) ? '[REDACTED]' : value;
  };
  return redactValue(resolvedConfig, []) as JsonObject;
}

function shouldRedact(path: string, provenance: ResolvedConfig['provenance']): boolean {
  return provenance.leaves[path]?.sourceClass === 'secret';
}

function getPathValue(object: JsonObject, path: string): JsonValue | undefined {
  let value: JsonValue | undefined = object;
  for (const segment of path.split('.')) {
    if (!isJsonObject(value) || Array.isArray(value)) {
      return undefined;
    }
    value = value[segment];
  }
  return value;
}

export function isConfigShapeValueType(value: string): value is ConfigShapeValueType {
  return (CONFIG_SHAPE_VALUE_TYPES as readonly string[]).includes(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function cloneJson<T extends JsonValue>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function formatPath(path: string[]): string {
  return path.length === 0 ? '<root>' : path.join('.');
}
