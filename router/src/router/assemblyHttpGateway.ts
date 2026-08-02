import { randomUUID } from 'node:crypto';
import {
  createServer,
  type IncomingMessage,
  type Server as HttpServer,
  type ServerResponse
} from 'node:http';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type HttpRequestFrameMetadata
} from '../protocol/envelope.js';
import type {
  RuntimeAssemblyRequestRoutingFrameHeader,
  RuntimeAssemblyRequestStartFrameHeader
} from '../protocol/runtimeAssemblyRequest.js';
import { validateRuntimeAssemblyRequestStartFrameHeader } from '../protocol/runtimeProtocol.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation
} from '../protocol/cancelReason.js';
import { GatewayError, toGatewayError } from './errors.js';
import type {
  RuntimeBinaryStreamHandlers,
  RuntimeDispatcher,
  RuntimeSelfIngressTestCorrelation
} from './runtimeDispatcher.js';
import {
  DEFAULT_HTTP_BACKPRESSURE_DRAIN_TIMEOUT_MS,
  HttpStreamResponseWriter,
  type HttpStreamLifecycleCounters
} from './httpStreamResponseWriter.js';
import {
  hasCorsOriginHeader,
  isCorsPreflightRequest,
  isCorsResponseHeader,
  writeAutomaticCorsHeaders,
  writeAutomaticCorsPreflightResponse
} from './httpCors.js';
import { readOriginFormUrlForGatewayMetadata } from './bind.js';
import {
  canonicalHttpHost,
  type RouterActiveAssemblySnapshot,
  type RouterActiveAssemblySnapshotStore,
  type RuntimeAssemblyIngressBinding
} from './runtimeAssemblySnapshot.js';
import { readServiceDeploymentSelector } from './serviceDeploymentSelection.js';

const DEFAULT_HTTP_REQUEST_TIMEOUT_MS = 120_000;
const MAX_JAVASCRIPT_DATE_MS = 8_640_000_000_000_000;
const MAX_NODE_TIMEOUT_MS = 2_147_483_647;
const TEST_CASE_CAPABILITY_HEADER = 'x-skiff-test-case-capability';
const TEST_CASE_PARENT_REQUEST_ID_HEADER =
  'x-skiff-test-case-parent-request-id';
const TEST_CASE_CORRELATION_TOKEN = /^[A-Za-z0-9_.:-]{1,256}$/;
const RESERVED_TEST_CASE_HEADERS = new Set([
  TEST_CASE_CAPABILITY_HEADER,
  TEST_CASE_PARENT_REQUEST_ID_HEADER
]);

export interface AssemblyHttpGatewayOptions {
  snapshots: RouterActiveAssemblySnapshotStore;
  dispatcher: RuntimeDispatcher;
  host?: string;
  port: number;
  maxRequestBytes: number;
  maxResponseBytes: number;
  backpressureDrainTimeoutMs?: number;
  requestTimeoutMs?: number;
}

export interface AssemblyHttpGatewayListenResult {
  host: string;
  port: number;
  server: HttpServer;
  url: string;
}

export class AssemblyHttpGateway {
  private server: HttpServer | undefined;
  private readonly streamCounters: HttpStreamLifecycleCounters = {
    activeWriters: 0,
    backpressureWaiters: 0,
    backpressureCancels: 0
  };

  constructor(private readonly options: AssemblyHttpGatewayOptions) {}

  async listen(): Promise<AssemblyHttpGatewayListenResult> {
    if (this.server !== undefined) {
      throw new Error('assembly HTTP gateway is already listening');
    }
    const host = this.options.host ?? '127.0.0.1';
    const server = createServer((request, response) => {
      this.handleRequest(request, response).catch((error: unknown) => {
        this.writeError(response, toGatewayError(error));
      });
    });
    await new Promise<void>((resolveListen) => {
      server.listen(this.options.port, host, resolveListen);
    });
    const address = server.address();
    if (address === null || typeof address === 'string') {
      throw new Error('assembly HTTP gateway did not bind to a TCP port');
    }
    this.server = server;
    return {
      host,
      port: address.port,
      server,
      url: `http://${host}:${address.port}`
    };
  }

  async close(): Promise<void> {
    await new Promise<void>((resolveClose, rejectClose) => {
      if (this.server === undefined) {
        resolveClose();
        return;
      }
      this.server.close((error) => {
        if (error !== undefined) {
          rejectClose(error);
        } else {
          resolveClose();
        }
      });
    });
    this.server = undefined;
  }

  streamLifecycleCounters(): HttpStreamLifecycleCounters {
    return { ...this.streamCounters };
  }

  private async handleRequest(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<void> {
    const snapshot = this.options.snapshots.get();
    const testCorrelationHeaders = readTestCaseCorrelationHeaders(request);
    const serviceManagesCors = hasExplicitOptionsIngress(snapshot, request);
    if (!serviceManagesCors) {
      writeAutomaticCorsHeaders(request, response);
      if (isCorsPreflightRequest(request)) {
        if (testCorrelationHeaders !== undefined) {
          throw new GatewayError(
            400,
            'InvalidTestCaseCorrelation',
            'test case capability self-ingress cannot use automatic CORS preflight'
          );
        }
        assertHttpIngressPathExists(
          snapshot,
          readHttpIngressTarget(request),
          request
        );
        writeAutomaticCorsPreflightResponse(request, response);
        return;
      }
    }
    const selection = selectHttpIngress(snapshot, request);
    const timeoutMs = effectiveHttpRequestTimeoutMs(
      this.options.requestTimeoutMs ?? DEFAULT_HTTP_REQUEST_TIMEOUT_MS
    );
    const body = await readRequestBody(request, this.options.maxRequestBytes);
    const requestId = randomUUID();
    const clientDisconnect = clientDisconnectSignal(request, response);
    const httpRequest = buildHttpRequestMetadata(request, selection.url);
    const productionHeader = assemblyHttpRequestHeader({
      snapshot,
      binding: selection.binding,
      requestId,
      timeoutMs,
      httpRequest
    });
    const header = testCorrelationHeaders === undefined
      ? productionHeader
      : assemblyTestHttpRequestHeader({
          snapshot,
          binding: selection.binding,
          requestId,
          timeoutMs,
          routing: productionHeader.routing,
          mode: productionHeader.mode,
          httpRequest,
          testCaseCapability: testCorrelationHeaders.testCaseCapability,
          testCaseParentRequestId: testCorrelationHeaders.parentRequestId
        });
    const testCorrelation = testCorrelationHeaders === undefined
      ? undefined
      : selfIngressTestCorrelation(
          snapshot,
          selection.binding,
          testCorrelationHeaders
        );
    try {
      if (header.mode === 'serverStream') {
        const writer = new HttpStreamResponseWriter({
          response,
          clientDisconnectSignal: clientDisconnect.signal,
          backpressureDrainTimeoutMs:
            this.options.backpressureDrainTimeoutMs ??
            DEFAULT_HTTP_BACKPRESSURE_DRAIN_TIMEOUT_MS,
          counters: this.streamCounters,
          maxResponseBytes: this.options.maxResponseBytes,
          writeHeaders: writeResponseHeaders
        });
        const handlers: RuntimeBinaryStreamHandlers = {
          onStart: (runtimeResponse, terminal) =>
            writer.enqueueStart(runtimeResponse, terminal),
          onChunk: (runtimeResponse, terminal) =>
            writer.enqueueChunk(runtimeResponse, terminal),
          onEnd: (runtimeResponse, terminal) => {
            writer.enqueueEnd(runtimeResponse, terminal);
            writer.markEndReceived();
          },
          closeFromPendingTerminal: (terminal) =>
            writer.closeFromPendingTerminal(terminal)
        };
        const dispatchOptions = {
          signal: clientDisconnect.signal,
          cancelReason: requestCancelReasonForSituation(
            REQUEST_CANCEL_SITUATION.clientDisconnect
          )
        };
        try {
          await (testCorrelation === undefined
            ? this.options.dispatcher.dispatchBinaryStream(
                { header, payloadBytes: body },
                timeoutMs,
                handlers,
                dispatchOptions
              )
            : this.options.dispatcher.dispatchPinnedTestBinaryStream(
                { header, payloadBytes: body },
                timeoutMs,
                handlers,
                dispatchOptions,
                testCorrelation
              ));
        } finally {
          writer.dispose();
        }
        if (!response.writableEnded) response.end();
        return;
      }
      const dispatchInput = { header, payloadBytes: body };
      const dispatchOptions = {
        signal: clientDisconnect.signal,
        cancelReason: requestCancelReasonForSituation(
          REQUEST_CANCEL_SITUATION.clientDisconnect
        )
      };
      const runtimeResponse = await (testCorrelation === undefined
        ? this.options.dispatcher.dispatchBinary(
            dispatchInput,
            timeoutMs,
            dispatchOptions
          )
        : this.options.dispatcher.dispatchPinnedTestBinary(
            dispatchInput,
            timeoutMs,
            testCorrelation,
            dispatchOptions
          ));
      if (runtimeResponse.payloadBytes.byteLength > this.options.maxResponseBytes) {
        throw new GatewayError(
          502,
          'ResponseTooLarge',
          `runtime response exceeds ${this.options.maxResponseBytes} bytes`
        );
      }
      const httpResponse = runtimeResponse.header.httpResponse;
      if (httpResponse === undefined) {
        throw new GatewayError(
          502,
          'InvalidRuntimeResponse',
          'HTTP unary response must include status and headers'
        );
      }
      response.statusCode = httpResponse.status;
      writeResponseHeaders(response, httpResponse.headers);
      response.end(Buffer.from(runtimeResponse.payloadBytes));
    } finally {
      clientDisconnect.complete();
    }
  }

  private writeError(response: ServerResponse, error: GatewayError): void {
    if (response.headersSent) {
      response.end();
      return;
    }
    response.statusCode = error.statusCode;
    response.setHeader('content-type', 'application/json; charset=utf-8');
    response.end(JSON.stringify({ error: error.toHttpPayload() }));
  }
}

export function assemblyHttpRequestHeader(input: {
  snapshot: RouterActiveAssemblySnapshot;
  binding: RuntimeAssemblyIngressBinding;
  requestId: string;
  timeoutMs: number;
  httpRequest: HttpRequestFrameMetadata;
}): RuntimeAssemblyRequestStartFrameHeader {
  assertValidHttpTimeout(input.timeoutMs, 'request timeout');
  const selector = input.binding.selector;
  if (selector.protocol !== 'http') {
    throw new Error('canonical HTTP requests require an HTTP ingress binding');
  }
  const candidate = {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: input.requestId,
    mode: input.binding.operationMode,
    caller: {
      kind: 'gateway'
    },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: input.snapshot.assembly.assemblyIdentity,
      assemblyGeneration: input.snapshot.generation,
      deployment: { ...input.binding.deployment },
      gatewayEntryIdentity: input.binding.gatewayEntryIdentity,
      ingress: {
        protocol: 'http',
        method: selector.method.toUpperCase(),
        path: selector.path
      }
    },
    deadline: {
      timeoutMs: input.timeoutMs,
      expiresAt: new Date(Date.now() + input.timeoutMs).toISOString()
    },
    trace: {
      traceId: randomUUID(),
      spanId: randomUUID()
    },
    httpRequest: input.httpRequest,
    testEffectsEnabled: false
  } as const;
  const validation = validateRuntimeAssemblyRequestStartFrameHeader(candidate);
  if (!validation.ok) {
    throw new Error(validation.error);
  }
  return validation.envelope;
}

export function assemblyTestHttpRequestHeader(input: {
  snapshot: RouterActiveAssemblySnapshot;
  binding: RuntimeAssemblyIngressBinding;
  requestId: string;
  timeoutMs: number;
  routing: RuntimeAssemblyRequestRoutingFrameHeader;
  mode: RuntimeAssemblyRequestStartFrameHeader['mode'];
  httpRequest: HttpRequestFrameMetadata;
  testCaseCapability: string;
  testCaseParentRequestId?: string;
}): RuntimeAssemblyRequestStartFrameHeader {
  const productionHeader = assemblyHttpRequestHeader({
    snapshot: input.snapshot,
    binding: input.binding,
    requestId: input.requestId,
    timeoutMs: input.timeoutMs,
    httpRequest: input.httpRequest
  });
  if (
    productionHeader.mode !== input.mode ||
    !sameAssemblyRouting(productionHeader.routing, input.routing)
  ) {
    throw new Error(
      'runtime assembly test dispatch does not match the exact active gateway binding'
    );
  }
  const candidate = {
    ...productionHeader,
    mode: input.mode,
    routing: input.routing,
    httpRequest: input.httpRequest,
    testEffectsEnabled: true,
    testCaseCapability: input.testCaseCapability,
    ...(input.testCaseParentRequestId === undefined
      ? {}
      : { testCaseParentRequestId: input.testCaseParentRequestId })
  };
  const validation = validateRuntimeAssemblyRequestStartFrameHeader(candidate);
  if (!validation.ok) {
    throw new Error(validation.error);
  }
  return validation.envelope;
}

function sameAssemblyRouting(
  left: RuntimeAssemblyRequestRoutingFrameHeader,
  right: RuntimeAssemblyRequestRoutingFrameHeader
): boolean {
  return (
    left.kind === right.kind &&
    left.assemblyIdentity === right.assemblyIdentity &&
    left.assemblyGeneration === right.assemblyGeneration &&
    sameDeployment(left.deployment, right.deployment) &&
    left.gatewayEntryIdentity === right.gatewayEntryIdentity &&
    left.ingress.protocol === right.ingress.protocol &&
    left.ingress.method === right.ingress.method &&
    left.ingress.path === right.ingress.path
  );
}

interface HttpIngressTarget {
  deployment: ReturnType<typeof readServiceDeploymentSelector>;
  url: URL;
}

function readHttpIngressTarget(request: IncomingMessage): HttpIngressTarget {
  const deployment = readServiceDeploymentSelector(request);
  const rawHost = request.headers.host;
  if (typeof rawHost !== 'string' || rawHost.length === 0 || rawHost.includes(',')) {
    throw new GatewayError(400, 'RequestHostRequired', 'request Host must be singular and present');
  }
  let host: string;
  try {
    host = canonicalHttpHost(rawHost);
  } catch (error) {
    throw new GatewayError(400, 'RequestHostInvalid', 'request Host is invalid', error);
  }
  let url: URL;
  try {
    url = readOriginFormUrlForGatewayMetadata(request.url, 'http', host);
  } catch (error) {
    throw new GatewayError(
      400,
      'RequestUrlInvalid',
      'request target must be canonical origin-form',
      error
    );
  }
  return { deployment, url };
}

function selectHttpIngress(
  snapshot: RouterActiveAssemblySnapshot,
  request: IncomingMessage
): { binding: RuntimeAssemblyIngressBinding; url: URL } {
  const target = readHttpIngressTarget(request);
  const binding = snapshot.ingress.get(target.deployment, {
    protocol: 'http',
    method: (request.method ?? 'GET').toUpperCase(),
    path: target.url.pathname
  });
  if (binding === undefined) {
    throw new GatewayError(
      404,
      'AssemblyIngressNotFound',
      `No committed RuntimeAssembly ingress matches ${target.deployment.serviceId}@${target.deployment.contractVersion} ${request.method ?? 'GET'} ${target.url.pathname}`
    );
  }
  if (
    binding.operationMode === 'serverStream' &&
    binding.adapterKind !== 'rawHttp'
  ) {
    throw new GatewayError(
      500,
      'InvalidAssemblyIngress',
      'only rawHttp bindings may use serverStream mode'
    );
  }
  return { binding, url: target.url };
}

function hasExplicitOptionsIngress(
  snapshot: RouterActiveAssemblySnapshot,
  request: IncomingMessage
): boolean {
  if (!hasCorsOriginHeader(request)) {
    return false;
  }
  try {
    const target = readHttpIngressTarget(request);
    return snapshot.ingress.get(target.deployment, {
      protocol: 'http',
      method: 'OPTIONS',
      path: target.url.pathname
    }) !== undefined;
  } catch (error) {
    if (error instanceof GatewayError) {
      return false;
    }
    throw error;
  }
}

function assertHttpIngressPathExists(
  snapshot: RouterActiveAssemblySnapshot,
  target: HttpIngressTarget,
  request: IncomingMessage
): void {
  const exists = snapshot.ingress.values().some((binding) =>
    binding.selector.protocol === 'http' &&
    binding.selector.path === target.url.pathname &&
    binding.deployment.serviceId === target.deployment.serviceId &&
    binding.deployment.contractVersion === target.deployment.contractVersion
  );
  if (!exists) {
    throw new GatewayError(
      404,
      'AssemblyIngressNotFound',
      `No committed RuntimeAssembly ingress matches ${target.deployment.serviceId}@${target.deployment.contractVersion} ${request.method ?? 'GET'} ${target.url.pathname}`
    );
  }
}

function sameDeployment(
  left: RuntimeAssemblyIngressBinding['deployment'],
  right: RuntimeAssemblyIngressBinding['deployment']
): boolean {
  return (
    left.serviceId === right.serviceId &&
    left.contractVersion === right.contractVersion &&
    left.deploymentRevision === right.deploymentRevision &&
    left.deploymentArtifactIdentity === right.deploymentArtifactIdentity
  );
}

export function effectiveHttpRequestTimeoutMs(
  platformCapMs: number
): number {
  assertValidHttpTimeout(platformCapMs, 'platform HTTP timeout cap');
  return platformCapMs;
}

function assertValidHttpTimeout(value: number, owner: string): void {
  if (
    !Number.isSafeInteger(value) ||
    value <= 0 ||
    value > MAX_NODE_TIMEOUT_MS ||
    Date.now() + value > MAX_JAVASCRIPT_DATE_MS
  ) {
    throw new GatewayError(
      500,
      'InvalidHttpTimeout',
      `${owner} must be a positive safe integer representable as a deadline and timer`
    );
  }
}

async function readRequestBody(request: IncomingMessage, limit: number): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
    size += buffer.byteLength;
    if (size > limit) {
      throw new GatewayError(413, 'RequestTooLarge', `request body exceeds ${limit} bytes`);
    }
    chunks.push(buffer);
  }
  return Buffer.concat(chunks);
}

function buildHttpRequestMetadata(
  request: IncomingMessage,
  url: URL
): HttpRequestFrameMetadata {
  return {
    method: (request.method ?? 'GET').toUpperCase(),
    url: url.toString(),
    path: url.pathname,
    query: Array.from(url.searchParams.entries()).map(([name, value]) => ({ name, value })),
    headers: request.rawHeaders.reduce<Array<{ name: string; value: string }>>(
      (headers, value, index, rawHeaders) => {
        const name = value.toLowerCase();
        if (
          index % 2 === 0 &&
          rawHeaders[index + 1] !== undefined &&
          !RESERVED_TEST_CASE_HEADERS.has(name)
        ) {
          headers.push({ name, value: rawHeaders[index + 1]! });
        }
        return headers;
      },
      []
    )
  };
}

interface TestCaseCorrelationHeaders {
  testCaseCapability: string;
  parentRequestId: string;
}

function readTestCaseCorrelationHeaders(
  request: IncomingMessage
): TestCaseCorrelationHeaders | undefined {
  const capabilityValues = rawHeaderValues(
    request,
    TEST_CASE_CAPABILITY_HEADER
  );
  const parentValues = rawHeaderValues(
    request,
    TEST_CASE_PARENT_REQUEST_ID_HEADER
  );
  if (capabilityValues.length === 0 && parentValues.length === 0) {
    return undefined;
  }
  if (
    capabilityValues.length !== 1 ||
    parentValues.length !== 1 ||
    !TEST_CASE_CORRELATION_TOKEN.test(capabilityValues[0]!) ||
    !TEST_CASE_CORRELATION_TOKEN.test(parentValues[0]!)
  ) {
    throw new GatewayError(
      400,
      'InvalidTestCaseCorrelation',
      'test case capability and parent request headers must be a singular valid pair'
    );
  }
  return {
    testCaseCapability: capabilityValues[0]!,
    parentRequestId: parentValues[0]!
  };
}

function rawHeaderValues(
  request: IncomingMessage,
  expectedName: string
): string[] {
  const values: string[] = [];
  for (let index = 0; index < request.rawHeaders.length; index += 2) {
    if (request.rawHeaders[index]?.toLowerCase() === expectedName) {
      const value = request.rawHeaders[index + 1];
      if (value !== undefined) values.push(value);
    }
  }
  return values;
}

function selfIngressTestCorrelation(
  snapshot: RouterActiveAssemblySnapshot,
  binding: RuntimeAssemblyIngressBinding,
  headers: TestCaseCorrelationHeaders
): RuntimeSelfIngressTestCorrelation {
  const contract = snapshot.resolvedContracts?.find((candidate) =>
    candidate.serviceId === binding.deployment.serviceId &&
    candidate.contractVersion === binding.deployment.contractVersion
  );
  const runtimeBinding = snapshot.deploymentRuntimeBindings?.find((candidate) =>
    sameDeployment(candidate.deployment, binding.deployment)
  );
  if (contract === undefined || runtimeBinding === undefined) {
    throw new GatewayError(
      500,
      'InvalidAssemblyIngress',
      'active HTTP ingress is missing its exact Runtime build or service protocol authority'
    );
  }
  return {
    parentRequestId: headers.parentRequestId,
    testCaseCapability: headers.testCaseCapability,
    buildId: runtimeBinding.packageBuildId,
    serviceProtocolIdentity: contract.serviceProtocolIdentity
  };
}

function writeResponseHeaders(
  response: ServerResponse,
  headers: readonly { name: string; value: string }[]
): void {
  for (const header of headers) {
    if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(header.name)) {
      throw new GatewayError(502, 'InvalidRuntimeResponse', 'runtime response header is invalid');
    }
    if (isCorsResponseHeader(header.name) && response.hasHeader(header.name)) {
      continue;
    }
    response.appendHeader(header.name, header.value);
  }
}

function clientDisconnectSignal(
  request: IncomingMessage,
  response: ServerResponse
): { signal: AbortSignal; complete(): void } {
  const controller = new AbortController();
  const abort = () => controller.abort();
  request.once('aborted', abort);
  response.once('close', abort);
  return {
    signal: controller.signal,
    complete: () => {
      request.off('aborted', abort);
      response.off('close', abort);
    }
  };
}
