import { randomUUID } from 'node:crypto';
import type { IncomingMessage, ServerResponse } from 'node:http';

import { ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT } from '../protocol/assemblyActivationProtocol.js';
import { decodeRawAssemblyActivationRequest } from '../protocol/assemblyActivationRawCodec.js';
import type { AssemblyActivationCoordinator } from './assemblyActivationCoordinator.js';
import type { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import { assemblyHttpRequestHeader } from './assemblyHttpGateway.js';
import { toGatewayError } from './errors.js';
import type { RuntimeDispatcher } from './runtimeDispatcher.js';
import type { RuntimeRegistry } from './runtimeRegistry.js';
import {
  canonicalIngressHost,
  type RouterActiveAssemblySnapshotStore,
  type RuntimeAssemblyIngressSelector
} from './runtimeAssemblySnapshot.js';

const ACTIVATION_PATH = ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT.slice('POST '.length);

export interface AssemblyControlPlaneOptions {
  coordinator: AssemblyActivationCoordinator;
  dispatcher: RuntimeDispatcher;
  registry: AssemblyRuntimeRegistry;
  runtimeRegistry: Pick<RuntimeRegistry, 'capabilityConnectionsSnapshot'>;
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
        capabilityConnections: this.options.runtimeRegistry.capabilityConnectionsSnapshot(),
        replicas: this.options.registry.snapshot()
      });
      return true;
    }
    if (url.pathname === '/__skiff/test-dispatch') {
      await this.handleTestDispatch(request, response);
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

  private async handleTestDispatch(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<void> {
    if (request.method !== 'POST') {
      response.setHeader('allow', 'POST');
      this.writeJson(response, 405, {
        error: { code: 'MethodNotAllowed', message: 'test dispatch requires POST' }
      });
      return;
    }
    const body = decodeRuntimeAssemblyTestDispatch(await readBody(request));
    const snapshot = this.options.snapshots.get();
    const binding = snapshot.ingress.get(body.ingress);
    if (
      binding === undefined ||
      binding.contractOperationId !== body.contractOperationId
    ) {
      throw new Error(
        'runtime assembly test dispatch does not match an exact active ingress operation'
      );
    }
    const timeoutMs = body.timeoutMs ?? 30_000;
    const header = assemblyHttpRequestHeader({
      snapshot,
      binding,
      requestId: randomUUID(),
      timeoutMs,
      httpRequest: {
        method: body.ingress.method!,
        url: `http://${body.ingress.host}${body.ingress.path}`,
        path: body.ingress.path,
        query: [],
        headers: []
      },
      testEffectsEnabled: body.testEffectsEnabled,
      testEffectDoubles: body.testEffectDoubles,
      callerTarget: '__skiff.runtime-assembly-test-dispatch'
    });
    const runtimeResponse = await this.options.dispatcher.dispatchBinary(
      {
        header,
        payloadBytes: Buffer.from(body.payloadBase64 ?? '', 'base64')
      },
      timeoutMs
    );
    this.writeJson(response, 200, {
      ok: true,
      header: runtimeResponse.header,
      payloadBase64: Buffer.from(runtimeResponse.payloadBytes).toString('base64')
    });
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

interface RuntimeAssemblyTestDispatch {
  contractOperationId: string;
  ingress: RuntimeAssemblyIngressSelector & {
    protocol: 'http';
    method: string;
  };
  payloadBase64?: string;
  testEffectsEnabled: boolean;
  testEffectDoubles: Record<
    string,
    Array<{ expectRequest?: unknown; response: unknown }>
  >;
  timeoutMs?: number;
}

function decodeRuntimeAssemblyTestDispatch(
  bytes: Buffer
): RuntimeAssemblyTestDispatch {
  const value: unknown = JSON.parse(bytes.toString('utf8'));
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('runtime assembly test dispatch body must be an object');
  }
  const body = value as Record<string, unknown>;
  const supported = new Set([
    'kind',
    'contractOperationId',
    'ingress',
    'payloadBase64',
    'testEffectsEnabled',
    'testEffectDoubles',
    'timeoutMs'
  ]);
  const unknown = Object.keys(body).find((field) => !supported.has(field));
  if (unknown !== undefined || body.kind !== 'runtimeAssembly') {
    throw new Error(
      unknown === undefined
        ? 'runtime assembly test dispatch kind must be runtimeAssembly'
        : `runtime assembly test dispatch does not support ${unknown}`
    );
  }
  if (
    typeof body.contractOperationId !== 'string' ||
    body.ingress === null ||
    typeof body.ingress !== 'object' ||
    Array.isArray(body.ingress) ||
    typeof body.testEffectsEnabled !== 'boolean' ||
    body.testEffectDoubles === null ||
    typeof body.testEffectDoubles !== 'object' ||
    Array.isArray(body.testEffectDoubles)
  ) {
    throw new Error('runtime assembly test dispatch has invalid canonical fields');
  }
  const ingress = body.ingress as Record<string, unknown>;
  if (
    ingress.protocol !== 'http' ||
    typeof ingress.host !== 'string' ||
    typeof ingress.method !== 'string' ||
    typeof ingress.path !== 'string'
  ) {
    throw new Error('runtime assembly test dispatch requires an HTTP ingress selector');
  }
  const selector = {
    protocol: 'http' as const,
    host: canonicalIngressHost(ingress.host),
    method: ingress.method.toUpperCase(),
    path: ingress.path
  };
  return {
    contractOperationId: body.contractOperationId,
    ingress: selector,
    ...(typeof body.payloadBase64 === 'string'
      ? { payloadBase64: body.payloadBase64 }
      : {}),
    testEffectsEnabled: body.testEffectsEnabled,
    testEffectDoubles: body.testEffectDoubles as RuntimeAssemblyTestDispatch['testEffectDoubles'],
    ...(typeof body.timeoutMs === 'number' &&
    Number.isSafeInteger(body.timeoutMs) &&
    body.timeoutMs > 0
      ? { timeoutMs: body.timeoutMs }
      : {})
  };
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
