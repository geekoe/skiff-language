import WebSocket from 'ws';

import type { JsonSchema, LoadedManifest, OperationManifest } from '../../src/manifest/types.js';
import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  isRecord,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RequestStartFrameHeader,
  type ResponseEndFrameHeader,
  type WebSocketConnectionPolicyFrameMetadata,
  type WebSocketContextCodecFrameMetadata,
  type RuntimeBinaryFrame,
  type RuntimeCapabilitiesEnvelope,
  type RuntimeCapabilitiesFrameHeader,
  type RuntimeRegisterFrameHeader,
  type RuntimeRegisterEnvelope
} from '../../src/protocol/envelope.js';
import {
  decodeOperationPayload,
  encodeRuntimePayload
} from './runtimePayloadCodec.js';
import { RuntimeDispatcher } from '../../src/router/runtimeDispatcher.js';
import {
  RuntimeEndpoint,
  type RuntimeEndpointListenOptions,
  type RuntimeEndpointListenResult
} from '../../src/router/runtimeEndpoint.js';
import {
  RuntimeRegistry,
  type RuntimeRegistryDependencies
} from '../../src/router/runtimeRegistry.js';

import { onceWithTimeout } from './events.js';
import { DEFAULT_TEST_BUILD_ID } from './manifests.js';

const resources: Array<{ close(): Promise<void> | void }> = [];
const JSON_CONTENT_TYPE = 'application/json; charset=utf-8';

export type RuntimeRequestFrame = RuntimeBinaryFrame<RequestStartFrameHeader>;

export interface RuntimeRouter {
  dispatcher: RuntimeDispatcher;
  endpoint: RuntimeEndpoint;
  registry: RuntimeRegistry;
  close(): Promise<void>;
}

export function createRuntimeRouter(
  dependencies: RuntimeRegistryDependencies = {}
): RuntimeRouter {
  const registry = new RuntimeRegistry(dependencies);
  const endpoint = new RuntimeEndpoint({ registry });
  const dispatcher = new RuntimeDispatcher({
    registry,
    frameSender: endpoint
  });
  endpoint.setDispatcher(dispatcher);
  return {
    dispatcher,
    endpoint,
    registry,
    close: () => endpoint.close()
  };
}

export async function listenRuntimeRouter(
  runtime: RuntimeRouter,
  options: RuntimeEndpointListenOptions = { port: 0 }
): Promise<RuntimeEndpointListenResult> {
  return await runtime.endpoint.listen(options);
}

export interface RequestStartEnvelope extends Omit<RequestStartFrameHeader, 'schemaVersion' | 'type'> {
  type: 'request.start';
  args: Record<string, unknown>;
}

function isRuntimeRequestFrame(frame: RuntimeBinaryFrame): frame is RuntimeRequestFrame {
  return frame.header.type === 'request.start';
}

export function trackResource<T extends { close(): Promise<void> | void }>(resource: T): T {
  resources.push(resource);
  return resource;
}

export async function closeTrackedResources(): Promise<void> {
  while (resources.length > 0) {
    const resource = resources.pop();
    await resource?.close();
  }
}

export async function openRegisteredRuntime(
  registryUrl: string,
  register: RuntimeRegisterEnvelope
): Promise<WebSocket> {
  return await openBinaryRegisteredRuntime(registryUrl, register);
}

export async function openBinaryRegisteredRuntime(
  registryUrl: string,
  register: RuntimeRegisterEnvelope
): Promise<WebSocket> {
  const ws = new WebSocket(registryUrl);
  trackResource({ close: () => ws.close() });
  await onceWithTimeout(ws, 'open', `${register.runtimeId} socket open`);
  const registered = waitForBinaryRuntimeRegistered(ws, register.runtimeId);
  const normalizedRegister = {
    ...register,
    revisionId: canonicalTestRevisionId(register.revisionId)
  };
  const { type: _type, ...metadata } = normalizedRegister;
  const header: RuntimeRegisterFrameHeader = {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'runtime.register',
    ...metadata
  };
  ws.send(encodeRuntimeFrame(header));
  await registered;
  return ws;
}

export async function openRuntimeCapabilities(
  registryUrl: string,
  capabilities: RuntimeCapabilitiesEnvelope
): Promise<WebSocket> {
  const ws = new WebSocket(registryUrl);
  trackResource({ close: () => ws.close() });
  await onceWithTimeout(ws, 'open', `${capabilities.runtimeId} socket open`);
  const { type: _type, ...metadata } = capabilities;
  const header: RuntimeCapabilitiesFrameHeader = {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'runtime.capabilities',
    ...metadata
  };
  ws.send(encodeRuntimeFrame(header));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return ws;
}

function canonicalTestRevisionId(revisionId: string): string {
  if (/^[0-9a-f]{64}$/.test(revisionId)) {
    return revisionId;
  }
  let hash = 0;
  for (let index = 0; index < revisionId.length; index += 1) {
    hash = (hash * 31 + revisionId.charCodeAt(index)) >>> 0;
  }
  return hash.toString(16).padStart(8, '0').repeat(8).slice(0, 64);
}

export class MockRuntime {
  private readonly operationByTarget = new Map<string, OperationManifest>();
  private readonly responseSchemaByRequestId = new Map<string, JsonSchema>();
  private readonly websocketContextCodecByRequestId = new Map<
    string,
    WebSocketContextCodecFrameMetadata
  >();

  private constructor(
    readonly ws: WebSocket,
    manifest?: LoadedManifest
  ) {
    for (const operation of manifest?.operations ?? []) {
      this.operationByTarget.set(operation.target, operation);
    }
  }

  static async register(
    registryUrl: string,
    register: RuntimeRegisterEnvelope,
    manifest?: LoadedManifest
  ): Promise<MockRuntime> {
    return new MockRuntime(await openRegisteredRuntime(registryUrl, register), manifest);
  }

  static async capabilities(
    registryUrl: string,
    capabilities: RuntimeCapabilitiesEnvelope
  ): Promise<MockRuntime> {
    return new MockRuntime(await openRuntimeCapabilities(registryUrl, capabilities));
  }

  collectRequests(count: number, label: string): Promise<RequestStartEnvelope[]> {
    return collectRuntimeRequests(this.ws, count, label, (frame) =>
      this.decodeRequestFrame(frame)
    );
  }

  collectRequestFrames(count: number, label: string): Promise<RuntimeRequestFrame[]> {
    return collectRuntimeRequestFrames(this.ws, count, label);
  }

  waitForRequestFrame(requestId: string): Promise<RuntimeRequestFrame> {
    return waitForRuntimeRequestFrame(this.ws, requestId);
  }

  onRequestFrame(handler: (request: RuntimeRequestFrame) => void): void {
    this.ws.on('message', (data) => {
      let frame: RuntimeBinaryFrame;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (!isRuntimeRequestFrame(frame)) {
        return;
      }
      handler(frame);
    });
  }

  waitForRequest(requestId: string): Promise<RequestStartEnvelope> {
    return waitForRuntimeRequest(this.ws, requestId, (frame) =>
      this.decodeRequestFrame(frame)
    );
  }

  onRequest(handler: (request: RequestStartEnvelope) => void): void {
    this.ws.on('message', (data) => {
      const frame = decodeRuntimeTestFrame(data);
      if (frame !== null && isRuntimeRequestFrame(frame)) {
        handler(this.decodeRequestFrame(frame));
      }
    });
  }

  sendResponse(requestId: string, payload: unknown): void {
    if (isWebSocketConnectResult(payload)) {
      this.sendWebSocketConnectResponse(requestId, payload);
      return;
    }
    const schema = this.responseSchemaByRequestId.get(requestId) ?? { type: 'any' };
    const responsePayload = schema.type === 'null' ? null : payload;
    const payloadBytes = encodeRuntimePayload(responsePayload, schema);
    this.ws.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId,
        payloadPresent: payloadBytes.byteLength > 0
      }, payloadBytes)
    );
  }

  sendBinaryResponse(requestId: string, payloadBytes: string | Buffer | Uint8Array): void {
    sendRuntimeBinaryResponse(this.ws, requestId, payloadBytes);
  }

  sendBinaryJsonResponse(requestId: string, payload: unknown): void {
    sendRuntimeBinaryResponse(this.ws, requestId, Buffer.from(JSON.stringify(payload)));
  }

  sendHttpFrameResponse(input: {
    requestId: string;
    status: number;
    headers?: Array<{ name: string; value: string }>;
    body?: string | Buffer | Uint8Array;
  }): void {
    const header: ResponseEndFrameHeader & {
      httpResponse: {
        status: number;
        headers: Array<{ name: string; value: string }>;
      };
    } = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: input.requestId,
      payloadPresent: input.body !== undefined,
      httpResponse: {
        status: input.status,
        headers: input.headers ?? []
      }
    };
    const body = input.body ?? new Uint8Array();
    const payloadBytes =
      typeof body === 'string'
        ? Buffer.from(body, 'utf8')
        : Buffer.isBuffer(body)
          ? body
          : body;
    this.ws.send(encodeRuntimeFrame(header, payloadBytes));
  }

  respondWithHttpFrame(input: {
    status: number;
    headers?: Array<{ name: string; value: string }>;
    body?: string | Buffer | Uint8Array;
  }): void {
    this.ws.on('message', (data) => {
      let frame: RuntimeBinaryFrame;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== 'request.start') {
        return;
      }
      const response = {
        requestId: frame.header.requestId,
        status: input.status
      };
      this.sendHttpFrameResponse(
        input.headers === undefined
          ? input.body === undefined
            ? response
            : { ...response, body: input.body }
          : input.body === undefined
            ? { ...response, headers: input.headers }
            : { ...response, headers: input.headers, body: input.body }
      );
    });
  }

  sendError(
    requestId: string,
    error: { code: string; message: string; details?: unknown }
  ): void {
    this.ws.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId,
        error
      })
    );
  }

  respondWithPayload(
    payload:
      | unknown
      | ((request: RequestStartEnvelope) => unknown)
  ): void {
    this.onRequest((request) => {
      this.sendResponse(
        request.requestId,
        typeof payload === 'function'
          ? (payload as (request: RequestStartEnvelope) => unknown)(request)
          : payload
      );
    });
  }

  respondWithBinaryJsonPayload(
    payload:
      | unknown
      | ((request: RuntimeRequestFrame) => unknown)
  ): void {
    this.onRequestFrame((request) => {
      this.sendBinaryJsonResponse(
        request.header.requestId,
        typeof payload === 'function'
          ? (payload as (request: RuntimeRequestFrame) => unknown)(request)
          : payload
      );
    });
  }

  respondWithRuntimeId(runtimeId: string): void {
    this.respondWithPayload({ runtimeId });
  }

  respondWithBinaryRuntimeId(runtimeId: string): void {
    this.respondWithBinaryJsonPayload({ runtimeId });
  }

  respondHttp(input: {
    status: number;
    headers?: Array<{ name: string; value: string }>;
    body: unknown;
  }): void {
    this.respondWithHttpFrame({
      status: input.status,
      headers: input.headers ?? [],
      body:
        typeof input.body === 'string' || Buffer.isBuffer(input.body) || input.body instanceof Uint8Array
          ? input.body
          : JSON.stringify(input.body)
    });
  }

  respondHttpJson(
    value: unknown | ((request: RuntimeRequestFrame) => unknown),
    status = 200,
    headers: Array<{ name: string; value: string }> = []
  ): void {
    this.ws.on('message', (data) => {
      let request: RuntimeBinaryFrame;
      try {
        request = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (!isRuntimeRequestFrame(request)) {
        return;
      }
      this.sendHttpFrameResponse({
        requestId: request.header.requestId,
        status,
        headers: withDefaultContentType(headers, JSON_CONTENT_TYPE),
        body: JSON.stringify(
          typeof value === 'function'
            ? (value as (request: RuntimeRequestFrame) => unknown)(request)
            : value
        )
      });
    });
  }

  respondHttpEmpty(status = 204): void {
    this.respondWithHttpFrame({
      status,
      body: Buffer.alloc(0)
    });
  }

  respondRawHttpRuntime(runtimeId: string): void {
    this.respondHttpJson((request: RuntimeRequestFrame) => ({
      buildId: request.header.buildId,
      protocolIdentity: request.header.serviceProtocolIdentity,
      runtimeId
    }), 200, [
      {
        name: 'content-type',
        value: JSON_CONTENT_TYPE
      }
    ]);
  }

  respondWithActivationIdentity(): void {
    this.respondHttpJson((request: RuntimeRequestFrame) =>
      request.header.activationIdentity !== undefined
        ? { activationIdentity: request.header.activationIdentity }
        : {}
    );
  }

  respondWebSocketAccept(
    input:
      | {
          userId: string;
          deviceId?: string;
          platform?: string;
          clientVersion?: string;
          language?: string;
          connectionPolicy?: WebSocketConnectionPolicyFrameMetadata;
        }
      | ((request: RequestStartEnvelope) => {
          userId: string;
          deviceId?: string;
          platform?: string;
          clientVersion?: string;
          language?: string;
          connectionPolicy?: WebSocketConnectionPolicyFrameMetadata;
        })
  ): void {
    this.onRequest((request) => {
      const accepted = typeof input === 'function' ? input(request) : input;
      const deviceId = accepted.deviceId ?? accepted.userId;
      this.sendResponse(request.requestId, {
        tag: 'accept',
        context: {
          userId: accepted.userId,
          deviceId,
          platform: accepted.platform ?? 'web',
          clientVersion: accepted.clientVersion ?? '1.0.0',
          language: accepted.language ?? 'en'
        },
        identity: accepted.userId,
        ...(accepted.connectionPolicy !== undefined
          ? { connectionPolicy: accepted.connectionPolicy }
          : {})
      });
    });
  }

  private decodeRequestFrame(frame: RuntimeRequestFrame): RequestStartEnvelope {
    const operation = this.operationByTarget.get(frame.header.target);
    const args =
      operation === undefined || frame.header.websocketAdapter !== undefined
        ? {}
        : decodeOperationPayload(frame.payloadBytes, operation.parameters);
    if (operation !== undefined) {
      this.responseSchemaByRequestId.set(frame.header.requestId, operation.response);
    }
    const contextExpectation = frame.header.websocketAdapter?.contextExpectation;
    if (contextExpectation?.kind === 'typed') {
      this.websocketContextCodecByRequestId.set(frame.header.requestId, {
        operationAbiId: contextExpectation.connectOperationAbiId,
        contextTypeIdentity: contextExpectation.contextTypeIdentity
      });
    }
    const { schemaVersion: _schemaVersion, ...header } = frame.header;
    return {
      ...header,
      args
    };
  }

  private sendWebSocketConnectResponse(
    requestId: string,
    payload: WebSocketConnectResultFixture
  ): void {
    if (payload.tag === 'reject') {
      this.ws.send(
        encodeRuntimeFrame({
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId,
          payloadPresent: false,
          websocketConnect: {
            result: 'reject',
            code: payload.code,
            reason: payload.reason,
            contextPayloadPresent: false
          }
        })
      );
      return;
    }
    const contextBytes = Buffer.from(JSON.stringify(payload.context ?? null), 'utf8');
    const contextCodec = this.websocketContextCodecByRequestId.get(requestId) ?? {
      operationAbiId: 'operation:test:websocket-connect-context',
      contextTypeIdentity: 'type:test:websocket-context'
    };
    const businessIdentity =
      typeof payload.businessIdentity === 'string'
        ? payload.businessIdentity
        : typeof payload.identity === 'string'
          ? payload.identity
          : undefined;
    this.ws.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId,
          payloadPresent: contextBytes.byteLength > 0,
          websocketConnect: {
            result: 'accept',
            ...(businessIdentity !== undefined ? { businessIdentity } : {}),
            ...(payload.connectionPolicy !== undefined
              ? { connectionPolicy: payload.connectionPolicy }
              : {}),
            contextPayloadPresent: contextBytes.byteLength > 0,
            ...(contextBytes.byteLength > 0
              ? {
                  contextCodec
                }
              : {})
          }
        },
        contextBytes
      )
    );
  }
}

type WebSocketConnectResultFixture =
  | {
      tag: 'accept';
      context?: unknown;
      identity?: string;
      businessIdentity?: string;
      connectionPolicy?: WebSocketConnectionPolicyFrameMetadata;
    }
  | {
      tag: 'reject';
      code: number;
      reason: string;
    };

function isWebSocketConnectResult(value: unknown): value is WebSocketConnectResultFixture {
  return (
    isRecord(value) &&
    (value.tag === 'accept' || value.tag === 'reject')
  );
}

export function collectRuntimeRequests(
  ws: WebSocket,
  count: number,
  label: string,
  decodeFrame?: (frame: RuntimeRequestFrame) => RequestStartEnvelope
): Promise<RequestStartEnvelope[]> {
  return new Promise((resolve, reject) => {
    const requests: RequestStartEnvelope[] = [];
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      const frame = decodeRuntimeTestFrame(data);
      if (frame !== null && isRuntimeRequestFrame(frame)) {
        requests.push(decodeFrame ? decodeFrame(frame) : requestEnvelopeFromFrame(frame));
        if (requests.length === count) {
          cleanup();
          resolve(requests);
        }
        return;
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

export function collectRuntimeRequestFrames(
  ws: WebSocket,
  count: number,
  label: string
): Promise<RuntimeRequestFrame[]> {
  return new Promise((resolve, reject) => {
    const frames: RuntimeRequestFrame[] = [];
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: RuntimeBinaryFrame;
      try {
        frame = decodeRuntimeFrame(data);
      } catch (error) {
        cleanup();
        reject(error);
        return;
      }
      if (!isRuntimeRequestFrame(frame)) {
        return;
      }
      frames.push(frame);
      if (frames.length === count) {
        cleanup();
        resolve(frames);
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

export function waitForRuntimeRegistered(ws: WebSocket, runtimeId: string): Promise<void> {
  return waitForBinaryRuntimeRegistered(ws, runtimeId);
}

export function waitForBinaryRuntimeRegistered(
  ws: WebSocket,
  runtimeId: string
): Promise<void> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${runtimeId} binary registration`));
    }, 1000);
    const onClose = () => {
      cleanup();
      reject(new Error(`${runtimeId} socket closed before binary registration`));
    };
    const onMessage = (data: WebSocket.RawData) => {
      let frame: RuntimeBinaryFrame;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (
        frame.header.type === 'runtime.registered' &&
        frame.header.runtimeId === runtimeId &&
        frame.payloadBytes.byteLength === 0
      ) {
        cleanup();
        resolve();
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('close', onClose);
      ws.off('message', onMessage);
    };
    ws.on('close', onClose);
    ws.on('message', onMessage);
  });
}

export function waitForRuntimeRequest(
  ws: WebSocket,
  requestId: string,
  decodeFrame?: (frame: RuntimeRequestFrame) => RequestStartEnvelope
): Promise<RequestStartEnvelope> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for runtime request ${requestId}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      const frame = decodeRuntimeTestFrame(data);
      if (
        frame !== null &&
        isRuntimeRequestFrame(frame) &&
        frame.header.requestId === requestId
      ) {
        cleanup();
        resolve(decodeFrame ? decodeFrame(frame) : requestEnvelopeFromFrame(frame));
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

export function waitForRuntimeRequestFrame(
  ws: WebSocket,
  requestId: string
): Promise<RuntimeRequestFrame> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for runtime request frame ${requestId}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: RuntimeBinaryFrame;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (
        isRuntimeRequestFrame(frame) &&
        frame.header.requestId === requestId
      ) {
        cleanup();
        resolve(frame);
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

export function sendRuntimeBinaryResponse(
  ws: WebSocket,
  requestId: string,
  payloadBytes: string | Buffer | Uint8Array
): void {
  const payload =
    typeof payloadBytes === 'string'
      ? Buffer.from(payloadBytes, 'utf8')
      : Buffer.isBuffer(payloadBytes)
        ? payloadBytes
        : Buffer.from(payloadBytes.buffer, payloadBytes.byteOffset, payloadBytes.byteLength);
  ws.send(
    encodeRuntimeFrame(
      {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId,
        payloadPresent: payload.byteLength > 0
      },
      payload
    )
  );
}

export function respondWithActivationIdentity(ws: WebSocket): void {
  ws.on('message', (data) => {
    const frame = decodeRuntimeTestFrame(data);
    if (frame?.header.type !== 'request.start') {
      return;
    }
    const body = JSON.stringify(
      frame.header.activationIdentity !== undefined
        ? { activationIdentity: frame.header.activationIdentity }
        : {}
    );
    ws.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId: frame.header.requestId,
          payloadPresent: true,
          httpResponse: {
            status: 200,
            headers: [{ name: 'content-type', value: JSON_CONTENT_TYPE }]
          }
        },
        Buffer.from(body)
      )
    );
  });
}

export function respondWithRawHttpRuntime(ws: WebSocket, runtimeId: string): void {
  ws.on('message', (data) => {
    const frame = decodeRuntimeTestFrame(data);
    if (frame?.header.type !== 'request.start') {
      return;
    }
    const body = JSON.stringify({
      buildId: frame.header.buildId,
      protocolIdentity: frame.header.serviceProtocolIdentity,
      runtimeId
    });
    ws.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId: frame.header.requestId,
          payloadPresent: true,
          httpResponse: {
            status: 200,
            headers: [{ name: 'content-type', value: JSON_CONTENT_TYPE }]
          }
        },
        Buffer.from(body)
      )
    );
  });
}

function withDefaultContentType(
  headers: Array<{ name: string; value: string }>,
  contentType: string
): Array<{ name: string; value: string }> {
  if (headers.some((header) => header.name.toLowerCase() === 'content-type')) {
    return headers;
  }
  return [{ name: 'content-type', value: contentType }, ...headers];
}

export function createRequestStart(input: {
  requestId: string;
  target: string;
  serviceId?: string;
  version?: string;
  serviceProtocolIdentity: string;
  operationAbiId?: string;
  buildId?: string;
  gatewayEntryIdentity?: string;
  activationIdentity?: string;
}): RequestStartEnvelope {
  const request: RequestStartEnvelope = {
    type: 'request.start',
    requestId: input.requestId,
    mode: 'unary',
    caller: {
      kind: 'gateway',
      target: 'gateway.hello.http.test'
    },
    target: input.target,
    operationAbiId: input.operationAbiId ?? `operation:test:${input.target}`,
    selector: `operation:${input.operationAbiId ?? `operation:test:${input.target}`}`,
    buildId: input.buildId ?? DEFAULT_TEST_BUILD_ID,
    serviceProtocolIdentity: input.serviceProtocolIdentity,
    trace: {
      traceId: `${input.requestId}-trace`,
      spanId: `${input.requestId}-span`
    },
    args: {}
  };
  if (input.gatewayEntryIdentity !== undefined) {
    request.gatewayEntryIdentity = input.gatewayEntryIdentity;
  }
  if (input.serviceId !== undefined) {
    request.serviceId = input.serviceId;
  }
  if (input.version !== undefined) {
    request.version = input.version;
  }
  if (input.activationIdentity !== undefined) {
    request.activationIdentity = input.activationIdentity;
  }
  return request;
}

export function waitForRouterControl(
  ws: WebSocket,
  fingerprint: string
): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for router control ${fingerprint}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      const envelope = decodeRuntimeTestMessage(data);
      if (
        isRecord(envelope) &&
        envelope.type === 'router.control' &&
        envelope.fingerprint === fingerprint
      ) {
        cleanup();
        resolve(envelope);
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

function decodeRuntimeTestMessage(data: WebSocket.RawData): unknown {
  return decodeRuntimeTestFrame(data)?.header ?? null;
}

function decodeRuntimeTestFrame(data: WebSocket.RawData): RuntimeBinaryFrame | null {
  try {
    return decodeRuntimeFrame(data);
  } catch {
    return null;
  }
}

function requestEnvelopeFromFrame(frame: RuntimeRequestFrame): RequestStartEnvelope {
  const { schemaVersion: _schemaVersion, ...header } = frame.header;
  return {
    ...header,
    type: 'request.start',
    args: {}
  };
}
