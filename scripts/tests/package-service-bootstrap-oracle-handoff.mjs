import assert from 'node:assert/strict';
import {
  mkdtemp,
  rm,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { captureCheckedCommand } from '../lib/command-execution.mjs';
import { bootstrapCanonicalArgs } from '../lib/isolated-test-runtime-instance.mjs';
import {
  validatePackageServiceBootstrapReceipt,
} from '../lib/package-service-ecosystem-smoke-oracle.mjs';

const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('actual locked bootstrap receipt crosses the production JavaScript oracle', async (t) => {
  const workRoot = await mkdtemp(join(tmpdir(), 'skiff-p5-f28a-bootstrap-oracle-'));
  t.after(() => rm(workRoot, { recursive: true, force: true }));
  const environment = 'f28a-bootstrap-oracle-handoff';
  const outcome = await captureCheckedCommand(
    'cargo',
    bootstrapCanonicalArgs({
      skiffRoot: checkout,
      artifactRoot: join(workRoot, 'artifacts'),
      environment,
    }),
    { cwd: checkout },
  );
  const receipt = JSON.parse(outcome.stdout);
  assert.strictEqual(
    validatePackageServiceBootstrapReceipt(receipt, environment),
    receipt,
  );
});
