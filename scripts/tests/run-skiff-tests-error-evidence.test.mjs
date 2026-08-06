import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  captureIsolatedRuntimeLogEvidence,
  ISOLATED_RUNTIME_LOG_EVIDENCE_SCHEMA_VERSION,
  ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES,
} from '../lib/isolated-test-runtime-log-evidence.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const entry = join(root, 'scripts', 'run-skiff-tests.mjs');
const suiteModuleUrl = pathToFileURL(
  join(root, 'scripts', 'lib', 'skiff-source-test-suite.mjs'),
).href;
const fixtureEnvironmentName = 'SKIFF_RUN_SKIFF_TESTS_ERROR_FIXTURE';

test('plain entry Error preserves the original one-line diagnostic and exits 1', async () => {
  await withEntryHarness(async (runEntry) => {
    const result = runEntry({ message: 'ordinary failure' });

    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.signal, null);
    assert.equal(result.stdout, '');
    assert.equal(result.stderr, 'error: ordinary failure\n');
  });
});

test('entry appends valid isolated evidence immediately after the Error message', async () => {
  await withEntryHarness(async (runEntry) => {
    const result = runEntry({
      message: 'source-test child failed',
      evidence: {
        schemaVersion: ISOLATED_RUNTIME_LOG_EVIDENCE_SCHEMA_VERSION,
        logs: [
          {
            component: 'runtime',
            stream: 'stderr',
            sanitizedTail: 'whole-assembly candidate link failed: exact cause\n',
            truncated: false,
          },
        ],
      },
    });

    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.signal, null);
    assert.equal(result.stdout, '');
    assert.equal(
      result.stderr,
      [
        'error: source-test child failed',
        '[isolated runtime stderr]',
        'whole-assembly candidate link failed: exact cause',
        '',
      ].join('\n'),
    );
  });
});

test('entry ignores empty and invalid isolated evidence', async () => {
  await withEntryHarness(async (runEntry) => {
    const fixtures = [
      {
        message: 'empty evidence',
        evidence: {
          schemaVersion: ISOLATED_RUNTIME_LOG_EVIDENCE_SCHEMA_VERSION,
          logs: [
            {
              component: 'runtime',
              stream: 'stderr',
              sanitizedTail: ' \n',
              truncated: false,
            },
          ],
        },
      },
      {
        message: 'invalid evidence',
        evidence: {
          schemaVersion: 'invalid-schema',
          logs: [
            {
              component: 'runtime',
              stream: 'stderr',
              sanitizedTail: 'must not render',
              truncated: false,
            },
          ],
        },
      },
    ];

    for (const fixture of fixtures) {
      const result = runEntry(fixture);
      assert.equal(result.status, 1, result.stderr);
      assert.equal(result.signal, null);
      assert.equal(result.stdout, '');
      assert.equal(result.stderr, `error: ${fixture.message}\n`);
    }
  });
});

test('entry renders only the existing sanitized bounded log tail', async () => {
  const evidenceRoot = await mkdtemp(join(tmpdir(), 'skiff-entry-log-evidence-'));
  const secret = 'P5_F404_RAW_SECRET';
  const privatePath = `/private/var/tmp/${secret}/runtime.skiff`;
  try {
    const logDir = join(evidenceRoot, 'instance', 'logs');
    await mkdir(logDir, { recursive: true });
    await writeFile(
      join(logDir, 'runtime.err.log'),
      `${'x'.repeat(ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES * 2)}\n`
        + `token=${secret} at ${privatePath}\n`,
    );
    const evidence = await captureIsolatedRuntimeLogEvidence(evidenceRoot);

    await withEntryHarness(async (runEntry) => {
      const result = runEntry({ message: 'sanitized failure', evidence });
      const prefix = [
        'error: sanitized failure',
        '[isolated runtime stderr (tail, truncated)]',
        '',
      ].join('\n');

      assert.equal(result.status, 1, result.stderr);
      assert.equal(result.signal, null);
      assert.equal(result.stdout, '');
      assert.ok(result.stderr.startsWith(prefix), result.stderr);
      assert.doesNotMatch(result.stderr, new RegExp(secret));
      assert.doesNotMatch(result.stderr, /\/private\/var\/tmp/);
      assert.match(result.stderr, /<REDACTED_SECRET>/);
      assert.match(result.stderr, /<PATH>/);
      const renderedTail = result.stderr.slice(prefix.length).trimEnd();
      assert.ok(
        Buffer.byteLength(renderedTail) <= ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES,
        `rendered tail was ${Buffer.byteLength(renderedTail)} bytes`,
      );
    });
  } finally {
    await rm(evidenceRoot, { recursive: true, force: true });
  }
});

async function withEntryHarness(run) {
  const harnessRoot = await mkdtemp(join(tmpdir(), 'skiff-entry-harness-'));
  const loaderPath = join(harnessRoot, 'loader.mjs');
  try {
    const stubSource = [
      'export async function runCanonicalSkiffSourceTests() {',
      `  const fixture = JSON.parse(process.env.${fixtureEnvironmentName});`,
      '  const error = new Error(fixture.message);',
      "  if (Object.hasOwn(fixture, 'evidence')) {",
      "    Object.defineProperty(error, 'isolatedRuntimeLogEvidence', {",
      '      value: fixture.evidence,',
      '    });',
      '  }',
      '  throw error;',
      '}',
    ].join('\n');
    const loaderSource = [
      `const suiteModuleUrl = ${JSON.stringify(suiteModuleUrl)};`,
      `const stubSource = ${JSON.stringify(stubSource)};`,
      'export async function load(url, context, nextLoad) {',
      '  if (url === suiteModuleUrl) {',
      "    return { format: 'module', shortCircuit: true, source: stubSource };",
      '  }',
      '  return nextLoad(url, context);',
      '}',
    ].join('\n');
    await writeFile(loaderPath, loaderSource);

    await run((fixture) => spawnSync(
      process.execPath,
      ['--no-warnings', '--experimental-loader', loaderPath, entry],
      {
        cwd: root,
        encoding: 'utf8',
        env: {
          ...process.env,
          [fixtureEnvironmentName]: JSON.stringify(fixture),
        },
      },
    ));
  } finally {
    await rm(harnessRoot, { recursive: true, force: true });
  }
}
