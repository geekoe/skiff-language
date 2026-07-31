import assert from 'node:assert/strict';
import { access, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { parseVerifyArgs } from '../lib/verify-cli.mjs';
import { assertPlanIntegrity } from '../lib/verify-plan.mjs';
import { runVerifyPlan } from '../lib/verify-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('verify CLI parses --jobs with default 1 and rejects invalid budgets', () => {
  assert.equal(parseVerifyArgs([]).jobs, 1);
  assert.equal(parseVerifyArgs(['--jobs', '4']).jobs, 4);
  assert.equal(parseVerifyArgs(['--jobs=2']).jobs, 2);
  assert.equal(
    parseVerifyArgs(['--only', 'tests', '--jobs', '3', '--list']).jobs,
    3,
  );
  for (const args of [
    ['--jobs', '0'],
    ['--jobs', '-1'],
    ['--jobs', '1.5'],
    ['--jobs', 'abc'],
    ['--jobs', '01'],
    ['--jobs=1', '--jobs', '2'],
    ['--jobs'],
  ]) {
    assert.throws(() => parseVerifyArgs(args), /--jobs/);
  }
});

test('jobs=1 runs tasks serially in plan order', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-serial-'));
  const log = join(fixture, 'log');
  try {
    const plan = serialMarkerPlan(fixture, log, ['one', 'two', 'three'], 60);
    const summary = await runVerifyPlan(plan, fixture, { jobs: 1 });
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'one', status: 'passed' },
        { id: 'two', status: 'passed' },
        { id: 'three', status: 'passed' },
      ],
    );
    assert.deepEqual((await readFile(log, 'utf8')).trim().split('\n'), [
      'start one',
      'end one',
      'start two',
      'end two',
      'start three',
      'end three',
    ]);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('jobs=2 caps concurrency at two slots and preserves plan dispatch order', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-parallel-'));
  const log = join(fixture, 'log');
  try {
    const plan = {
      selectors: ['test'],
      tasks: [
        handshakeTask('one', fixture, log, {
          signal: join(fixture, 'one-started'),
          signalFirst: true,
          waitFor: join(fixture, 'two-started'),
        }),
        handshakeTask('two', fixture, log, {
          waitFor: join(fixture, 'one-started'),
          signal: join(fixture, 'two-started'),
          delayMs: 80,
        }),
        handshakeTask('three', fixture, log, {
          signal: join(fixture, 'three-started'),
          signalFirst: true,
          waitFor: join(fixture, 'four-started'),
        }),
        handshakeTask('four', fixture, log, {
          waitFor: join(fixture, 'three-started'),
          signal: join(fixture, 'four-started'),
          delayMs: 80,
        }),
      ],
    };
    const summary = await runVerifyPlan(plan, fixture, { jobs: 2 });
    assert.ok(summary.results.every((result) => result.status === 'passed'));

    const lines = (await readFile(log, 'utf8')).trim().split('\n');
    assert.deepEqual(
      lines.filter((line) => line.startsWith('start ')),
      ['start one', 'start two', 'start three', 'start four'],
    );
    assert.ok(
      lineIndex(lines, 'start two') < lineIndex(lines, 'end one'),
      'two must overlap one under jobs=2',
    );
    assert.ok(
      lineIndex(lines, 'start three') > lineIndex(lines, 'end one'),
      'three must wait until at least one slot is free',
    );
    let concurrent = 0;
    let maximum = 0;
    for (const line of lines) {
      if (line.startsWith('start ')) {
        concurrent += 1;
        maximum = Math.max(maximum, concurrent);
      } else {
        concurrent -= 1;
      }
    }
    assert.equal(maximum, 2);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('slots reserve budget so a slots=2 task runs alone under jobs=2', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-slots-'));
  const log = join(fixture, 'log');
  try {
    const plan = {
      selectors: ['test'],
      tasks: [
        markerTask('heavy', fixture, log, 150, { slots: 2 }),
        markerTask('light', fixture, log, 50),
      ],
    };
    const summary = await runVerifyPlan(plan, fixture, { jobs: 2 });
    assert.ok(summary.results.every((result) => result.status === 'passed'));
    const lines = (await readFile(log, 'utf8')).trim().split('\n');
    assert.ok(
      lineIndex(lines, 'start light') > lineIndex(lines, 'end heavy'),
      'light must wait while a slots=2 task occupies the whole jobs=2 budget',
    );

    const pair = {
      selectors: ['test'],
      tasks: [
        markerTask('first', fixture, log, 120),
        markerTask('second', fixture, log, 60),
      ],
    };
    await runVerifyPlan(pair, fixture, { jobs: 2 });
    const pairLines = (await readFile(log, 'utf8')).trim().split('\n');
    assert.ok(
      lineIndex(pairLines, 'start second') < lineIndex(pairLines, 'end first'),
      'two slots=1 tasks must overlap under jobs=2',
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('exclusive tasks start only when nothing else is running', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-exclusive-'));
  const log = join(fixture, 'log');
  try {
    const plan = {
      selectors: ['test'],
      tasks: [
        markerTask('exclusive', fixture, log, 120, { exclusive: true }),
        markerTask('filler-a', fixture, log, 100),
        markerTask('filler-b', fixture, log, 100),
      ],
    };
    const summary = await runVerifyPlan(plan, fixture, { jobs: 2 });
    assert.ok(summary.results.every((result) => result.status === 'passed'));
    const lines = (await readFile(log, 'utf8')).trim().split('\n');
    assert.ok(
      lineIndex(lines, 'start filler-a') > lineIndex(lines, 'end exclusive'),
      'fillers must wait while the exclusive task runs',
    );
    assert.ok(
      lineIndex(lines, 'start filler-b') < lineIndex(lines, 'end filler-a'),
      'non-exclusive fillers may share the remaining budget after exclusivity',
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('live/manual registry tasks are forced exclusive even without the flag', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-live-exclusive-'));
  const log = join(fixture, 'log');
  try {
    const plan = {
      selectors: ['test'],
      tasks: [
        markerTask('live:manual', fixture, log, 120, {
          kind: 'live/manual',
          tier: 'live/manual',
          ownership: 'external',
        }),
        markerTask('neighbor', fixture, log, 50),
      ],
    };
    const summary = await runVerifyPlan(plan, fixture, { jobs: 2 });
    assert.ok(summary.results.every((result) => result.status === 'passed'));
    const lines = (await readFile(log, 'utf8')).trim().split('\n');
    assert.ok(
      lineIndex(lines, 'start neighbor') > lineIndex(lines, 'end live:manual'),
      'live/manual must run alone regardless of the exclusive field',
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('abort stops dispatch, terminates in-flight process groups, and preserves completed results', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-abort-'));
  const completedMarker = join(fixture, 'completed-ran');
  const startedMarker = join(fixture, 'long-started');
  const grandchildMarker = join(fixture, 'grandchild-ran');
  const neverMarker = join(fixture, 'never-ran');
  try {
    const controller = new AbortController();
    const plan = {
      selectors: ['test'],
      tasks: [
        {
          id: 'completed',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            `require('node:fs').writeFileSync(${JSON.stringify(completedMarker)}, 'ran')`,
          ],
          cwd: fixture,
        },
        {
          id: 'long-running',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            [
              `const { spawn } = require('node:child_process');`,
              `const fs = require('node:fs');`,
              `fs.writeFileSync(${JSON.stringify(startedMarker)}, 'started');`,
              `const grandchild = spawn(process.execPath, ['--eval', ${JSON.stringify(
                `setTimeout(() => require('node:fs').writeFileSync(${JSON.stringify(grandchildMarker)}, 'ran'), 2500)`,
              )}], { stdio: 'ignore' });`,
              `setInterval(() => {}, 500);`,
            ].join('\n'),
          ],
          cwd: fixture,
        },
        {
          id: 'never-dispatched',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            `require('node:fs').writeFileSync(${JSON.stringify(neverMarker)}, 'ran')`,
          ],
          cwd: fixture,
        },
      ],
    };
    const summaryPromise = runVerifyPlan(plan, fixture, {
      jobs: 1,
      signal: controller.signal,
    });
    await waitForFile(startedMarker);
    controller.abort(new Error('test interrupt'));
    const summary = await summaryPromise;

    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'completed', status: 'passed' },
        { id: 'long-running', status: 'interrupted' },
        { id: 'never-dispatched', status: 'interrupted' },
      ],
    );
    assert.match(summary.results[1].reason, /test interrupt/);
    assert.match(summary.results[2].reason, /before task dispatch/);
    await access(completedMarker);
    await assert.rejects(access(neverMarker), { code: 'ENOENT' });
    await delay(400);
    await assert.rejects(access(grandchildMarker), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('mutating tasks write through redirect env vars into the private root only', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-mutation-'));
  const redirectPath = 'generated/data';
  const privateRootName = 'mutation-fixture';
  try {
    const plan = {
      selectors: ['test'],
      tasks: [
        {
          id: 'mutation:fixture',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            [
              `const fs = require('node:fs');`,
              `const path = require('node:path');`,
              `const target = process.env.SKIFF_MUTATION_REDIRECT;`,
              `const privateRoot = process.env.SKIFF_VERIFY_TASK_PRIVATE_ROOT;`,
              `if (!target || !privateRoot) process.exit(2);`,
              `if (!target.startsWith(privateRoot)) process.exit(3);`,
              `if (!fs.existsSync(privateRoot)) process.exit(4);`,
              `fs.writeFileSync(path.join(target, 'payload.txt'), 'written');`,
              `if (!fs.existsSync(path.join(target, 'payload.txt'))) process.exit(5);`,
            ].join('\n'),
          ],
          cwd: fixture,
          exclusive: true,
          mutation: {
            paths: [redirectPath],
            redirect: { SKIFF_MUTATION_REDIRECT: redirectPath },
          },
        },
      ],
    };
    const summary = await runVerifyPlan(plan, fixture, { jobs: 1 });
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [{ id: 'mutation:fixture', status: 'passed' }],
    );
    await assert.rejects(
      stat(join(fixture, 'var', 'verify', 'tasks', privateRootName)),
      { code: 'ENOENT' },
    );
    await assert.rejects(access(join(fixture, redirectPath)), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('parallel task output is captured as contiguous per-task blocks', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-output-'));
  try {
    const makeTask = (id, prefix) => ({
      id,
      kind: 'test',
      command: process.execPath,
      args: [
        '--eval',
        `for (let index = 0; index < 30; index += 1) process.stdout.write(${JSON.stringify(prefix)} + '-' + index + '\\n')`,
      ],
      cwd: fixture,
    });
    const plan = {
      selectors: ['test'],
      tasks: [makeTask('alpha', 'A'), makeTask('beta', 'B')],
    };
    const chunks = [];
    const originalLog = console.log;
    const originalWrite = process.stdout.write;
    console.log = (...values) => chunks.push(`${values.join(' ')}\n`);
    process.stdout.write = (chunk) => {
      chunks.push(String(chunk));
      return true;
    };
    try {
      await runVerifyPlan(plan, fixture, { jobs: 2 });
    } finally {
      console.log = originalLog;
      process.stdout.write = originalWrite;
    }
    const lines = chunks.join('').split('\n');
    for (const prefix of ['A', 'B']) {
      const indexes = lines.flatMap((line, index) =>
        line.startsWith(`${prefix}-`) ? [index] : []);
      assert.equal(indexes.length, 30, prefix);
      for (let index = 1; index < indexes.length; index += 1) {
        assert.equal(
          indexes[index],
          indexes[index - 1] + 1,
          `${prefix} output must be contiguous`,
        );
      }
    }
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('assertPlanIntegrity rejects invalid slots, exclusive, and mutation shapes', () => {
  const base = {
    id: 'integrity-task',
    kind: 'test',
    command: 'node',
    args: [],
    cwd: root,
  };
  for (const slots of [0, -1, 1.5, '1']) {
    assert.throws(
      () => assertPlanIntegrity([{ ...base, slots }]),
      /invalid verify task slots/,
    );
  }
  for (const exclusive of ['yes', 1, null]) {
    assert.throws(
      () => assertPlanIntegrity([{ ...base, exclusive }]),
      /invalid verify task exclusive/,
    );
  }
  assert.doesNotThrow(() =>
    assertPlanIntegrity([{ ...base, slots: 2, exclusive: true }]));

  const validMutation = {
    paths: ['var/generated'],
    redirect: { SKIFF_GENERATED_ROOT: 'var/generated' },
  };
  assert.throws(
    () => assertPlanIntegrity([{ ...base, mutation: validMutation }]),
    /mutating verify task must be exclusive/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      ...base,
      exclusive: true,
      mutation: { ...validMutation, paths: [] },
    }]),
    /mutation paths/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      ...base,
      exclusive: true,
      mutation: {
        paths: ['/abs/path'],
        redirect: { SKIFF_ABS: '/abs/path' },
      },
    }]),
    /mutation paths/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      ...base,
      exclusive: true,
      mutation: {
        paths: ['a/../b'],
        redirect: { SKIFF_TRAVERSAL: 'a/../b' },
      },
    }]),
    /mutation paths/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      ...base,
      exclusive: true,
      mutation: {
        paths: ['a'],
        redirect: { '1BAD': 'a' },
      },
    }]),
    /redirect key/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      ...base,
      exclusive: true,
      mutation: {
        paths: ['a'],
        redirect: { SKIFF_OUTSIDE: 'b' },
      },
    }]),
    /redirect value/,
  );
  assert.doesNotThrow(() =>
    assertPlanIntegrity([{ ...base, exclusive: true, mutation: validMutation }]));
});

test('runVerifyPlan rejects invalid jobs budgets and tasks whose slots exceed the budget', async () => {
  const plan = {
    selectors: ['test'],
    tasks: [
      {
        id: 'slots-big',
        kind: 'test',
        command: process.execPath,
        args: ['--eval', ''],
        cwd: root,
        slots: 2,
      },
    ],
  };
  await assert.rejects(
    runVerifyPlan(plan, root, { jobs: 1 }),
    /requires 2 slots but the jobs budget is 1/,
  );
  for (const jobs of [0, -1, 1.5, '2']) {
    await assert.rejects(
      runVerifyPlan(plan, root, { jobs }),
      /jobs must be a positive integer/,
    );
  }
});

function serialMarkerPlan(fixture, log, names, delayMs) {
  return {
    selectors: ['test'],
    tasks: names.map((name) => markerTask(name, fixture, log, delayMs)),
  };
}

function handshakeTask(id, fixture, log, { waitFor, signal, delayMs = 0, signalFirst = false }) {
  const statements = [
    `const fs = require('node:fs');`,
  ];
  if (signalFirst) {
    statements.push(
      `fs.appendFileSync(${JSON.stringify(log)}, 'start ${id}\\n');`,
      ...(signal === undefined ? [] : [`fs.writeFileSync(${JSON.stringify(signal)}, 'started');`]),
    );
  }
  statements.push(...waitStatements(waitFor));
  if (!signalFirst) {
    statements.push(
      `fs.appendFileSync(${JSON.stringify(log)}, 'start ${id}\\n');`,
      ...(signal === undefined ? [] : [`fs.writeFileSync(${JSON.stringify(signal)}, 'started');`]),
    );
  }
  if (delayMs > 0) {
    statements.push(
      `setTimeout(() => {`,
      `  fs.appendFileSync(${JSON.stringify(log)}, 'end ${id}\\n');`,
      `}, ${delayMs});`,
    );
  } else {
    statements.push(`fs.appendFileSync(${JSON.stringify(log)}, 'end ${id}\\n');`);
  }
  return {
    id,
    kind: 'test',
    command: process.execPath,
    args: ['--eval', statements.join('\n')],
    cwd: fixture,
  };
}

function waitStatements(waitFor) {
  if (waitFor === undefined) {
    return [];
  }
  return [
    `const deadline = Date.now() + 5000;`,
    `while (Date.now() < deadline) {`,
    `  if (fs.existsSync(${JSON.stringify(waitFor)})) break;`,
    `  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10);`,
    `}`,
    `if (!fs.existsSync(${JSON.stringify(waitFor)})) process.exit(7);`,
  ];
}

function markerTask(id, fixture, log, delayMs, options = {}) {
  return {
    id,
    kind: 'test',
    command: process.execPath,
    args: [
      '--eval',
      [
        `const fs = require('node:fs');`,
        `fs.appendFileSync(${JSON.stringify(log)}, 'start ${id}\\n');`,
        `setTimeout(() => {`,
        `  fs.appendFileSync(${JSON.stringify(log)}, 'end ${id}\\n');`,
        `}, ${delayMs});`,
      ].join(''),
    ],
    cwd: fixture,
    ...options,
  };
}

function lineIndex(lines, value) {
  const index = lines.indexOf(value);
  assert.notEqual(index, -1, `missing log line ${value}`);
  return index;
}

async function waitForFile(path, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      await access(path);
      return;
    } catch {}
    await delay(20);
  }
  throw new Error(`timed out waiting for ${path}`);
}

function delay(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}
