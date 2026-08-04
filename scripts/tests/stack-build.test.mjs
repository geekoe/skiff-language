import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import { buildStack, buildStackInvocation } from '../lib/stack-build.mjs';
import { loadStackConfig } from '../lib/stack-config.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');

test('stack build maps build.yml onto build-runtime-stack args and CARGO_TARGET_DIR', async (t) => {
  const { configDir } = await buildFixture(t);
  const stack = await loadStackConfig(configDir, { skiffRoot, files: ['build.yml'] });
  const invocation = buildStackInvocation({ stack, skiffRoot });

  assert.equal(invocation.command, process.execPath);
  assert.ok(invocation.args[0].endsWith(join('scripts', 'build-runtime-stack.mjs')));
  assert.ok(invocation.args.includes('--target'));
  assert.equal(invocation.args[invocation.args.indexOf('--target') + 1], 'x86_64-unknown-linux-gnu');
  assert.equal(invocation.args[invocation.args.indexOf('--zig-dir') + 1], '/cache/zig');
  assert.equal(
    invocation.args[invocation.args.indexOf('--build-root') + 1],
    join(skiffRoot, 'build', 'runtime-stack'),
  );
  assert.equal(invocation.args[invocation.args.indexOf('--profile') + 1], 'release');
  assert.equal(invocation.args[invocation.args.indexOf('--only') + 1], 'router,runtime');
  assert.equal(invocation.env.CARGO_TARGET_DIR, join(skiffRoot, 'build', 'cargo-target'));
});

test('buildStack runs the mapped invocation with the resolved environment', async (t) => {
  const { configDir } = await buildFixture(t);
  let invoked;
  const result = await buildStack({
    configDir,
    skiffRoot,
    runCommand: async (command, args, options) => {
      invoked = { command, args, options };
    },
  });
  assert.equal(invoked.command, process.execPath);
  assert.equal(invoked.options.env.CARGO_TARGET_DIR, join(skiffRoot, 'build', 'cargo-target'));
  assert.equal(result.buildRoot, join(skiffRoot, 'build', 'runtime-stack'));
});

test('buildStack rejects a missing build.yml without running anything', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-build-missing-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  let invoked = false;
  await assert.rejects(
    buildStack({
      configDir: root,
      skiffRoot,
      runCommand: async () => {
        invoked = true;
      },
    }),
    /build\.yml is required/,
  );
  assert.equal(invoked, false);
});

async function buildFixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-build-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const configDir = join(root, 'configDir');
  await mkdir(configDir, { recursive: true });
  await writeFile(join(configDir, 'build.yml'), [
    'target: x86_64-unknown-linux-gnu',
    'zigDir: /cache/zig',
    'buildRoot: build/runtime-stack',
    'cargoTargetDir: build/cargo-target',
    'units:',
    '  - router',
    '  - runtime',
    '',
  ].join('\n'));
  return { configDir, root };
}
