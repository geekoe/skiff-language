const RELOAD_PATH = '/__skiff/reload-artifacts';
const REDACTED_TARGET = '<redacted-runtime-reload-target>';

export class RuntimeReloadUrlError extends Error {
  constructor(reason) {
    super(`invalid runtime reload URL (${reason}; target=${REDACTED_TARGET})`);
    this.name = 'RuntimeReloadUrlError';
    this.code = reason;
  }
}

export function parseRuntimeReloadUrl(value) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new RuntimeReloadUrlError('reload_url_empty');
  }
  if (value.trim() !== value || /\s/.test(value)) {
    throw new RuntimeReloadUrlError('reload_url_format');
  }
  if (!value.startsWith('http://')) {
    throw new RuntimeReloadUrlError('reload_url_scheme');
  }

  const remainder = value.slice('http://'.length);
  if (remainder.includes('#')) {
    throw new RuntimeReloadUrlError('reload_url_fragment');
  }
  if (remainder.includes('?')) {
    throw new RuntimeReloadUrlError('reload_url_query');
  }

  const slashIndex = remainder.indexOf('/');
  const authority = slashIndex === -1 ? remainder : remainder.slice(0, slashIndex);
  const path = slashIndex === -1 ? '' : remainder.slice(slashIndex);
  if (authority.includes('@')) {
    throw new RuntimeReloadUrlError('reload_url_userinfo');
  }
  if (authority.includes('[') || authority.includes(']') || colonCount(authority) > 1) {
    throw new RuntimeReloadUrlError('reload_url_ipv6');
  }

  const colonIndex = authority.lastIndexOf(':');
  if (colonIndex === -1) {
    throw new RuntimeReloadUrlError('reload_url_port');
  }
  const host = authority.slice(0, colonIndex).toLowerCase();
  const rawPort = authority.slice(colonIndex + 1);
  if (!validRuntimeReloadHost(host)) {
    throw new RuntimeReloadUrlError('reload_url_host');
  }
  if (!/^\d+$/.test(rawPort)) {
    throw new RuntimeReloadUrlError('reload_url_port');
  }
  const port = Number(rawPort);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65_535) {
    throw new RuntimeReloadUrlError('reload_url_port');
  }
  if (path !== '' && path !== '/' && path !== RELOAD_PATH) {
    throw new RuntimeReloadUrlError('reload_url_path');
  }

  const display = `http://${host}:${port}`;
  return {
    baseUrl: display,
    display,
    normalized: `${display}${RELOAD_PATH}`,
  };
}

function validRuntimeReloadHost(host) {
  if (host.length === 0 || host.length > 253 || !/^[a-z0-9.-]+$/.test(host)) {
    return false;
  }
  if (/^[0-9.]+$/.test(host)) {
    const octets = host.split('.');
    return octets.length === 4
      && octets.every((octet) => /^(0|[1-9]\d{0,2})$/.test(octet) && Number(octet) <= 255);
  }
  return host.split('.').every((label) => label.length > 0
    && label.length <= 63
    && /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(label));
}

function colonCount(value) {
  return [...value].filter((character) => character === ':').length;
}
