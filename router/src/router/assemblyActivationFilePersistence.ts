import { randomBytes } from 'node:crypto';
import {
  lstat,
  open,
  readFile,
  realpath,
  rename,
  unlink
} from 'node:fs/promises';
import { dirname, join, relative, resolve, sep } from 'node:path';

import {
  decodeEnvironmentActivationState,
  type EnvironmentActivationState
} from '../protocol/assemblyActivationProtocol.js';
import { decodeRawEnvironmentActivationState } from '../protocol/assemblyActivationRawCodec.js';
import {
  throwCleanupErrors,
  throwPrimaryWithCleanup
} from './assemblyActivationCleanupErrors.js';

export type ActivationPersistenceStep =
  | 'temporary-open'
  | 'temporary-write'
  | 'temporary-sync'
  | 'temporary-close'
  | 'rename'
  | 'parent-sync'
  | 'parent-close'
  | 'temporary-cleanup';

export type ActivationPersistenceFailpoint = (
  step: ActivationPersistenceStep
) => void | Promise<void>;

export type ActivationStatePaths = Readonly<{
  state: string;
  lock: string;
}>;

export class AssemblyActivationFilePersistence {
  private canonicalRoot: Promise<string> | undefined;

  constructor(
    private readonly artifactRoot: string,
    private readonly failpoint?: ActivationPersistenceFailpoint
  ) {}

  async paths(environment: string): Promise<ActivationStatePaths> {
    if (!/^[A-Za-z0-9._-]{1,200}$/.test(environment) || environment === '.' || environment === '..') {
      throw new Error('invalid activation environment');
    }
    const root = await (this.canonicalRoot ??= realpath(resolve(this.artifactRoot)));
    const state = resolve(root, 'environments', environment, 'activation.json');
    const pathRelative = relative(root, state);
    if (pathRelative.startsWith(`..${sep}`) || pathRelative === '..') {
      throw new Error('activation state path escapes artifact root');
    }
    const metadata = await lstat(state);
    if (metadata.isSymbolicLink()) {
      throw new Error('activation state path must not be a symlink');
    }
    if (!metadata.isFile()) {
      throw new Error('activation state path must be a file');
    }
    const parent = dirname(state);
    if (await realpath(parent) !== parent) {
      throw new Error('activation state parent must not contain symlinks');
    }
    return { state, lock: join(parent, '.activation.lock') };
  }

  async read(environment: string): Promise<EnvironmentActivationState> {
    const { state: path } = await this.paths(environment);
    const bytes = await readFile(path);
    const state = decodeRawEnvironmentActivationState(bytes);
    if (state.environment !== environment) {
      throw new Error('activation state environment does not match its canonical path');
    }
    if (!bytes.equals(Buffer.from(canonicalActivationJson(state)))) {
      throw new Error('activation state is not canonical JSON');
    }
    return state;
  }

  async replace(
    environment: string,
    state: EnvironmentActivationState
  ): Promise<void> {
    const { state: destination } = await this.paths(environment);
    const temporary = join(
      dirname(destination),
      `.activation.${process.pid}.${randomToken()}.tmp`
    );
    let handle: Awaited<ReturnType<typeof open>> | undefined;
    let parent: Awaited<ReturnType<typeof open>> | undefined;
    let temporaryIdentity: { device: string; inode: string } | undefined;
    let renamed = false;
    let primaryError: unknown;
    let failed = false;
    try {
      await this.reach('temporary-open');
      handle = await open(temporary, 'wx', 0o600);
      const temporaryStats = await handle.stat({ bigint: true });
      temporaryIdentity = {
        device: temporaryStats.dev.toString(),
        inode: temporaryStats.ino.toString()
      };
      await this.reach('temporary-write');
      await handle.writeFile(canonicalActivationJson(state));
      await this.reach('temporary-sync');
      await handle.sync();
      await handle.close();
      handle = undefined;
      await this.reach('rename');
      await rename(temporary, destination);
      renamed = true;
      parent = await open(dirname(destination), 'r');
      await this.reach('parent-sync');
      await parent.sync();
    } catch (error) {
      primaryError = error;
      failed = true;
    }

    const cleanupErrors: unknown[] = [];
    if (handle !== undefined) {
      await captureCleanup(cleanupErrors, async () => {
        await handle?.close();
        await this.reach('temporary-close');
      });
    }
    if (parent !== undefined) {
      await captureCleanup(cleanupErrors, async () => {
        await parent?.close();
        await this.reach('parent-close');
      });
    }
    if (temporaryIdentity !== undefined && !renamed) {
      await captureCleanup(cleanupErrors, async () => {
        await removeOwnedTemporary(temporary, temporaryIdentity);
        await this.reach('temporary-cleanup');
      });
    }
    if (failed) {
      throwPrimaryWithCleanup(
        primaryError,
        cleanupErrors,
        'activation state persistence and cleanup both failed'
      );
    }
    throwCleanupErrors(cleanupErrors, 'activation state persistence cleanup failed');
  }

  private async reach(step: ActivationPersistenceStep): Promise<void> {
    await this.failpoint?.(step);
  }
}

export function canonicalActivationJson(state: EnvironmentActivationState): string {
  return JSON.stringify(sortJsonValue(decodeEnvironmentActivationState(state)));
}

function sortJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortJsonValue);
  }
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, sortJsonValue(nested)])
    );
  }
  return value;
}

function randomToken(): string {
  return randomBytes(16).toString('hex');
}

async function removeOwnedTemporary(
  path: string,
  expected: { device: string; inode: string }
): Promise<void> {
  let metadata: Awaited<ReturnType<typeof lstat>>;
  try {
    metadata = await lstat(path, { bigint: true });
  } catch (error) {
    if (isNodeError(error, 'ENOENT')) {
      return;
    }
    throw error;
  }
  if (
    metadata.isSymbolicLink() ||
    metadata.dev.toString() !== expected.device ||
    metadata.ino.toString() !== expected.inode
  ) {
    throw new Error('activation temporary identity changed; refusing to remove foreign file');
  }
  await unlink(path);
}

async function captureCleanup(
  errors: unknown[],
  cleanup: () => Promise<unknown>
): Promise<void> {
  try {
    await cleanup();
  } catch (error) {
    errors.push(error);
  }
}

function isNodeError(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code;
}
