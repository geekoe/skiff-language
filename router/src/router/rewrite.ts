import type { IncomingMessage } from 'node:http';

import { isPublicationId } from '../publicationId.js';

export interface RouterRewriteRule {
  host: string;
  path?: string;
  service: string;
  version?: string;
}

export interface RouterRewriteMatch {
  service: string;
  version?: string;
}

interface ReadRewriteRulesOptions {
  configName?: string;
}

export function readRewriteRules(
  value: unknown,
  options: ReadRewriteRulesOptions = {}
): RouterRewriteRule[] {
  const name = options.configName ?? 'rewrite';
  if (value === undefined || value === null) {
    return [];
  }
  if (!Array.isArray(value)) {
    throw new Error(`router config ${name} must be an array`);
  }

  const rules: RouterRewriteRule[] = [];
  const seen = new Set<string>();
  for (let index = 0; index < value.length; index += 1) {
    const rawRule = value[index];
    if (!isRecord(rawRule)) {
      throw new Error(`router config ${name}[${index}] must be an object`);
    }
    rejectUnknownFields(rawRule, `${name}[${index}]`, new Set(['host', 'path', 'service', 'version']));
    const host = normalizeHost(readRequiredString(rawRule.host, `${name}[${index}].host`));
    if (!host) {
      throw new Error(`router config ${name}[${index}].host must be a non-empty host`);
    }
    const path = readOptionalPath(rawRule.path, `${name}[${index}].path`);
    const service = readServiceId(rawRule.service, `${name}[${index}].service`);
    const version = readOptionalVersion(rawRule.version, `${name}[${index}].version`);
    const key = rewriteRuleKey(host, path);
    if (seen.has(key)) {
      throw new Error(
        path === undefined
          ? `duplicate router rewrite rule for host ${host}`
          : `duplicate router rewrite rule for host ${host} path ${path}`
      );
    }
    seen.add(key);
    rules.push({
      host,
      ...(path !== undefined ? { path } : {}),
      service,
      ...(version !== undefined ? { version } : {})
    });
  }
  return rules;
}

export function resolveRewrite(
  rules: readonly RouterRewriteRule[] | undefined,
  input: { host?: string | string[]; pathname: string }
): RouterRewriteMatch | undefined {
  if (!rules || rules.length === 0) {
    return undefined;
  }
  const hostHeader = firstHeader(input.host);
  if (!hostHeader) {
    return undefined;
  }
  const host = normalizeHost(hostHeader);
  if (!host) {
    return undefined;
  }
  const exactPathRule = rules.find(
    (rule) =>
      normalizeHost(rule.host) === host &&
      rule.path !== undefined &&
      rule.path === input.pathname
  );
  const fallbackRule =
    exactPathRule ??
    rules.find((rule) => normalizeHost(rule.host) === host && rule.path === undefined);
  if (!fallbackRule) {
    return undefined;
  }
  return {
    service: fallbackRule.service,
    ...(fallbackRule.version !== undefined ? { version: fallbackRule.version } : {})
  };
}

export function resolveRequestRewrite(
  rules: readonly RouterRewriteRule[] | undefined,
  request: IncomingMessage,
  url: URL
): RouterRewriteMatch | undefined {
  const input: { host?: string | string[]; pathname: string } = {
    pathname: url.pathname
  };
  if (request.headers.host !== undefined) {
    input.host = request.headers.host;
  }
  return resolveRewrite(rules, input);
}

export function normalizeHost(value: string): string {
  const withoutPort = value.trim().toLowerCase().replace(/\.$/, '');
  if (withoutPort.startsWith('[')) {
    const closingBracket = withoutPort.indexOf(']');
    return closingBracket === -1 ? withoutPort : withoutPort.slice(0, closingBracket + 1);
  }
  return withoutPort.split(':')[0] ?? '';
}

function readRequiredString(value: unknown, name: string): string {
  const text = readOptionalString(value, name);
  if (text === undefined) {
    throw new Error(`router config ${name} is required`);
  }
  return text;
}

function readOptionalString(value: unknown, name: string): string | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`router config ${name} must be a non-empty string`);
  }
  return value.trim();
}

function readOptionalPath(value: unknown, name: string): string | undefined {
  const path = readOptionalString(value, name);
  if (path === undefined) {
    return undefined;
  }
  if (!path.startsWith('/')) {
    throw new Error(`router config ${name} must start with /`);
  }
  return path;
}

function readServiceId(value: unknown, name: string): string {
  const service = readRequiredString(value, name);
  if (!isPublicationId(service)) {
    throw new Error(`router config ${name} must be a valid publication id`);
  }
  return service;
}

function readOptionalVersion(value: unknown, name: string): string | undefined {
  const version = readOptionalString(value, name);
  if (version === undefined) {
    return undefined;
  }
  if (!/^[A-Za-z0-9._:-]+$/.test(version)) {
    throw new Error(`router config ${name} must be a valid version`);
  }
  return version;
}

function rejectUnknownFields(
  value: Record<string, unknown>,
  name: string,
  allowedFields: ReadonlySet<string>
): void {
  for (const field of Object.keys(value)) {
    if (!allowedFields.has(field)) {
      throw new Error(`router config ${name}.${field} is not supported`);
    }
  }
}

function rewriteRuleKey(host: string, path: string | undefined): string {
  return `${host}\0${path ?? ''}`;
}

function firstHeader(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
