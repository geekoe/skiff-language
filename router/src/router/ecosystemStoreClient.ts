import { spawn } from 'node:child_process';

import {
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

const DEFAULT_ADAPTER_TIMEOUT_MS = 20_000;
const MAX_ADAPTER_OUTPUT_BYTES = 64 * 1024 * 1024;

export interface EcosystemStoreClientOptions {
  compilerPath: string;
  artifactRoot: string;
  timeoutMs?: number;
}

/**
 * The Router's only production boundary to the canonical ecosystem store.
 * Path resolution, identity validation, locking and CAS remain owned by the
 * Rust adapter.
 */
export class EcosystemStoreClient
  implements AssemblyActivationStateStore, RuntimeAssemblySnapshotLoader
{
  constructor(private readonly options: EcosystemStoreClientOptions) {}

  async ensureEnvironmentBootstrap(environment: string): Promise<EnvironmentActivationState> {
    return this.readState({
      operation: 'ensureEnvironmentBootstrap',
      environment
    });
  }

  async read(environment: string): Promise<EnvironmentActivationState> {
    return this.readState({ operation: 'readEnvironment', environment });
  }

  async prepare(
    request: AssemblyActivationRequest,
    participantReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    return this.readState({
      operation: 'prepareEnvironment',
      environment: request.environment,
      activationId: request.activationId,
      expectedGeneration: request.expectedGeneration,
      candidateGeneration: request.expectedGeneration + 1,
      assembly: request.assembly,
      participantReplicaIds: [...participantReplicaIds]
    });
  }

  async abort(
    environment: string,
    pending: PendingActivation
  ): Promise<EnvironmentActivationState> {
    return this.readState({
      operation: 'abortEnvironment',
      environment,
      activationId: pending.activationId,
      expectedGeneration: pending.expectedGeneration
    });
  }

  async commit(
    environment: string,
    pending: PendingActivation,
    connectedReplicaIds: readonly string[],
    preparedReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    return this.readState({
      operation: 'commitEnvironment',
      environment,
      activationId: pending.activationId,
      expectedGeneration: pending.expectedGeneration,
      candidateGeneration: pending.candidateGeneration,
      assembly: pending.assembly,
      connectedReplicaIds: [...connectedReplicaIds],
      preparedReplicaIds: [...preparedReplicaIds]
    });
  }

  async load(ref: RuntimeAssemblyRef): Promise<LoadedRuntimeAssembly> {
    const response = await this.invoke({
      operation: 'readRouterSnapshot',
      assembly: ref
    });
    return decodeRouterSnapshot(response, ref).assembly;
  }

  private async readState(request: EcosystemStoreRequest): Promise<EnvironmentActivationState> {
    return decodeEnvironmentActivationState(await this.invoke(request));
  }

  private invoke(request: EcosystemStoreRequest): Promise<unknown> {
    const timeoutMs = this.options.timeoutMs ?? DEFAULT_ADAPTER_TIMEOUT_MS;
    return new Promise((resolve, reject) => {
      const child = spawn(
        this.options.compilerPath,
        ['__ecosystem-store', '--artifact-root', this.options.artifactRoot],
        { stdio: ['pipe', 'pipe', 'pipe'] }
      );
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      let stdoutBytes = 0;
      let stderrBytes = 0;
      let settled = false;
      const timeout = setTimeout(() => {
        if (settled) return;
        settled = true;
        child.kill('SIGKILL');
        reject(new Error(`ecosystem-store adapter timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      const finish = (action: () => void) => {
        if (settled) return;
        settled = true;
        clearTimeout(timeout);
        action();
      };
      child.stdout.on('data', (chunk: Buffer) => {
        stdoutBytes += chunk.byteLength;
        if (stdoutBytes > MAX_ADAPTER_OUTPUT_BYTES) {
          finish(() => {
            child.kill('SIGKILL');
            reject(new Error('ecosystem-store adapter stdout exceeded the bounded limit'));
          });
          return;
        }
        stdout.push(chunk);
      });
      child.stderr.on('data', (chunk: Buffer) => {
        stderrBytes += chunk.byteLength;
        if (stderrBytes > MAX_ADAPTER_OUTPUT_BYTES) {
          finish(() => {
            child.kill('SIGKILL');
            reject(new Error('ecosystem-store adapter stderr exceeded the bounded limit'));
          });
          return;
        }
        stderr.push(chunk);
      });
      child.stdin.on('error', (error) => {
        finish(() => {
          child.kill('SIGKILL');
          reject(new Error('failed to write ecosystem-store adapter request', {
            cause: error
          }));
        });
      });
      child.on('error', (error) => {
        finish(() => reject(new Error(
          `failed to start ecosystem-store adapter ${this.options.compilerPath}`,
          { cause: error }
        )));
      });
      child.on('close', (code, signal) => {
        finish(() => {
          if (code !== 0) {
            const detail = Buffer.concat(stderr).toString('utf8').trim();
            reject(new Error(
              `ecosystem-store adapter failed with ${signal ?? code}: ${detail || 'no stderr'}`
            ));
            return;
          }
          try {
            resolve(parseStrictActivationJson(Buffer.concat(stdout)));
          } catch (error) {
            reject(new Error('ecosystem-store adapter returned invalid strict JSON', {
              cause: error
            }));
          }
        });
      });
      child.stdin.end(`${JSON.stringify(request)}\n`);
    });
  }
}

type EcosystemStoreRequest =
  | { operation: 'ensureEnvironmentBootstrap'; environment: string }
  | { operation: 'readEnvironment'; environment: string }
  | {
      operation: 'prepareEnvironment';
      environment: string;
      activationId: string;
      expectedGeneration: number;
      candidateGeneration: number;
      assembly: RuntimeAssemblyRef;
      participantReplicaIds: string[];
    }
  | {
      operation: 'abortEnvironment';
      environment: string;
      activationId: string;
      expectedGeneration: number;
    }
  | {
      operation: 'commitEnvironment';
      environment: string;
      activationId: string;
      expectedGeneration: number;
      candidateGeneration: number;
      assembly: RuntimeAssemblyRef;
      connectedReplicaIds: string[];
      preparedReplicaIds: string[];
    }
  | { operation: 'readRouterSnapshot'; assembly: RuntimeAssemblyRef };
