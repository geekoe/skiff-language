import { randomUUID } from 'node:crypto';
import type { IncomingMessage, ServerResponse } from 'node:http';

import { ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT } from '../protocol/assemblyActivationProtocol.js';
import { decodeRawAssemblyActivationRequest } from '../protocol/assemblyActivationRawCodec.js';
import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type HttpRequestFrameMetadata
} from '../protocol/envelope.js';
import type {
  RuntimeAssemblyRequestRoutingFrameHeader,
  RuntimeAssemblyRequestStartFrameHeader
} from '../protocol/runtimeAssemblyRequest.js';
import { validateRuntimeAssemblyRequestStartFrameHeader } from '../protocol/runtimeProtocol.js';
import type { AssemblyActivationCoordinator } from './assemblyActivationCoordinator.js';
import type { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import { assemblyTestHttpRequestHeader } from './assemblyHttpGateway.js';
import { toGatewayError } from './errors.js';
import type { RuntimeDispatcher } from './runtimeDispatcher.js';
import type { RuntimeRegistry } from './runtimeRegistry.js';
import {
  type RouterActiveAssemblySnapshot,
  type RouterActiveAssemblySnapshotStore,
  type RuntimeAssemblyIngressBinding
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
    const binding = exactTestDispatchBinding(body, snapshot);
    const header = assemblyTestHttpRequestHeader({
      snapshot,
      binding,
      requestId: randomUUID(),
      timeoutMs: body.timeoutMs,
      routing: body.routing,
      mode: body.mode,
      httpRequest: body.httpRequest
    });
    const runtimeResponse = await this.options.dispatcher.dispatchAssemblyTestBinary(
      {
        header,
        payloadBytes: body.payloadBytes
      },
      body.timeoutMs
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
  kind: 'test';
  routing: RuntimeAssemblyRequestRoutingFrameHeader;
  mode: RuntimeAssemblyRequestStartFrameHeader['mode'];
  httpRequest: HttpRequestFrameMetadata;
  payloadBytes: Buffer;
  timeoutMs: number;
}

function decodeRuntimeAssemblyTestDispatch(
  bytes: Buffer
): RuntimeAssemblyTestDispatch {
  const value: unknown = JSON.parse(bytes.toString('utf8'));
  const body = exactObject(value, 'runtime assembly test dispatch');
  exactFields(body, [
    'kind',
    'routing',
    'mode',
    'httpRequest',
    'payloadBase64',
    'timeoutMs'
  ], 'runtime assembly test dispatch');
  if (body.kind !== 'test') {
    throw new Error('runtime assembly test dispatch kind must be test');
  }
  const headerValidation = validateRuntimeAssemblyRequestStartFrameHeader({
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: '__skiff.test-control-decode',
    mode: body.mode,
    caller: { kind: 'gateway' },
    routing: body.routing,
    trace: {
      traceId: '__skiff.test-control-decode',
      spanId: '__skiff.test-control-decode'
    },
    httpRequest: body.httpRequest,
    testEffectsEnabled: true
  });
  if (!headerValidation.ok) {
    throw new Error(
      `runtime assembly test dispatch has invalid canonical fields: ${headerValidation.error}`
    );
  }
  if (typeof body.payloadBase64 !== 'string') {
    throw new Error('runtime assembly test dispatch payloadBase64 must be a string');
  }
  const payloadBytes = Buffer.from(body.payloadBase64, 'base64');
  if (payloadBytes.toString('base64') !== body.payloadBase64) {
    throw new Error(
      'runtime assembly test dispatch payloadBase64 must be canonical standard Base64'
    );
  }
  if (
    typeof body.timeoutMs !== 'number' ||
    !Number.isSafeInteger(body.timeoutMs) ||
    body.timeoutMs <= 0
  ) {
    throw new Error(
      'runtime assembly test dispatch timeoutMs must be a positive safe integer'
    );
  }
  return {
    kind: 'test',
    routing: headerValidation.envelope.routing,
    mode: headerValidation.envelope.mode,
    httpRequest: headerValidation.envelope.httpRequest,
    payloadBytes,
    timeoutMs: body.timeoutMs
  };
}

function exactObject(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function exactFields(
  value: Record<string, unknown>,
  fields: readonly string[],
  label: string
): void {
  const supported = new Set(fields);
  const unknown = Object.keys(value).find((field) => !supported.has(field));
  if (unknown !== undefined) {
    throw new Error(`${label} does not support ${unknown}`);
  }
  const missing = fields.find(
    (field) => !Object.prototype.hasOwnProperty.call(value, field)
  );
  if (missing !== undefined) {
    throw new Error(`${label} requires ${missing}`);
  }
}

function exactTestDispatchBinding(
  body: RuntimeAssemblyTestDispatch,
  snapshot: RouterActiveAssemblySnapshot
): RuntimeAssemblyIngressBinding {
  if (
    body.routing.assemblyIdentity !== snapshot.assembly.assemblyIdentity ||
    body.routing.assemblyGeneration !== snapshot.generation
  ) {
    throw new Error(
      'runtime assembly test dispatch does not match the exact active assembly generation'
    );
  }
  const binding = snapshot.ingress.get(body.routing.ingress);
  if (
    binding === undefined ||
    binding.selector.protocol !== body.routing.ingress.protocol ||
    binding.selector.host !== body.routing.ingress.host ||
    binding.selector.method !== body.routing.ingress.method ||
    binding.selector.path !== body.routing.ingress.path ||
    binding.gatewayEntryIdentity !== body.routing.gatewayEntryIdentity ||
    binding.operationMode !== body.mode
  ) {
    throw new Error(
      'runtime assembly test dispatch does not match the exact active gateway binding'
    );
  }
  return binding;
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
