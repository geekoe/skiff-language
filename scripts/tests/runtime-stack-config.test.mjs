import assert from 'node:assert/strict';
import { test } from 'node:test';

import { renderRouterConfig } from '../lib/runtime-stack-config.mjs';

const routerConfig = {
  profile: 'dev',
  host: '127.0.0.1',
  environment: 'f04-host-test',
  artifactRoots: ['/tmp/skiff/artifacts'],
  identityCliPath: '/tmp/skiff/bin/artifact-identity',
  devReload: true,
  releaseMode: false,
  httpPort: 4100,
  runtimePort: 4101,
};

test('router config renders an explicit environment', () => {
  const rendered = renderRouterConfig(routerConfig);

  assert.match(rendered, /^environment: "f04-host-test"$/m);
  assert.equal(rendered.match(/^environment:/gm)?.length, 1);
});

test('router config fails closed when environment is omitted or empty', () => {
  const { environment: _environment, ...withoutEnvironment } = routerConfig;
  assert.throws(
    () => renderRouterConfig(withoutEnvironment),
    /router environment is required/,
  );
  assert.throws(
    () => renderRouterConfig({ ...routerConfig, environment: '' }),
    /router environment is required/,
  );
});
