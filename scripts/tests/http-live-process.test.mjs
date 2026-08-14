import assert from 'node:assert/strict';
import http from 'node:http';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import test from 'node:test';

import {
  closeLogs,
  createHermeticEgressProxy,
  HTTP_ADMIN_UNSAFE_ENV,
  spawnLoggedProcess,
  unsafeHttpBypassEnvironmentNames,
  withoutUnsafeHttpBypassEnvironment,
} from '../lib/http_live_process.mjs';

test('hermetic egress proxy forwards absolute safe origins and flushes gated response heads', async () => {
  let releaseBody;
  const upstream = http.createServer((_request, response) => {
    response.writeHead(200, { 'content-type': 'application/octet-stream' });
    response.flushHeaders();
    releaseBody = () => response.end('proxied');
  });
  await new Promise((resolvePromise, reject) => {
    upstream.once('error', reject);
    upstream.listen(0, '127.0.0.1', resolvePromise);
  });
  const upstreamAddress = upstream.address();
  assert(upstreamAddress !== null && typeof upstreamAddress === 'object');
  const publicOrigin = 'http://93.184.216.34';
  const proxy = await createHermeticEgressProxy([{
    publicOrigin,
    localPort: upstreamAddress.port,
  }]);
  try {
    const responsePromise = requestThroughProxy(proxy.url, `${publicOrigin}/stream`);
    const response = await Promise.race([responsePromise, delay(500, null)]);
    assert.notEqual(response, null, 'proxy must flush the upstream head before its gated body');
    assert.equal(response.status, 200);
    assert.equal(typeof releaseBody, 'function');
    releaseBody();
    assert.equal(await response.body, 'proxied');
    proxy.assertExactTargets(1);
    assert.deepEqual(proxy.targets, [`${publicOrigin}/stream`]);
  } finally {
    releaseBody?.();
    await proxy.close();
    await new Promise((resolvePromise, reject) => {
      upstream.close((error) => error === undefined ? resolvePromise() : reject(error));
      upstream.closeAllConnections?.();
    });
  }
});

test('HTTP process environment removes every unsafe-target bypass and preserves safe facts', () => {
  const environment = {
    PATH: '/usr/bin:/bin',
    SAFE_FACT: 'kept',
    [HTTP_ADMIN_UNSAFE_ENV]: '1',
    SKIFF_HTTP_ADMIN_BYPASS_SSRF: '1',
    SKIFF_HTTP_ALLOW_PRIVATE_TARGETS: 'true',
    SKIFF_HTTP_ALLOW_LOCAL_TARGETS: 'yes',
  };

  assert.deepEqual(unsafeHttpBypassEnvironmentNames(environment), [
    'SKIFF_HTTP_ADMIN_ALLOW_UNSAFE',
    'SKIFF_HTTP_ADMIN_BYPASS_SSRF',
    'SKIFF_HTTP_ALLOW_LOCAL_TARGETS',
    'SKIFF_HTTP_ALLOW_PRIVATE_TARGETS',
  ]);
  assert.deepEqual(withoutUnsafeHttpBypassEnvironment(environment), {
    PATH: '/usr/bin:/bin',
    SAFE_FACT: 'kept',
  });
  assert.equal(environment[HTTP_ADMIN_UNSAFE_ENV], '1', 'sanitizing must not mutate the caller');
});

test('managed HTTP process receives the sanitized environment', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-http-process-env-'));
  const stdoutPath = join(root, 'stdout.log');
  const stderrPath = join(root, 'stderr.log');
  let processHandle;
  try {
    processHandle = await spawnLoggedProcess(process.execPath, [
      '-e',
      `process.stdout.write(JSON.stringify({
        unsafe: process.env.${HTTP_ADMIN_UNSAFE_ENV},
        alias: process.env.SKIFF_HTTP_ADMIN_BYPASS_SSRF,
        safe: process.env.SAFE_FACT,
      }))`,
    ], {
      cwd: root,
      stdoutPath,
      stderrPath,
      environment: {
        PATH: process.env.PATH,
        SAFE_FACT: 'kept',
        [HTTP_ADMIN_UNSAFE_ENV]: '1',
        SKIFF_HTTP_ADMIN_BYPASS_SSRF: '1',
      },
    });
    const exit = await new Promise((resolvePromise, reject) => {
      processHandle.child.once('error', reject);
      processHandle.child.once('exit', (code, signal) => resolvePromise({ code, signal }));
    });
    assert.deepEqual(exit, { code: 0, signal: null });
    await closeLogs(processHandle);
    processHandle = undefined;
    assert.deepEqual(JSON.parse(await readFile(stdoutPath, 'utf8')), { safe: 'kept' });
    assert.equal(await readFile(stderrPath, 'utf8'), '');
  } finally {
    if (processHandle !== undefined) await closeLogs(processHandle).catch(() => {});
    await rm(root, { recursive: true, force: true });
  }
});

function requestThroughProxy(proxyUrl, target) {
  const proxy = new URL(proxyUrl);
  return new Promise((resolvePromise, reject) => {
    const request = http.request({
      host: proxy.hostname,
      port: proxy.port,
      method: 'GET',
      path: target,
    }, (response) => {
      const chunks = [];
      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      resolvePromise({
        status: response.statusCode,
        body: new Promise((resolveBody) => {
          response.once('end', () => resolveBody(Buffer.concat(chunks).toString('utf8')));
        }),
      });
    });
    request.once('error', reject);
    request.end();
  });
}
