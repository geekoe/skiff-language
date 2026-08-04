import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import { captureCheckedCommand } from '../lib/command-execution.mjs';

const testDir = resolve(import.meta.dirname);
const skiffRoot = resolve(testDir, '..', '..');
const instanceScript = join(skiffRoot, 'scripts', 'skiff-instance.mjs');

test('skiff instance up/status/down manages processes from instance.yml', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-runtime-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const devHome = join(root, 'dev-home');
  await writeFile(join(root, 'instance.yml'), [
    'schemaVersion: skiff-instance-v1',
    'profile: dev',
    `devHome: ${JSON.stringify(devHome)}`,
    `artifactRoot: ${JSON.stringify(join(devHome, 'artifacts'))}`,
    `pidDir: ${JSON.stringify(join(devHome, 'pids'))}`,
    `logDir: ${JSON.stringify(join(devHome, 'logs'))}`,
    'processes:',
    '  - name: probe',
    `    command: ${JSON.stringify(process.execPath)}`,
    '    args:',
    '      - -e',
    '      - setInterval(() => {}, 1000)',
    `    cwd: ${JSON.stringify(root)}`,
    '    ports: []',
    '    healthUrl: null',
    '',
  ].join('\n'));

  await captureCheckedCommand(process.execPath, [instanceScript, 'up', '--runtime', root], {
    cwd: skiffRoot,
  });
  let status = await statusJson(root);
  assert.equal(status.processes[0].alive, true);

  await captureCheckedCommand(process.execPath, [instanceScript, 'down', '--runtime', root], {
    cwd: skiffRoot,
  });
  status = await statusJson(root);
  assert.equal(status.processes[0].alive, false);
});

test('skiff instance up fails closed without instance.yml', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-missing-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await assert.rejects(
    captureCheckedCommand(process.execPath, [instanceScript, 'up', '--runtime', root], {
      cwd: skiffRoot,
    }),
    /exited with 1/,
  );
});

async function statusJson(root) {
  const outcome = await captureCheckedCommand(
    process.execPath,
    [instanceScript, 'status', '--runtime', root],
    { cwd: skiffRoot },
  );
  return JSON.parse(outcome.stdout);
}
