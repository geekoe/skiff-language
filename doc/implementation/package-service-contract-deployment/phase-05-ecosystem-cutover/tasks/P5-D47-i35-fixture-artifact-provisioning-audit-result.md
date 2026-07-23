# P5-D47：I35 Fixture Artifact Provisioning Audit Result

结论：COMPLETE。已有canonical hermetic seed入口，无需新增实现节点。

`skiff test --artifact-root`只从显式root解析`skiff.run/std@1.0.0` immutable PackageArtifact；isolated runtime的
自动seed root与source dependency root强制分离。正确入口是
`bootstrapCanonicalArgs()`调用locked `skiff-package-service-smoke-fixture --bootstrap-only`，内部以
compiler-owned `author_official_std_package()`从current-checkout validated official std source author，并经唯一
PackageArtifact writer、immutable record与CAS pointer发布。

第三次命令必须在全新空`/tmp/skiff-p5-i35-source-artifacts.*`先执行canonical bootstrap，再运行带
`--deny-skips --require-tests`的fixture test，并用owned command、10分钟deadline、30秒cleanup删除复核。不得使用generic
package publish、stable root/config/watch或4000/4001。

唯一完整命令如下，后续复验逐字执行，不重复I35其它证据：

```bash
node --input-type=module - <<'NODE'
import assert from 'node:assert/strict';
import { mkdtemp, readdir, rm, stat } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const checkout = resolve(process.cwd());
assert.equal(checkout, '/Users/geek/workspace/skiff-phase-05-integration');
const [{ runOwnedCommand }, { bootstrapCanonicalArgs }] = await Promise.all([
  import(pathToFileURL(join(checkout, 'scripts/lib/owned-command.mjs')).href),
  import(pathToFileURL(join(checkout, 'scripts/lib/isolated-test-runtime-instance.mjs')).href),
]);
const artifactRoot = await mkdtemp('/tmp/skiff-p5-i35-source-artifacts.');
assert.deepEqual(await readdir(artifactRoot), []);
const controller = new AbortController();
const interrupt = (signal) => controller.abort(new Error(`I35 fixture interrupted by ${signal}`));
const handlers = new Map([
  ['SIGINT', () => interrupt('SIGINT')],
  ['SIGTERM', () => interrupt('SIGTERM')],
]);
for (const [signal, handler] of handlers) process.on(signal, handler);
const deadline = setTimeout(
  () => controller.abort(new Error('I35 fixture deadline expired after 600000ms')),
  600_000,
);
const env = {
  ...process.env,
  CARGO_TARGET_DIR: join(checkout, 'build/cargo-target'),
};
for (const key of [
  'SKIFF_DEV_HOME',
  'SKIFF_DEV_RELOAD_URL',
  'SKIFF_TEST_ARTIFACT_ROOT',
  'SKIFF_TEST_RUNTIME_ARTIFACT_ROOT',
  'SKIFF_TEST_ACTIVATION_URL',
  'SKIFF_TEST_INGRESS_URL',
]) delete env[key];
const commandOptions = {
  cwd: checkout,
  env,
  signal: controller.signal,
  stopTimeoutMs: 30_000,
};
try {
  await runOwnedCommand(
    'cargo',
    bootstrapCanonicalArgs({
      skiffRoot: checkout,
      artifactRoot,
      environment: 'skiff-p5-i35-fixture',
    }),
    commandOptions,
  );
  await runOwnedCommand(process.execPath, [
    join(checkout, 'scripts/skiff.mjs'),
    'test',
    join(checkout, 'test-runner/fixtures/package-service-i02-spawn-submit'),
    '--artifact-root',
    artifactRoot,
    '--deny-skips',
    '--require-tests',
  ], commandOptions);
} finally {
  clearTimeout(deadline);
  for (const [signal, handler] of handlers) process.off(signal, handler);
  await rm(artifactRoot, { recursive: true, force: true });
  await assert.rejects(stat(artifactRoot), { code: 'ENOENT' });
}
NODE
```
