import {
  lstat,
  mkdir,
  mkdtemp,
  open,
  readFile,
  readdir,
  rm,
  symlink,
  unlink,
  writeFile
} from 'node:fs/promises';
import { hostname, tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import type {
  AssemblyActivationRequest,
  EnvironmentActivationState,
  PendingActivation
} from '../src/protocol/assemblyActivationProtocol.js';
import { withActivationFileLock } from '../src/router/assemblyActivationFileLock.js';
import type { ActivationPersistenceStep } from '../src/router/assemblyActivationFilePersistence.js';
import {
  FileAssemblyActivationStateStore,
  MemoryAssemblyActivationStateStore,
  canonicalActivationJson,
  initialActivationState,
  type AssemblyActivationStateStore,
  type FileAssemblyActivationStateStoreOptions
} from '../src/router/assemblyActivationStateStore.js';

const ENVIRONMENT = 'test';
const ASSEMBLY_A = identity('a');
const ASSEMBLY_B = identity('b');
const ASSEMBLY_C = identity('c');
const temporaryRoots: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryRoots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('FileAssemblyActivationStateStore cooperative CAS', () => {
  it('serializes 20 rounds across two instances with exactly one winner and one conflict', async () => {
    const fixture = await fileFixture();
    const left = new FileAssemblyActivationStateStore(fixture.root);
    const right = new FileAssemblyActivationStateStore(fixture.root);
    const ledger: Array<readonly ['fulfilled', 'rejected']> = [];

    for (let round = 0; round < 20; round += 1) {
      const results = await Promise.allSettled([
        left.prepare(request(`activation-left-${round}`, ASSEMBLY_B), ['replica-a']),
        right.prepare(request(`activation-right-${round}`, ASSEMBLY_C), ['replica-a'])
      ]);
      const fulfilled = results.filter(
        (result): result is PromiseFulfilledResult<EnvironmentActivationState> =>
          result.status === 'fulfilled'
      );
      const rejected = results.filter(
        (result): result is PromiseRejectedResult => result.status === 'rejected'
      );
      expect(fulfilled).toHaveLength(1);
      expect(rejected).toHaveLength(1);
      expect(errorMessage(rejected[0]?.reason)).toMatch(/different assembly activation/);
      ledger.push(['fulfilled', 'rejected']);
      await (round % 2 === 0 ? right : left).abort(
        ENVIRONMENT,
        requiredPending(fulfilled[0]?.value)
      );
      await expect(left.read(ENVIRONMENT)).resolves.toMatchObject({ pending: null });
      await expectNoOwnedResidue(fixture.environmentRoot);
    }

    expect(ledger).toEqual(Array.from({ length: 20 }, () => ['fulfilled', 'rejected']));
  });

  it('makes the same prepare idempotent across concurrent File instances', async () => {
    const fixture = await fileFixture();
    const left = new FileAssemblyActivationStateStore(fixture.root);
    const right = new FileAssemblyActivationStateStore(fixture.root);
    const activation = request('activation-same', ASSEMBLY_B);

    const [first, second] = await Promise.all([
      left.prepare(activation, ['replica-b', 'replica-a']),
      right.prepare(activation, ['replica-a', 'replica-b'])
    ]);

    expect(first).toEqual(second);
    expect(first.pending?.participantReplicaIds).toEqual(['replica-a', 'replica-b']);
    await expectNoOwnedResidue(fixture.environmentRoot);
  });

  it('reclaims only a complete same-host owner with an absent PID after grace', async () => {
    const fixture = await fileFixture();
    const lock = join(fixture.environmentRoot, '.activation.lock');
    await writeLockOwner(lock, { pid: absentPid(), createdAtMs: 1 });
    const left = new FileAssemblyActivationStateStore(fixture.root, shortLockOptions());
    const right = new FileAssemblyActivationStateStore(fixture.root, shortLockOptions());

    const results = await Promise.allSettled([
      left.prepare(request('activation-after-stale-left', ASSEMBLY_B), ['replica-a']),
      right.prepare(request('activation-after-stale-right', ASSEMBLY_C), ['replica-a'])
    ]);
    expect(results.filter((result) => result.status === 'fulfilled')).toHaveLength(1);
    const rejected = results.find((result) => result.status === 'rejected');
    expect(rejected?.status === 'rejected' ? errorMessage(rejected.reason) : '').toMatch(
      /different assembly activation/
    );
    await expectNoOwnedResidue(fixture.environmentRoot);
  });

  it('fails closed for foreign, live-or-reused PID, corrupt, identity-mismatched, and symlink owners', async () => {
    const cases: Array<{
      label: string;
      install: (lock: string) => Promise<void>;
      error: RegExp;
    }> = [
      {
        label: 'foreign',
        install: (lock) => writeLockOwner(lock, { hostname: 'foreign.invalid', pid: absentPid() }),
        error: /foreign host/
      },
      {
        label: 'PID reuse',
        install: (lock) => writeLockOwner(lock, { pid: process.pid, createdAtMs: 1 }),
        error: /PID is live or may have been reused/
      },
      {
        label: 'corrupt',
        install: (lock) => writeFile(lock, '{'),
        error: /incomplete or invalid/
      },
      {
        label: 'identity mismatch',
        install: (lock) => writeLockOwner(lock, { inodeOffset: 1n, pid: absentPid() }),
        error: /incomplete or invalid/
      },
      {
        label: 'symlink',
        install: async (lock) => {
          const target = `${lock}.foreign`;
          await writeFile(target, 'foreign');
          await symlink(target, lock);
        },
        error: /incomplete or invalid/
      }
    ];

    for (const lockCase of cases) {
      const fixture = await fileFixture();
      const lock = join(fixture.environmentRoot, '.activation.lock');
      await lockCase.install(lock);
      const before = await lstat(lock);
      const store = new FileAssemblyActivationStateStore(fixture.root, shortLockOptions());
      await expect(
        store.prepare(request(`activation-${lockCase.label.replaceAll(' ', '-')}`, ASSEMBLY_B), ['replica-a'])
      ).rejects.toThrow(lockCase.error);
      const after = await lstat(lock);
      expect([after.dev, after.ino]).toEqual([before.dev, before.ino]);
    }
  });

  it('does not reclaim an absent same-host owner before the bounded grace elapses', async () => {
    const fixture = await fileFixture();
    const lock = join(fixture.environmentRoot, '.activation.lock');
    await writeLockOwner(lock, { pid: absentPid() });
    const before = await lstat(lock);
    const store = new FileAssemblyActivationStateStore(fixture.root, {
      lock: { acquireTimeoutMs: 30, retryDelayMs: 2, staleGraceMs: 1_000 }
    });

    await expect(
      store.prepare(request('activation-before-grace', ASSEMBLY_B), ['replica-a'])
    ).rejects.toThrow(/stale grace did not elapse/);
    const after = await lstat(lock);
    expect([after.dev, after.ino]).toEqual([before.dev, before.ino]);
  });
});

describe('activation state reducer parity', () => {
  it('keeps File and Memory prepare/abort/commit behavior identical', async () => {
    const fixture = await fileFixture();
    const stores: AssemblyActivationStateStore[] = [
      new FileAssemblyActivationStateStore(fixture.root),
      new MemoryAssemblyActivationStateStore(fixture.initial)
    ];

    const results = await Promise.all(stores.map(exerciseReducer));
    expect(results[0]).toEqual(results[1]);
    expect(results[0]?.firstCommitError).toMatch(/must not be empty/);
    expect(results[0]?.incompleteCommitError).toMatch(/every frozen participant/);
    expect(results[0]?.committed).toMatchObject({ committed: { generation: 2 }, pending: null });
    expect(results[0]?.replayedWithEmptyAck).toEqual(results[0]?.committed);
    await expectNoOwnedResidue(fixture.environmentRoot);
  });
});

describe('activation file persistence', () => {
  it('rejects corrupt, non-canonical, and symlink state records', async () => {
    const corrupt = await fileFixture();
    await writeFile(corrupt.statePath, '{');
    await expect(new FileAssemblyActivationStateStore(corrupt.root).read(ENVIRONMENT)).rejects.toThrow();

    const nonCanonical = await fileFixture();
    await writeFile(nonCanonical.statePath, `${canonicalActivationJson(nonCanonical.initial)}\n`);
    await expect(
      new FileAssemblyActivationStateStore(nonCanonical.root).read(ENVIRONMENT)
    ).rejects.toThrow(/not canonical JSON/);

    const linked = await fileFixture();
    const foreign = join(linked.root, 'foreign-activation.json');
    await writeFile(foreign, canonicalActivationJson(linked.initial));
    await unlink(linked.statePath);
    await symlink(foreign, linked.statePath);
    await expect(new FileAssemblyActivationStateStore(linked.root).read(ENVIRONMENT)).rejects.toThrow(
      /must not be a symlink/
    );
  });

  it('reopens the exact durable committed state from a new File instance', async () => {
    const fixture = await fileFixture();
    const first = new FileAssemblyActivationStateStore(fixture.root);
    const prepared = await first.prepare(request('activation-reopen', ASSEMBLY_B), ['replica-a']);
    const pending = requiredPending(prepared);
    await first.commit(ENVIRONMENT, pending, ['replica-a'], ['replica-a']);

    const reopened = new FileAssemblyActivationStateStore(fixture.root);
    await expect(reopened.read(ENVIRONMENT)).resolves.toEqual(
      initialActivationState({ environment: ENVIRONMENT, generation: 2, assemblyIdentity: ASSEMBLY_B })
    );
    await expectNoOwnedResidue(fixture.environmentRoot);
  });

  it.each<ActivationPersistenceStep>([
    'temporary-open',
    'temporary-write',
    'temporary-sync',
    'rename',
    'parent-sync'
  ])('keeps only canonical old/new state and converges after the %s failpoint', async (step) => {
    let armed = true;
    const injected = new Error(`injected ${step}`);
    const fixture = await fileFixture({
      persistenceFailpoint: (reached) => {
        if (armed && reached === step) {
          armed = false;
          throw injected;
        }
      }
    });
    const next = await new MemoryAssemblyActivationStateStore(fixture.initial).prepare(
      request('activation-failpoint', ASSEMBLY_B),
      ['replica-a']
    );

    await expect(
      fixture.store.prepare(request('activation-failpoint', ASSEMBLY_B), ['replica-a'])
    ).rejects.toBe(injected);
    const afterFailureBytes = await readFile(fixture.statePath, 'utf8');
    expect([canonicalActivationJson(fixture.initial), canonicalActivationJson(next)]).toContain(
      afterFailureBytes
    );
    expect(afterFailureBytes).toBe(
      step === 'parent-sync' ? canonicalActivationJson(next) : canonicalActivationJson(fixture.initial)
    );
    await expectNoOwnedResidue(fixture.environmentRoot);

    await expect(
      fixture.store.prepare(request('activation-failpoint', ASSEMBLY_B), ['replica-a'])
    ).resolves.toEqual(next);
    await expect(readFile(fixture.statePath, 'utf8')).resolves.toBe(canonicalActivationJson(next));
    await expectNoOwnedResidue(fixture.environmentRoot);
  });

  it('preserves both primary and cleanup errors without leaving owned files', async () => {
    const primary = new Error('injected write failure');
    const cleanup = new Error('injected cleanup failure');
    const fixture = await fileFixture({
      persistenceFailpoint: (step) => {
        if (step === 'temporary-write') {
          throw primary;
        }
        if (step === 'temporary-cleanup') {
          throw cleanup;
        }
      }
    });

    const error = await fixture.store
      .prepare(request('activation-error-order', ASSEMBLY_B), ['replica-a'])
      .then(() => undefined, (failure: unknown) => failure);
    expect(error).toBeInstanceOf(AggregateError);
    expect((error as AggregateError).errors).toEqual([primary, cleanup]);
    await expectNoOwnedResidue(fixture.environmentRoot);
  });

  it('preserves a mutation error together with lock cleanup identity failure', async () => {
    const fixture = await fileFixture();
    const lock = join(fixture.environmentRoot, '.activation.lock');
    const primary = new Error('primary mutation failure');

    const error = await withActivationFileLock(
      lock,
      async () => {
        await unlink(lock);
        throw primary;
      },
      { acquireTimeoutMs: 30, retryDelayMs: 2, staleGraceMs: 0 }
    ).then(() => undefined, (failure: unknown) => failure);
    expect(error).toBeInstanceOf(AggregateError);
    expect((error as AggregateError).errors[0]).toBe(primary);
    expect((error as AggregateError).errors).toHaveLength(2);
    await expectNoOwnedResidue(fixture.environmentRoot);
  });
});

async function exerciseReducer(store: AssemblyActivationStateStore) {
  const prepared = await store.prepare(
    request('activation-parity', ASSEMBLY_B),
    ['replica-b', 'replica-a']
  );
  const pending = requiredPending(prepared);
  const same = await store.prepare(
    request('activation-parity', ASSEMBLY_B),
    ['replica-a', 'replica-b']
  );
  const firstCommitError = await store
    .commit(ENVIRONMENT, pending, [], [])
    .then(() => undefined, errorMessage);
  const incompleteCommitError = await store
    .commit(ENVIRONMENT, pending, ['replica-a', 'replica-b'], ['replica-a'])
    .then(() => undefined, errorMessage);
  const committed = await store.commit(
    ENVIRONMENT,
    pending,
    ['replica-b', 'replica-a'],
    ['replica-b', 'replica-a']
  );
  const replayedWithEmptyAck = await store.commit(ENVIRONMENT, pending, [], []);
  const abortPrepared = await store.prepare(
    request('activation-abort', ASSEMBLY_C, 2),
    ['replica-a']
  );
  const aborted = await store.abort(ENVIRONMENT, requiredPending(abortPrepared));
  return {
    prepared,
    same,
    firstCommitError,
    incompleteCommitError,
    committed,
    replayedWithEmptyAck,
    aborted
  };
}

async function fileFixture(
  options: FileAssemblyActivationStateStoreOptions = {}
): Promise<{
  root: string;
  environmentRoot: string;
  statePath: string;
  initial: EnvironmentActivationState;
  store: FileAssemblyActivationStateStore;
}> {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-activation-state-'));
  temporaryRoots.push(root);
  const environmentRoot = join(root, 'environments', ENVIRONMENT);
  const statePath = join(environmentRoot, 'activation.json');
  await mkdir(dirname(statePath), { recursive: true });
  const initial = initialActivationState({
    environment: ENVIRONMENT,
    generation: 1,
    assemblyIdentity: ASSEMBLY_A
  });
  await writeFile(statePath, canonicalActivationJson(initial));
  return {
    root,
    environmentRoot,
    statePath,
    initial,
    store: new FileAssemblyActivationStateStore(root, options)
  };
}

function request(
  activationId: string,
  assemblyIdentity: string,
  expectedGeneration = 1
): AssemblyActivationRequest {
  return {
    schemaVersion: 'skiff-assembly-activation-request-v1',
    environment: ENVIRONMENT,
    activationId,
    expectedGeneration,
    assembly: { assemblyIdentity }
  };
}

function requiredPending(state: EnvironmentActivationState | undefined): PendingActivation {
  if (state?.pending === null || state?.pending === undefined) {
    throw new Error('test expected a pending activation');
  }
  return state.pending;
}

function identity(character: string): string {
  return `skiff-runtime-assembly-v1:sha256:${character.repeat(64)}`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function shortLockOptions(): FileAssemblyActivationStateStoreOptions {
  return { lock: { acquireTimeoutMs: 100, retryDelayMs: 2, staleGraceMs: 0 } };
}

function absentPid(): number {
  for (const candidate of [999_999, 888_888, 777_777]) {
    try {
      process.kill(candidate, 0);
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ESRCH') {
        return candidate;
      }
    }
  }
  throw new Error('test could not find an absent PID');
}

async function writeLockOwner(
  path: string,
  overrides: Readonly<{
    pid?: number;
    hostname?: string;
    createdAtMs?: number;
    inodeOffset?: bigint;
  }>
): Promise<void> {
  const handle = await open(path, 'wx', 0o600);
  const stats = await handle.stat({ bigint: true });
  const owner = {
    schemaVersion: 'skiff-local-activation-lock-v1',
    nonce: 'a'.repeat(32),
    pid: overrides.pid ?? absentPid(),
    hostname: overrides.hostname ?? hostname(),
    device: stats.dev.toString(),
    inode: (stats.ino + (overrides.inodeOffset ?? 0n)).toString(),
    createdAtMs: overrides.createdAtMs ?? Date.now()
  };
  await handle.writeFile(JSON.stringify(owner));
  await handle.sync();
  await handle.close();
}

async function expectNoOwnedResidue(environmentRoot: string): Promise<void> {
  const owned = (await readdir(environmentRoot)).filter(
    (entry) =>
      entry === '.activation.lock' ||
      entry === '.activation.lock.reclaim' ||
      /^\.activation\..+\.tmp$/.test(entry)
  );
  expect(owned).toEqual([]);
}
