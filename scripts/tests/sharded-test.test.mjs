import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';

import {
  countTestCases,
  deriveBaseServices,
  discoverTestFiles,
  partitionTestFiles,
  planSourcePublish,
  renderPlan,
  sourceTreeDigest,
  writeStoreSidecar,
} from '../lib/test-orchestration.mjs';

async function tempTree(files) {
  const root = await mkdtemp(join(tmpdir(), 'sharded-test-'));
  for (const [relative, content] of Object.entries(files)) {
    const path = join(root, relative);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content);
  }
  return root;
}

async function manifestWithService(root) {
  return {
    packages: [],
    services: [
      { coordinate: 'example.com/api', root, version: '0.1.0' },
    ],
    testServices: [],
  };
}

test('discoverTestFiles walks nested roots and excludes ignored entries', async () => {
  let fixture;
  try {
    fixture = await tempTree({
      'a.test.skiff': 'test "one"',
      'sub/b.test.skiff': '',
      'sub/.hidden/c.test.skiff': '',
      'node_modules/d.test.skiff': '',
      '.git/e.test.skiff': '',
      'sub/not-a-test.skiff': '',
      'sub/other.txt': '',
    });
    assert.deepEqual(
      await discoverTestFiles([fixture]),
      [join(fixture, 'a.test.skiff'), join(fixture, 'sub', 'b.test.skiff')],
    );
    const fileRoot = join(fixture, 'a.test.skiff');
    assert.deepEqual(await discoverTestFiles([fileRoot]), [fileRoot]);
    assert.deepEqual(
      await discoverTestFiles([fixture, fileRoot]),
      [join(fixture, 'a.test.skiff'), join(fixture, 'sub', 'b.test.skiff')],
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('countTestCases counts test declarations per file', async () => {
  let fixture;
  try {
    fixture = await tempTree({
      'a.test.skiff': 'test "a1"\ntest "a2"\n',
      'b.test.skiff': 'test "b1"\n',
    });
    const files = [
      join(fixture, 'a.test.skiff'),
      join(fixture, 'b.test.skiff'),
    ];
    assert.deepEqual(await countTestCases(files), [2, 1]);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('partitionTestFiles balances case counts greedily and caps at file count', () => {
  const files = ['a', 'b', 'c', 'd', 'e'];
  const counts = [1, 2, 8, 3, 4];
  const shards = partitionTestFiles(files, counts, 2);
  assert.deepEqual(
    shards.map((shard) => shard.cases).sort((a, b) => a - b),
    [9, 9],
  );
  assert.deepEqual(shards.map((shard) => shard.index), [0, 1]);
  assert.deepEqual(shards[0].files, ['c', 'a']);
  assert.deepEqual(shards[1].files, ['e', 'd', 'b']);

  const capped = partitionTestFiles(files, counts, 20);
  assert.equal(capped.length, files.length);
  assert.deepEqual(
    capped.map((shard) => shard.cases).sort((a, b) => a - b),
    [1, 2, 3, 4, 8],
  );

  assert.deepEqual(partitionTestFiles([], [], 4), []);
});

test('sourceTreeDigest is deterministic and excludes ignored directories', async () => {
  let fixture;
  try {
    fixture = await tempTree({
      'package.yml': 'id: example.com/x\nversion: 0.1.0\n',
      'src/main.skiff': 'service "main"',
      'node_modules/junk.js': 'junk',
      '.git/junk': 'junk',
      'target/junk': 'junk',
      'dist/junk': 'junk',
      'build/junk': 'junk',
      '.stack/junk': 'junk',
      '.skiff-package-store/junk': 'junk',
      '.vscode/junk': 'junk',
    });
    const first = await sourceTreeDigest(fixture);
    const second = await sourceTreeDigest(fixture);
    assert.deepEqual(first, second);
    assert.deepEqual(
      first.files.map((file) => file.path),
      ['package.yml', 'src/main.skiff'],
    );
    assert.equal(first.files[0].sha256.length, 64);
    await writeFile(join(fixture, 'src', 'main.skiff'), 'service "changed"');
    const third = await sourceTreeDigest(fixture);
    assert.notEqual(third.digest, first.digest);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('planSourcePublish plans fresh publishes, reuse, and changed-file counts', async () => {
  let store;
  let sourceRoot;
  try {
    store = await mkdtemp(join(tmpdir(), 'sharded-store-'));
    sourceRoot = await tempTree({
      'package.yml': 'id: example.com/api\nversion: 0.1.0\n',
      'main.skiff': 'service "api"',
    });
    const manifest = await manifestWithService(sourceRoot);

    const missing = await planSourcePublish({
      manifest,
      store: join(store, 'missing'),
      fresh: false,
    });
    assert.equal(missing.mode, 'hermetic full rebuild');
    assert.equal(missing.entries.length, 1);
    assert.equal(missing.entries[0].action, 'publish');
    assert.equal(missing.entries[0].firstPublish, true);
    assert.equal(missing.entries[0].changedFiles, null);

    const emptyStore = await mkdtemp(join(tmpdir(), 'sharded-empty-store-'));
    const empty = await planSourcePublish({ manifest, store: emptyStore, fresh: false });
    assert.equal(empty.mode, 'hermetic full rebuild');
    assert.equal(empty.entries[0].action, 'publish');

    const digest = await sourceTreeDigest(sourceRoot);
    await writeStoreSidecar(store, {
      sources: [{
        coordinate: 'example.com/api',
        kind: 'service',
        root: sourceRoot,
        version: '0.1.0',
        digest: digest.digest,
        files: digest.files,
      }],
    });

    const reuse = await planSourcePublish({ manifest, store, fresh: false });
    assert.equal(reuse.mode, 'incremental reuse');
    assert.equal(reuse.entries[0].action, 'reuse');
    assert.equal(reuse.entries[0].changedFiles, null);

    await writeFile(join(sourceRoot, 'main.skiff'), 'service "changed"');
    const changed = await planSourcePublish({ manifest, store, fresh: false });
    assert.equal(changed.mode, 'incremental reuse');
    assert.equal(changed.entries[0].action, 'publish');
    assert.equal(changed.entries[0].firstPublish, false);
    assert.equal(changed.entries[0].changedFiles, 1);
    assert.equal(changed.entries[0].totalFiles, 2);

    const freshPlan = await planSourcePublish({ manifest, store, fresh: true });
    assert.equal(freshPlan.mode, 'hermetic full rebuild');
    assert.equal(freshPlan.entries[0].action, 'publish');
    assert.equal(freshPlan.entries[0].changedFiles, 1);
  } finally {
    await rm(store, { recursive: true, force: true });
    await rm(sourceRoot, { recursive: true, force: true });
  }
});

test('deriveBaseServices matches directory roots and files inside a testServices root', async () => {
  let fixture;
  try {
    fixture = await tempTree({
      'service-tests/a.test.skiff': 'test "a"',
      'service.yml': 'id: example.com/x\n',
    });
    const manifest = {
      packages: [],
      services: [
        { coordinate: 'example.com/api', root: join(fixture, 'api'), version: '0.1.0' },
        { coordinate: 'example.com/registry', root: join(fixture, 'registry'), version: '0.1.0' },
      ],
      testServices: [{
        coordinate: 'example.com/api-tests',
        subjectCoordinate: 'example.com/api',
        root: join(fixture, 'service-tests'),
        version: '0.1.0',
      }],
    };
    const directoryRoot = deriveBaseServices(manifest, [join(fixture, 'service-tests')]);
    assert.equal(directoryRoot.subjectCoordinate, 'example.com/api');
    assert.deepEqual(
      directoryRoot.baseServices.map((entry) => entry.coordinate),
      ['example.com/registry'],
    );
    const fileRoot = deriveBaseServices(
      manifest,
      [join(fixture, 'service-tests', 'a.test.skiff')],
    );
    assert.equal(fileRoot.subjectCoordinate, 'example.com/api');
    assert.throws(
      () => deriveBaseServices(manifest, [join(fixture, 'elsewhere')]),
      /no --sources test root matches/,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('renderPlan prints the canonical plan shape', () => {
  const plan = renderPlan({
    mode: 'hermetic full rebuild',
    store: '/abs/store',
    sourceEntries: [
      {
        coordinate: 'agine.ai/api',
        kind: 'service',
        root: '/abs/api',
        version: '0.1.0',
        bootstrap: false,
        action: 'publish',
        changedFiles: 3,
        totalFiles: 10,
        firstPublish: false,
      },
      {
        coordinate: 'agine.ai/aihub',
        kind: 'service',
        root: '/abs/aihub',
        version: '0.1.0',
        bootstrap: false,
        action: 'reuse',
        changedFiles: null,
        totalFiles: 10,
        firstPublish: false,
      },
    ],
    testLabel: 'agine/service-tests：9 个测试文件 / 42 个 case / 2 个 shard',
    baseLabel: 'resolve from store：account、registry',
  });
  assert.equal(plan, [
    'mode:    hermetic full rebuild（store 不存在 / --fresh）',
    'store:   /abs/store',
    'sources:',
    '  publish  agine.ai/api  （3 个文件变更）',
    '  reuse    agine.ai/aihub（unchanged）',
    'tests:    agine/service-tests：9 个测试文件 / 42 个 case / 2 个 shard',
    'base:     resolve from store：account、registry',
  ].join('\n'));
});
