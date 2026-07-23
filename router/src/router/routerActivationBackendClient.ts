import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';

import {
  ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
  decodeEnvironmentActivationState,
  type AssemblyActivationRequest,
  type EnvironmentActivationState,
  type PendingActivation,
  type RuntimeAssemblyRef
} from '../protocol/assemblyActivationProtocol.js';
import { parseStrictActivationJson } from '../protocol/strictActivationJson.js';
import type { AssemblyActivationStateStore } from './assemblyActivationStateStore.js';
import {
  decodeRouterSnapshot,
  type LoadedRuntimeAssembly,
  type RuntimeAssemblySnapshotLoader
} from './runtimeAssemblySnapshot.js';

const MAX_FRAME_BYTES = 1024 * 1024;

export interface RouterActivationBackendClientOptions {
  executablePath: string;
  args?: readonly string[];
}

export class RouterActivationBackendClient
  implements AssemblyActivationStateStore, RuntimeAssemblySnapshotLoader
{
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly pending = new Map<string, {
    resolve(value: unknown): void;
    reject(error: Error): void;
  }>();
  private nextRequestId = 1;
  private stdout = Buffer.alloc(0);
  private closed = false;

  constructor(options: RouterActivationBackendClientOptions) {
    this.child = spawn(options.executablePath, options.args ?? [], {
      stdio: ['pipe', 'pipe', 'pipe']
    });
    this.child.stdout.on('data', (chunk: Buffer) => this.receive(chunk));
    this.child.stderr.resume();
    this.child.on('error', (error) => this.failAll(
      new Error(`router activation backend failed to start: ${options.executablePath}`, {
        cause: error
      })
    ));
    this.child.on('close', (code, signal) => this.failAll(
      new Error(`router activation backend exited with ${signal ?? code ?? 'unknown status'}`)
    ));
  }

  async read(environment: string): Promise<EnvironmentActivationState> {
    return decodeState(await this.invoke('read', { environment }));
  }

  async prepare(
    request: AssemblyActivationRequest,
    participantReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    return decodeState(await this.invoke('prepare', {
      environment: request.environment,
      activationId: request.activationId,
      expectedGeneration: request.expectedGeneration,
      candidateGeneration: request.expectedGeneration + 1,
      assembly: request.assembly,
      participantReplicaIds: [...participantReplicaIds]
    }));
  }

  async commit(
    environment: string,
    pending: PendingActivation,
    connectedReplicaIds: readonly string[],
    preparedReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    return decodeState(await this.invoke('commit', {
      environment,
      activationId: pending.activationId,
      connectedReplicaIds: [...connectedReplicaIds],
      preparedReplicaIds: [...preparedReplicaIds]
    }));
  }

  async abort(
    environment: string,
    pending: PendingActivation
  ): Promise<EnvironmentActivationState> {
    return decodeState(await this.invoke('abort', {
      environment,
      activationId: pending.activationId
    }));
  }

  async load(ref: RuntimeAssemblyRef): Promise<LoadedRuntimeAssembly> {
    const snapshot = await this.invoke('read-snapshot', { assembly: ref });
    return decodeRouterSnapshot(snapshot, ref).assembly;
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.child.stdin.end();
    if (this.child.exitCode !== null || this.child.signalCode !== null) return;
    await new Promise<void>((resolve) => {
      const terminate = setTimeout(() => this.child.kill('SIGTERM'), 1_000);
      terminate.unref();
      this.child.once('close', () => {
        clearTimeout(terminate);
        resolve();
      });
    });
  }

  private invoke(operation: string, payload: unknown): Promise<unknown> {
    if (this.closed) {
      return Promise.reject(new Error('router activation backend client is closed'));
    }
    const requestId = `router-${this.nextRequestId++}`;
    return new Promise((resolve, reject) => {
      this.pending.set(requestId, { resolve, reject });
      this.child.stdin.write(`${JSON.stringify({ requestId, operation, payload })}\n`, (error) => {
        if (error === null || error === undefined) return;
        this.pending.delete(requestId);
        reject(new Error('failed to write router activation backend request', { cause: error }));
      });
    });
  }

  private receive(chunk: Buffer): void {
    this.stdout = Buffer.concat([this.stdout, chunk]);
    if (this.stdout.byteLength > MAX_FRAME_BYTES && !this.stdout.includes(0x0a)) {
      this.failAll(new Error('router activation backend frame exceeded the bounded limit'));
      this.child.kill('SIGKILL');
      return;
    }
    for (;;) {
      const newline = this.stdout.indexOf(0x0a);
      if (newline < 0) return;
      const frame = this.stdout.subarray(0, newline);
      this.stdout = this.stdout.subarray(newline + 1);
      if (frame.byteLength === 0 || frame.byteLength > MAX_FRAME_BYTES) {
        this.failAll(new Error('router activation backend returned an invalid bounded frame'));
        this.child.kill('SIGKILL');
        return;
      }
      try {
        this.dispatch(parseStrictActivationJson(frame));
      } catch (error) {
        this.failAll(new Error('router activation backend returned invalid strict JSON', {
          cause: error
        }));
        this.child.kill('SIGKILL');
        return;
      }
    }
  }

  private dispatch(input: unknown): void {
    if (!isRecord(input) || typeof input.requestId !== 'string') {
      this.failAll(new Error('router activation backend response is missing requestId'));
      this.child.kill('SIGKILL');
      return;
    }
    const pending = this.pending.get(input.requestId);
    if (pending === undefined) {
      this.failAll(new Error(`router activation backend returned unknown requestId ${input.requestId}`));
      this.child.kill('SIGKILL');
      return;
    }
    this.pending.delete(input.requestId);
    if (isRecord(input.error)) {
      pending.reject(new Error(
        `router activation backend ${String(input.error.code)}: ${String(input.error.message)}`
      ));
      return;
    }
    if ('snapshot' in input) {
      pending.resolve(input.snapshot);
      return;
    }
    pending.reject(new Error('router activation backend response has no outcome'));
  }

  private failAll(error: Error): void {
    this.closed = true;
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
  }
}

function decodeState(input: unknown): EnvironmentActivationState {
  if (!isRecord(input) || typeof input.environment !== 'string') {
    throw new Error('router activation backend returned invalid activation state');
  }
  if (!isRecord(input.committed)) {
    throw new Error(`router activation backend environment ${input.environment} is not initialized`);
  }
  return decodeEnvironmentActivationState({
    schemaVersion: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
    environment: input.environment,
    committed: input.committed,
    pending: input.pending ?? null
  });
}

function isRecord(input: unknown): input is Record<string, unknown> {
  return typeof input === 'object' && input !== null && !Array.isArray(input);
}
