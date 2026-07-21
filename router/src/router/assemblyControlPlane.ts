import type { IncomingMessage, ServerResponse } from 'node:http';

import { ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT } from '../protocol/assemblyActivationProtocol.js';
import { decodeRawAssemblyActivationRequest } from '../protocol/assemblyActivationRawCodec.js';
import type { AssemblyActivationCoordinator } from './assemblyActivationCoordinator.js';
import type { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import { toGatewayError } from './errors.js';
import type { RouterActiveAssemblySnapshotStore } from './runtimeAssemblySnapshot.js';

const ACTIVATION_PATH = ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT.slice('POST '.length);

export interface AssemblyControlPlaneOptions {
  coordinator: AssemblyActivationCoordinator;
  registry: AssemblyRuntimeRegistry;
  snapshots: RouterActiveAssemblySnapshotStore;
}

export class AssemblyControlPlane {
  constructor(private readonly options: AssemblyControlPlaneOptions) {}

  async handleRequest(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<boolean> {
    const url = new URL(request.url ?? '/', 'http://router.local');
    if (url.pathname === '/__router/health') {
      if (request.method !== 'GET') {
        response.setHeader('allow', 'GET');
        this.writeJson(response, 405, {
          error: { code: 'MethodNotAllowed', message: 'router health requires GET' }
        });
        return true;
      }
      const snapshot = this.options.snapshots.get();
      const state = this.options.coordinator.activationState();
      this.writeJson(response, 200, {
        ok: true,
        activeAssembly: {
          environment: snapshot.environment,
          generation: snapshot.generation,
          assemblyIdentity: snapshot.assembly.assemblyIdentity,
          ingressCount: snapshot.ingress.values().length
        },
        pendingActivation: state.pending,
        replicas: this.options.registry.snapshot()
      });
      return true;
    }
    if (url.pathname !== ACTIVATION_PATH) {
      return false;
    }
    if (request.method !== 'POST') {
      response.setHeader('allow', 'POST');
      this.writeJson(response, 405, {
        error: { code: 'MethodNotAllowed', message: 'assembly activation requires POST' }
      });
      return true;
    }
    const activation = decodeRawAssemblyActivationRequest(await readBody(request));
    const state = await this.options.coordinator.activate(activation);
    const snapshot = this.options.snapshots.get();
    this.writeJson(response, 200, {
      ok: true,
      committed: state.committed,
      activeAssembly: {
        environment: snapshot.environment,
        generation: snapshot.generation,
        assemblyIdentity: snapshot.assembly.assemblyIdentity
      },
      replicas: this.options.registry.snapshot()
    });
    return true;
  }

  async handleRequestWithErrors(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<boolean> {
    try {
      return await this.handleRequest(request, response);
    } catch (error: unknown) {
      const gatewayError = toGatewayError(error);
      const statusCode = gatewayError.statusCode === 500 ? classifyActivationError(error) : gatewayError.statusCode;
      this.writeJson(response, statusCode, {
        error: {
          code: statusCode === 503 ? 'AssemblyParticipantsUnavailable' : 'AssemblyActivationRejected',
          message: error instanceof Error ? error.message : gatewayError.message
        }
      });
      return true;
    }
  }

  private writeJson(response: ServerResponse, statusCode: number, value: unknown): void {
    if (response.headersSent) {
      response.end();
      return;
    }
    response.statusCode = statusCode;
    response.setHeader('content-type', 'application/json; charset=utf-8');
    response.end(JSON.stringify(value));
  }
}

async function readBody(request: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
    size += buffer.byteLength;
    if (size > 1024 * 1024) {
      throw new Error('assembly activation request body exceeds 1 MiB');
    }
    chunks.push(buffer);
  }
  return Buffer.concat(chunks);
}

function classifyActivationError(error: unknown): number {
  const message = error instanceof Error ? error.message : '';
  if (message.includes('healthy participant') || message.includes('disconnected')) {
    return 503;
  }
  if (message.includes('timed out')) {
    return 504;
  }
  if (message.includes('invalid') || message.includes('must be') || message.includes('JSON')) {
    return 400;
  }
  return 409;
}
