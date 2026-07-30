#!/usr/bin/env node

import http from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const SELECTOR_HEADERS = new Set(['x-skiff-service', 'x-skiff-version']);
const STATIC_HOP_BY_HOP_HEADERS = new Set([
  'connection',
  'keep-alive',
  'proxy-authenticate',
  'proxy-authorization',
  'proxy-connection',
  'te',
  'trailer',
  'transfer-encoding',
  'upgrade',
]);
const HEALTH_PATH = '/__local_ingress/health';

export function parseLocalIngressArgs(argv, environment = process.env) {
  let configPath = environment.SKIFF_LOCAL_INGRESS_CONFIG;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--config') {
      configPath = argv[++index];
      if (!configPath) throw new Error('--config requires a path');
      continue;
    }
    if (argument.startsWith('--config=')) {
      configPath = argument.slice('--config='.length);
      if (!configPath) throw new Error('--config requires a path');
      continue;
    }
    throw new Error(`Unknown argument: ${argument}`);
  }
  if (!configPath) {
    throw new Error(
      'local ingress requires --config <path> or SKIFF_LOCAL_INGRESS_CONFIG',
    );
  }
  return { configPath: path.resolve(configPath) };
}

export async function loadLocalIngressConfig(configPath) {
  const document = JSON.parse(await readFile(configPath, 'utf8'));
  return validateLocalIngressConfig(document);
}

export function validateLocalIngressConfig(document) {
  if (!document || typeof document !== 'object' || Array.isArray(document)) {
    throw new Error('local ingress config must be an object');
  }
  const listen = validateEndpoint(document.listen, 'listen', { allowZeroPort: true });
  const upstream = validateEndpoint(document.upstream, 'upstream');
  if (!document.hosts || typeof document.hosts !== 'object' || Array.isArray(document.hosts)) {
    throw new Error('hosts must be an object');
  }
  const hosts = new Map();
  for (const [inputHost, inputTarget] of Object.entries(document.hosts)) {
    const host = normalizeConfiguredHost(inputHost);
    if (host !== inputHost) {
      throw new Error(
        `configured Host must already be canonical lowercase without a port: ${inputHost}`,
      );
    }
    if (hosts.has(host)) throw new Error(`duplicate configured Host: ${host}`);
    if (!inputTarget || typeof inputTarget !== 'object' || Array.isArray(inputTarget)) {
      throw new Error(`Host ${host} target must be an object`);
    }
    const service = validateCanonicalToken(inputTarget.service, `Host ${host} service`);
    const version = validateCanonicalToken(inputTarget.version, `Host ${host} version`);
    hosts.set(host, Object.freeze({ service, version }));
  }
  if (hosts.size === 0) throw new Error('hosts must contain at least one mapping');
  return Object.freeze({ listen, upstream, hosts });
}

function validateEndpoint(input, label, { allowZeroPort = false } = {}) {
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error(`${label} must be an object`);
  }
  const host = validateCanonicalToken(input.host, `${label}.host`);
  const minimumPort = allowZeroPort ? 0 : 1;
  if (!Number.isSafeInteger(input.port) || input.port < minimumPort || input.port > 65_535) {
    throw new Error(`${label}.port must be an integer from ${minimumPort} to 65535`);
  }
  return Object.freeze({ host, port: input.port });
}

function validateCanonicalToken(input, label) {
  if (
    typeof input !== 'string'
    || input.length === 0
    || input !== input.trim()
    || /[\s\p{Cc}]/u.test(input)
  ) {
    throw new Error(`${label} must be a non-empty canonical token`);
  }
  return input;
}

function normalizeConfiguredHost(input) {
  if (
    typeof input !== 'string'
    || input.length === 0
    || input !== input.trim()
    || /[\s\p{Cc}:\/@,\[\]]/u.test(input)
  ) {
    throw new Error(`invalid configured Host: ${input}`);
  }
  return input.toLowerCase();
}

export function normalizeLocalIngressHost(input) {
  if (
    typeof input !== 'string'
    || input.length === 0
    || input !== input.trim()
    || /[\s\p{Cc}\/@,]/u.test(input)
  ) {
    return null;
  }
  if (input.startsWith('[')) {
    const close = input.indexOf(']');
    if (close <= 1) return null;
    const suffix = input.slice(close + 1);
    if (suffix && !/^:\d+$/u.test(suffix)) return null;
    return input.slice(1, close).toLowerCase();
  }
  const colon = input.lastIndexOf(':');
  if (colon >= 0) {
    if (input.indexOf(':') !== colon || !/^\d+$/u.test(input.slice(colon + 1))) return null;
    input = input.slice(0, colon);
  }
  if (!input || /[\[\]]/u.test(input)) return null;
  return input.toLowerCase();
}

function connectionHeaderNames(rawHeaders) {
  const names = new Set();
  for (let index = 0; index + 1 < rawHeaders.length; index += 2) {
    if (rawHeaders[index].toLowerCase() !== 'connection') continue;
    for (const token of rawHeaders[index + 1].split(',')) {
      const name = token.trim().toLowerCase();
      if (name) names.add(name);
    }
  }
  return names;
}

function sanitizeRawHeaders(rawHeaders, { upgrade = false, target } = {}) {
  const connectionNames = connectionHeaderNames(rawHeaders);
  const output = [];
  let upgradeValue;
  for (let index = 0; index + 1 < rawHeaders.length; index += 2) {
    const name = rawHeaders[index];
    const value = rawHeaders[index + 1];
    const lowerName = name.toLowerCase();
    if (SELECTOR_HEADERS.has(lowerName)) continue;
    if (lowerName === 'upgrade') upgradeValue ??= value;
    if (STATIC_HOP_BY_HOP_HEADERS.has(lowerName) || connectionNames.has(lowerName)) continue;
    output.push(name, value);
  }
  if (target) {
    output.push('x-skiff-service', target.service);
    output.push('x-skiff-version', target.version);
  }
  if (upgrade) {
    if (!upgradeValue) throw new Error('upgrade message is missing the Upgrade header');
    output.push('Connection', 'Upgrade');
    output.push('Upgrade', upgradeValue);
  }
  return output;
}

function requestOptions(request, config, target, { upgrade = false } = {}) {
  return {
    host: config.upstream.host,
    port: config.upstream.port,
    method: request.method,
    path: request.url,
    headers: sanitizeRawHeaders(request.rawHeaders, { upgrade, target }),
    agent: false,
  };
}

function writeJson(response, statusCode, body) {
  const payload = Buffer.from(`${JSON.stringify(body)}\n`);
  response.writeHead(statusCode, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': payload.length,
    'cache-control': 'no-store',
  });
  response.end(payload);
}

function targetForRequest(request, config) {
  const host = normalizeLocalIngressHost(request.headers.host);
  return host ? config.hosts.get(host) : undefined;
}

function proxyHttp(request, response, config, target) {
  const upstreamRequest = http.request(
    requestOptions(request, config, target),
    (upstreamResponse) => {
      response.writeHead(
        upstreamResponse.statusCode ?? 502,
        upstreamResponse.statusMessage,
        sanitizeRawHeaders(upstreamResponse.rawHeaders),
      );
      upstreamResponse.pipe(response);
      upstreamResponse.once('error', (error) => {
        if (!response.destroyed) response.destroy(error);
      });
      response.once('close', () => {
        if (!response.writableFinished) upstreamResponse.destroy();
      });
    },
  );

  upstreamRequest.once('error', () => {
    if (!response.headersSent) {
      writeJson(response, 502, { error: 'local ingress upstream unavailable' });
    } else if (!response.destroyed) {
      response.destroy();
    }
  });
  request.once('aborted', () => upstreamRequest.destroy());
  response.once('close', () => {
    if (!response.writableFinished) upstreamRequest.destroy();
  });
  request.pipe(upstreamRequest);
}

function socketResponse(socket, statusCode, reason, body) {
  if (socket.destroyed) return;
  const payload = Buffer.from(`${body}\n`);
  socket.end(
    `HTTP/1.1 ${statusCode} ${reason}\r\n`
    + 'Content-Type: text/plain; charset=utf-8\r\n'
    + `Content-Length: ${payload.length}\r\n`
    + 'Connection: close\r\n'
    + '\r\n',
    payload,
  );
}

function serializedHeaders(rawHeaders) {
  let output = '';
  for (let index = 0; index + 1 < rawHeaders.length; index += 2) {
    output += `${rawHeaders[index]}: ${rawHeaders[index + 1]}\r\n`;
  }
  return output;
}

function writeRawResponseHead(socket, response, rawHeaders) {
  socket.write(
    `HTTP/${response.httpVersion} ${response.statusCode} ${response.statusMessage}\r\n`
    + serializedHeaders(rawHeaders)
    + '\r\n',
  );
}

function coupleSockets(left, right) {
  const destroyBoth = (error) => {
    if (!left.destroyed) left.destroy(error);
    if (!right.destroyed) right.destroy(error);
  };
  left.once('error', destroyBoth);
  right.once('error', destroyBoth);
  left.once('close', () => {
    if (!right.destroyed) right.destroy();
  });
  right.once('close', () => {
    if (!left.destroyed) left.destroy();
  });
  left.pipe(right);
  right.pipe(left);
}

function proxyUpgrade(request, clientSocket, clientHead, config, target) {
  let upstreamRequest;
  try {
    upstreamRequest = http.request(requestOptions(request, config, target, { upgrade: true }));
  } catch {
    socketResponse(clientSocket, 400, 'Bad Request', 'invalid upgrade request');
    return;
  }

  upstreamRequest.once('upgrade', (upstreamResponse, upstreamSocket, upstreamHead) => {
    writeRawResponseHead(
      clientSocket,
      upstreamResponse,
      sanitizeRawHeaders(upstreamResponse.rawHeaders, { upgrade: true }),
    );
    if (clientHead.length > 0) upstreamSocket.write(clientHead);
    if (upstreamHead.length > 0) clientSocket.write(upstreamHead);
    coupleSockets(clientSocket, upstreamSocket);
  });
  upstreamRequest.once('response', (upstreamResponse) => {
    const headers = sanitizeRawHeaders(upstreamResponse.rawHeaders);
    headers.push('Connection', 'close');
    writeRawResponseHead(clientSocket, upstreamResponse, headers);
    upstreamResponse.pipe(clientSocket);
    upstreamResponse.once('error', () => clientSocket.destroy());
  });
  upstreamRequest.once('error', () => {
    socketResponse(clientSocket, 502, 'Bad Gateway', 'local ingress upstream unavailable');
  });
  clientSocket.once('close', () => upstreamRequest.destroy());
  upstreamRequest.end();
}

export function createLocalIngress(configInput) {
  const config = configInput.hosts instanceof Map
    ? configInput
    : validateLocalIngressConfig(configInput);
  const server = http.createServer((request, response) => {
    if (request.method === 'GET' && request.url === HEALTH_PATH) {
      writeJson(response, 200, { status: 'ok' });
      return;
    }
    const target = targetForRequest(request, config);
    if (!target) {
      writeJson(response, 421, { error: 'unknown local ingress Host' });
      return;
    }
    proxyHttp(request, response, config, target);
  });
  server.on('upgrade', (request, socket, head) => {
    const target = targetForRequest(request, config);
    if (!target) {
      socketResponse(socket, 421, 'Misdirected Request', 'unknown local ingress Host');
      return;
    }
    proxyUpgrade(request, socket, head, config, target);
  });
  return server;
}

export async function startLocalIngress(config) {
  const server = createLocalIngress(config);
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(config.listen.port, config.listen.host, () => {
      server.off('error', reject);
      resolve();
    });
  });
  return server;
}

async function main() {
  const { configPath } = parseLocalIngressArgs(process.argv.slice(2));
  const config = await loadLocalIngressConfig(configPath);
  const server = await startLocalIngress(config);
  const address = server.address();
  process.stdout.write(
    `[skiff-local-ingress] listening http://${address.address}:${address.port} `
    + `upstream=http://${config.upstream.host}:${config.upstream.port} `
    + `hosts=${config.hosts.size}\n`,
  );
}

if (import.meta.url === pathToFileURL(process.argv[1] || '').href) {
  main().catch((error) => {
    process.stderr.write(`[skiff-local-ingress] ${error.stack || error.message}\n`);
    process.exitCode = 1;
  });
}
