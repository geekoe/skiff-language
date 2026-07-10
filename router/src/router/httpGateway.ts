import { randomUUID } from 'node:crypto';
import {
  createServer,
  type IncomingMessage,
  type Server as HttpServer,
  type ServerResponse
} from 'node:http';

import type {
  HttpRouteAdapterManifest,
  LoadedHttpRoute,
  LoadedManifest,
  OperationManifest
} from '../manifest/types.js';
import {
  type DispatchMode,
  type HttpRequestFrameMetadata,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type HttpResponseFrameMetadata,
  type RequestStartFrameHeader,
  type TelemetryEvent
} from '../protocol/envelope.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation
} from '../protocol/cancelReason.js';
import { isPublicationId } from '../publicationId.js';
import { buildActivationLookup } from '../artifacts/activationLookup.js';
import type { ActivationLookup } from '../artifacts/loadArtifactRoot.js';
import {
  RouterActiveSnapshotStore,
  type RouterActiveSnapshot
} from './activeSnapshot.js';
import { DecodeError, GatewayError, toGatewayError } from './errors.js';
import { resolveRequestRewrite, type RouterRewriteRule } from './rewrite.js';
import type {
  RuntimeBinaryDispatchChunk,
  RuntimeBinaryDispatchResponse,
  RuntimeBinaryDispatchStart,
  RuntimeDispatcher,
  PendingTerminal,
  PendingTerminalSource,
  RuntimeStreamRequestTerminal
} from './runtimeDispatcher.js';
import type { RouterTelemetryEventSink } from '../telemetry/producer.js';

const CORS_ALLOWED_METHODS = ['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS'];
export const DEFAULT_HTTP_BODY_LIMIT_BYTES = 64 * 1024 * 1024;
export const DEFAULT_HTTP_BACKPRESSURE_DRAIN_TIMEOUT_MS = 10_000;
const DEFAULT_CORS_ALLOWED_HEADERS = [
  'accept',
  'authorization',
  'content-type',
  'x-requested-with',
  'x-skiff-service',
  'x-skiff-version',
  'x-skiff-release',
  'x-skiff-trace-id',
  'x-skiff-user-admin'
];

export interface HttpGatewayOptions {
  manifest: LoadedManifest;
  dispatcher: RuntimeDispatcher;
  activationByServiceOperation?: ActivationLookup;
  snapshotStore?: RouterActiveSnapshotStore;
  host?: string;
  port: number;
  bodyLimitBytes?: number;
  backpressureDrainTimeoutMs?: number;
  requestTimeoutMs?: number;
  rewrite?: readonly RouterRewriteRule[];
  telemetry?: RouterTelemetryEventSink;
}

export interface HttpStreamLifecycleCounters {
  activeWriters: number;
  backpressureWaiters: number;
  backpressureCancels: number;
}

export interface HttpGatewayListenResult {
  host: string;
  port: number;
  server: HttpServer;
  url: string;
}

interface RawHttpOperation {
  buildId: string;
  serviceId: string;
  serviceProtocolIdentity: string;
  gatewayTarget: string;
  operation: OperationManifest;
  operationAbiId: string;
  selector: string;
}

interface HttpRouteOperation {
  buildId: string;
  serviceId: string;
  serviceProtocolIdentity: string;
  method: string;
  path: string;
  gatewayTarget: string;
  gatewayEntryIdentity?: string;
  operationName: string;
  operationAbiId: string;
  selector: string;
  dispatchTarget: string;
  mode: DispatchMode;
  timeoutMs?: number;
  httpAdapter?: RequestStartFrameHeader['httpAdapter'];
}

interface HttpHeader {
  name: string;
  value: string;
}

interface HttpQueryParam {
  name: string;
  value: string;
}

interface HttpRequestTelemetryContext {
  startTime: bigint;
  method: string;
  path: string;
  bytesIn: number;
  routeKind?: 'route' | 'raw' | 'gateway';
  requestId?: string;
  traceId?: string;
  spanId?: string;
  serviceId?: string;
  buildId?: string;
  activationIdentity?: string;
  target?: string;
  errorCode?: string;
  completed: boolean;
}

export class HttpGateway {
  private readonly backpressureDrainTimeoutMs: number;
  private readonly bodyLimitBytes: number;
  private readonly requestTimeoutMs: number;
  private readonly snapshotStore: RouterActiveSnapshotStore;
  private readonly streamCounters: HttpStreamLifecycleCounters = {
    activeWriters: 0,
    backpressureWaiters: 0,
    backpressureCancels: 0
  };
  private rawOperationCache:
    | {
        rawOperationByDispatchKey: ReadonlyMap<string, RawHttpOperation>;
        snapshot: RouterActiveSnapshot;
      }
    | undefined;
  private httpRouteCache:
    | {
        routeByDispatchKey: ReadonlyMap<string, HttpRouteOperation>;
        routes: readonly HttpRouteOperation[];
        snapshot: RouterActiveSnapshot;
      }
    | undefined;
  private server: HttpServer | undefined;

  constructor(private readonly options: HttpGatewayOptions) {
    this.backpressureDrainTimeoutMs =
      options.backpressureDrainTimeoutMs ?? DEFAULT_HTTP_BACKPRESSURE_DRAIN_TIMEOUT_MS;
    this.bodyLimitBytes = options.bodyLimitBytes ?? DEFAULT_HTTP_BODY_LIMIT_BYTES;
    this.requestTimeoutMs = options.requestTimeoutMs ?? 120_000;
    this.snapshotStore =
      options.snapshotStore ??
      new RouterActiveSnapshotStore({
        activationByServiceOperation:
          options.activationByServiceOperation ?? buildActivationLookup([]),
        manifest: options.manifest
      });
  }

  async listen(): Promise<HttpGatewayListenResult> {
    if (this.server) {
      throw new Error('HTTP gateway is already listening');
    }

    const host = this.options.host ?? '127.0.0.1';
    const server = createServer((request, response) => {
      const telemetry = this.startRequestTelemetry(request);
      this.handleRequest(request, response, telemetry).catch((error: unknown) => {
        this.writeGatewayError(response, toGatewayError(error), telemetry);
      });
    });

    await new Promise<void>((resolve) => {
      server.listen(this.options.port, host, resolve);
    });

    const address = server.address();
    if (!address || typeof address === 'string') {
      throw new Error('HTTP gateway did not bind to a TCP port');
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
    await new Promise<void>((resolve, reject) => {
      if (!this.server) {
        resolve();
        return;
      }
      this.server.close((error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve();
      });
    });
    this.server = undefined;
  }

  streamLifecycleCounters(): HttpStreamLifecycleCounters {
    return { ...this.streamCounters };
  }

  private async handleRequest(
    request: IncomingMessage,
    response: ServerResponse,
    telemetry: HttpRequestTelemetryContext
  ): Promise<void> {
    this.attachRequestTelemetryFinalizers(response, telemetry);
    this.writeCorsHeaders(request, response);
    if (isCorsPreflightRequest(request)) {
      telemetry.routeKind = 'gateway';
      this.writeCorsPreflightResponse(request, response);
      return;
    }

    const url = requestUrl(request);

    if (url.pathname === '/__router/health' || url.pathname === '/__router/prune-runtimes') {
      this.writeGatewayError(
        response,
        new GatewayError(
          404,
          'ControlEndpointNotFound',
          'router control endpoints are served by the runtime/control listener'
        ),
        telemetry
      );
      return;
    }

    if (url.pathname === '/__skiff/reload-artifacts') {
      this.writeGatewayError(
        response,
        new GatewayError(
          404,
          'ControlEndpointNotFound',
          'router control endpoints are served by the runtime/control listener'
        ),
        telemetry
      );
      return;
    }

    if (url.pathname === '/favicon.ico') {
      telemetry.routeKind = 'gateway';
      response.statusCode = 204;
      response.end();
      return;
    }

    const rewrite = resolveRequestRewrite(this.options.rewrite, request, url);
    const dispatchServiceId = rewrite?.service ?? requestServiceId(request, url);
    const snapshot = this.currentSnapshot();
    const serviceVersion = this.resolveServiceVersion(
      snapshot,
      dispatchServiceId,
      snapshot.versionByService !== undefined
        ? rewrite?.version ?? requestVersion(request, url)
        : undefined
    );
    const routeDispatch = this.resolveHttpRouteDispatch(
      snapshot,
      dispatchServiceId,
      serviceVersion?.buildId,
      normalizeRequestMethod(request.method),
      url.pathname
    );
    if (routeDispatch.routeOperation) {
      await this.handleRouteRequest(
        snapshot,
        routeDispatch.routeOperation,
        request,
        response,
        url,
        telemetry
      );
      return;
    }
    if (routeDispatch.serviceHasRoutes) {
      this.writeGatewayError(
        response,
        new GatewayError(
          404,
          'HttpRouteNotFound',
          `No HTTP route is loaded for ${normalizeRequestMethod(request.method)} ${url.pathname}`
        ),
        telemetry
      );
      return;
    }

    const rawDispatch = this.resolveRawDispatch(snapshot, dispatchServiceId, serviceVersion?.buildId);
    if (!rawDispatch) {
      this.writeGatewayError(
        response,
        new GatewayError(
          404,
          'HttpServiceNotFound',
          dispatchServiceId
            ? `No raw HTTP service is loaded for ${dispatchServiceId}`
            : 'No service selector is available for raw dispatch'
        ),
        telemetry
      );
      return;
    }

    await this.handleRawRequest(
      snapshot,
      rawDispatch.rawOperation,
      request,
      response,
      url,
      telemetry
    );
  }

  private async handleRouteRequest(
    snapshot: RouterActiveSnapshot,
    routeOperation: HttpRouteOperation,
    request: IncomingMessage,
    response: ServerResponse,
    url: URL,
    telemetry: HttpRequestTelemetryContext
  ): Promise<void> {
    const rawBody = await this.readBodyBuffer(request);
    const timeoutMs = this.resolveTimeoutMs(
      snapshot.manifest,
      routeOperation.operationName,
      routeOperation.dispatchTarget,
      routeOperation.timeoutMs
    );
    const requestId = randomUUID();
    const traceId = request.headers['x-skiff-trace-id'];
    const activationIdentity = this.resolveActivationIdentity(
      snapshot,
      routeOperation.serviceId,
      routeOperation.dispatchTarget,
      routeOperation.buildId
    );
    const header: RequestStartFrameHeader = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.start',
      requestId,
      mode: routeOperation.mode,
      caller: {
        kind: 'gateway',
        target: routeOperation.gatewayTarget
      },
      target: routeOperation.dispatchTarget,
      operationAbiId: routeOperation.operationAbiId,
      selector: routeOperation.selector,
      serviceId: routeOperation.serviceId,
      buildId: routeOperation.buildId,
      serviceProtocolIdentity: routeOperation.serviceProtocolIdentity,
      ...(routeOperation.gatewayEntryIdentity !== undefined
        ? {
            gatewayEntryIdentity: routeOperation.gatewayEntryIdentity
          }
        : {}),
      ...(routeOperation.httpAdapter !== undefined
        ? {
            httpAdapter: routeOperation.httpAdapter
          }
        : {}),
      ...(activationIdentity !== undefined
        ? {
            activationIdentity
          }
        : {}),
      deadline: {
        timeoutMs,
        expiresAt: new Date(Date.now() + timeoutMs).toISOString()
      },
      trace: {
        traceId: typeof traceId === 'string' && traceId.length > 0 ? traceId : randomUUID(),
        spanId: randomUUID()
      },
      httpRequest: buildHttpRequestMetadata(request, url)
    };
    this.markDispatchTelemetry(telemetry, header, 'route', rawBody.byteLength);

    await this.dispatchHttpRequest(
      {
        header,
        payloadBytes: rawBody
      },
      timeoutMs,
      request,
      response
    );
  }

  private async handleRawRequest(
    snapshot: RouterActiveSnapshot,
    rawOperation: RawHttpOperation,
    request: IncomingMessage,
    response: ServerResponse,
    url: URL,
    telemetry: HttpRequestTelemetryContext
  ): Promise<void> {
    const operation = rawOperation.operation;
    const rawBody = await this.readBodyBuffer(request);
    const timeoutMs = this.resolveTimeoutMs(
      snapshot.manifest,
      operation.operation,
      operation.target,
      operation.timeoutMs
    );
    const requestId = randomUUID();
    const traceId = request.headers['x-skiff-trace-id'];
    const activationIdentity = this.resolveActivationIdentity(
      snapshot,
      rawOperation.serviceId,
      operation.target,
      rawOperation.buildId
    );
    const header: RequestStartFrameHeader = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.start',
      requestId,
      mode: operation.mode,
      caller: {
        kind: 'gateway',
        target: rawOperation.gatewayTarget
      },
      target: operation.target,
      operationAbiId: rawOperation.operationAbiId,
      selector: rawOperation.selector,
      serviceId: rawOperation.serviceId,
      buildId: rawOperation.buildId,
      serviceProtocolIdentity: rawOperation.serviceProtocolIdentity,
      ...(activationIdentity !== undefined
        ? {
            activationIdentity
          }
        : {}),
      deadline: {
        timeoutMs,
        expiresAt: new Date(Date.now() + timeoutMs).toISOString()
      },
      trace: {
        traceId: typeof traceId === 'string' && traceId.length > 0 ? traceId : randomUUID(),
        spanId: randomUUID()
      },
      httpRequest: buildHttpRequestMetadata(request, url)
    };
    this.markDispatchTelemetry(telemetry, header, 'raw', rawBody.byteLength);

    await this.dispatchHttpRequest(
      {
        header,
        payloadBytes: rawBody
      },
      timeoutMs,
      request,
      response
    );
  }

  private async dispatchHttpRequest(
    dispatch: {
      header: RequestStartFrameHeader;
      payloadBytes: Uint8Array;
    },
    timeoutMs: number,
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<void> {
    const clientDisconnect = this.clientDisconnectSignal(request, response);
    try {
      if (dispatch.header.mode === 'serverStream') {
        const streamWriter = new HttpStreamWriteOwner({
          response,
          clientDisconnectSignal: clientDisconnect.signal,
          backpressureDrainTimeoutMs: this.backpressureDrainTimeoutMs,
          counters: this.streamCounters
        });
        try {
          await this.options.dispatcher.dispatchBinaryStream(
            dispatch,
            timeoutMs,
            {
              onStart: (runtimeResponse, requestTerminal) => {
                streamWriter.enqueueStart(runtimeResponse, requestTerminal);
              },
              onChunk: (runtimeResponse, requestTerminal) => {
                streamWriter.enqueueChunk(runtimeResponse, requestTerminal);
              },
              onEnd: (runtimeResponse, requestTerminal) => {
                streamWriter.enqueueEnd(runtimeResponse, requestTerminal);
                streamWriter.markEndReceived();
              },
              closeFromPendingTerminal: (terminal) =>
                streamWriter.closeFromPendingTerminal(terminal)
            },
            {
              signal: clientDisconnect.signal,
              cancelReason: requestCancelReasonForSituation(
                REQUEST_CANCEL_SITUATION.clientDisconnect
              )
            }
          );
        } finally {
          streamWriter.dispose();
        }
        if (!response.writableEnded) {
          response.end();
        }
        return;
      }

      const runtimeResponse = await this.options.dispatcher.dispatchBinary(
        dispatch,
        timeoutMs,
        {
          signal: clientDisconnect.signal,
          cancelReason: requestCancelReasonForSituation(
            REQUEST_CANCEL_SITUATION.clientDisconnect
          )
        }
      );
      this.writeHttpFrameResponse(response, runtimeResponse);
    } finally {
      clientDisconnect.complete();
    }
  }

  private resolveActivationIdentity(
    snapshot: RouterActiveSnapshot,
    serviceId: string,
    target: string,
    buildId: string
  ): string | undefined {
    return snapshot.activationByServiceOperation.get({
      serviceId,
      target,
      buildId
    });
  }

  private async readBodyBuffer(request: IncomingMessage): Promise<Buffer> {
    const chunks: Buffer[] = [];
    let size = 0;

    for await (const chunk of request) {
      const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
      size += buffer.byteLength;
      if (size > this.bodyLimitBytes) {
        throw new DecodeError('request body is too large', {
          limitBytes: this.bodyLimitBytes
        });
      }
      chunks.push(buffer);
    }

    return Buffer.concat(chunks);
  }

  private resolveTimeoutMs(
    manifest: LoadedManifest,
    operationName: string,
    operationTarget: string,
    operationTimeoutMs: number | undefined
  ): number {
    return (
      operationTimeoutMs ??
      manifest.timeout?.methods?.[operationName] ??
      manifest.timeout?.methods?.[operationTarget] ??
      manifest.timeout?.defaultMs ??
      this.requestTimeoutMs
    );
  }

  private resolveRawDispatch(
    snapshot: RouterActiveSnapshot,
    serviceId: string | undefined,
    buildId: string | undefined
  ): {
    rawOperation: RawHttpOperation;
  } | null {
    if (!serviceId) {
      return null;
    }

    const rawOperation =
      buildId === undefined
        ? this.resolveUniqueRawDispatchForService(snapshot, serviceId)
        : this.rawOperationsForSnapshot(snapshot).get(rawDispatchKey(serviceId, buildId));
    if (!rawOperation) {
      return null;
    }
    return {
      rawOperation
    };
  }

  private resolveHttpRouteDispatch(
    snapshot: RouterActiveSnapshot,
    serviceId: string | undefined,
    buildId: string | undefined,
    method: string,
    path: string
  ): {
    routeOperation?: HttpRouteOperation;
    serviceHasRoutes: boolean;
  } {
    if (!serviceId) {
      return { serviceHasRoutes: false };
    }

    if (buildId !== undefined) {
      const routes = this.httpRoutesForSnapshot(snapshot);
      const routeOperation = routes.routeByDispatchKey.get(
        httpRouteDispatchKey(serviceId, buildId, method, path)
      );
      return {
        ...(routeOperation !== undefined ? { routeOperation } : {}),
        serviceHasRoutes: routes.routes.some(
          (route) => route.serviceId === serviceId && route.buildId === buildId
        )
      };
    }

    const routes = this.httpRoutesForSnapshot(snapshot).routes.filter(
      (route) => route.serviceId === serviceId
    );
    const matches = routes.filter((route) => route.method === method && route.path === path);
    if (matches.length > 1) {
      throw new GatewayError(
        400,
        'VersionRequired',
        'version selector is required when multiple builds are loaded for a service'
      );
    }
    return {
      ...(matches[0] !== undefined ? { routeOperation: matches[0] } : {}),
      serviceHasRoutes: routes.length > 0
    };
  }

  private resolveServiceVersion(
    snapshot: RouterActiveSnapshot,
    serviceId: string | undefined,
    version: string | undefined
  ): { buildId: string } | undefined {
    if (snapshot.versionByService === undefined) {
      return undefined;
    }
    if (!serviceId) {
      throw new GatewayError(
        400,
        'HttpServiceRequired',
        'service selector is required for version dispatch'
      );
    }
    if (!version) {
      throw new GatewayError(
        400,
        'VersionRequired',
        'version selector is required for version dispatch'
      );
    }
    const serviceVersion = snapshot.versionByService.get(serviceId)?.get(version);
    if (!serviceVersion) {
      throw new GatewayError(
        404,
        'VersionNotFound',
        `No version ${version} is loaded for service ${serviceId}`
      );
    }
    return { buildId: serviceVersion.buildId };
  }

  private currentSnapshot(): RouterActiveSnapshot {
    return this.snapshotStore.get();
  }

  private httpRoutesForSnapshot(snapshot: RouterActiveSnapshot): {
    routeByDispatchKey: ReadonlyMap<string, HttpRouteOperation>;
    routes: readonly HttpRouteOperation[];
  } {
    if (this.httpRouteCache?.snapshot === snapshot) {
      return this.httpRouteCache;
    }
    const compiled = this.compileHttpRoutes(snapshot.manifest);
    this.httpRouteCache = {
      ...compiled,
      snapshot
    };
    return compiled;
  }

  private compileHttpRoutes(manifest: LoadedManifest): {
    routeByDispatchKey: ReadonlyMap<string, HttpRouteOperation>;
    routes: readonly HttpRouteOperation[];
  } {
    const routeByDispatchKey = new Map<string, HttpRouteOperation>();
    const routes: HttpRouteOperation[] = [];
    for (const entry of manifest.httpRouteEntries) {
      const route = this.toHttpRouteOperation(entry);
      const key = httpRouteDispatchKey(
        route.serviceId,
        route.buildId,
        route.method,
        route.path
      );
      if (routeByDispatchKey.has(key)) {
        throw new Error(
          `HTTP route conflict for service ${route.serviceId} build ${route.buildId} ${route.method} ${route.path}`
        );
      }
      routeByDispatchKey.set(key, route);
      routes.push(route);
    }
    return { routeByDispatchKey, routes };
  }

  private toHttpRouteOperation(entry: LoadedHttpRoute): HttpRouteOperation {
    if (entry.buildId === undefined) {
      throw new Error(
        `HTTP route ${entry.method} ${entry.path} for service ${entry.serviceId} is missing buildId`
      );
    }
    const httpAdapter = toHttpAdapterFrameMetadata(entry.adapter ?? entry.typed?.adapter);
    const operationManifest = entry.operationManifest;
    if (operationManifest === undefined && entry.handler?.kind !== 'packageFunction') {
      throw new Error(
        `HTTP route ${entry.method} ${entry.path} for service ${entry.serviceId} is missing operationManifest`
      );
    }
    const operationAbiId = operationManifest?.operationAbiId ?? entry.operationAbiId;
    return {
      buildId: entry.buildId,
      serviceId: entry.serviceId,
      serviceProtocolIdentity: entry.serviceProtocolIdentity,
      method: entry.method,
      path: entry.path,
      gatewayTarget: entry.gatewayTarget,
      ...(entry.gatewayEntryIdentity !== undefined
        ? { gatewayEntryIdentity: entry.gatewayEntryIdentity }
        : {}),
      operationName: entry.operation ?? entry.dispatchTarget,
      operationAbiId,
      selector: entry.selector,
      dispatchTarget: entry.dispatchTarget,
      mode: operationManifest?.mode ?? 'unary',
      ...(operationManifest?.timeoutMs !== undefined
        ? { timeoutMs: operationManifest.timeoutMs }
        : {}),
      ...(httpAdapter !== undefined ? { httpAdapter } : {})
    };
  }

  private rawOperationsForSnapshot(
    snapshot: RouterActiveSnapshot
  ): ReadonlyMap<string, RawHttpOperation> {
    if (this.rawOperationCache?.snapshot === snapshot) {
      return this.rawOperationCache.rawOperationByDispatchKey;
    }
    const rawOperationByDispatchKey = this.compileRawOperations(snapshot.manifest);
    this.rawOperationCache = {
      rawOperationByDispatchKey,
      snapshot
    };
    return rawOperationByDispatchKey;
  }

  private compileRawOperations(manifest: LoadedManifest): ReadonlyMap<string, RawHttpOperation> {
    const operations = new Map<string, RawHttpOperation>();
    for (const entry of manifest.rawHttpEntries) {
      const key = rawDispatchKey(entry.serviceId, entry.buildId);
      if (operations.has(key)) {
        throw new Error(
          entry.buildId
            ? `raw HTTP operation conflict for service ${entry.serviceId} build ${entry.buildId}`
            : `raw HTTP operation conflict for service ${entry.serviceId}`
        );
      }
      operations.set(key, this.toRawHttpOperation(entry));
    }
    return operations;
  }

  private toRawHttpOperation(
    entry: NonNullable<LoadedManifest['rawHttpEntries']>[number]
  ): RawHttpOperation {
    if (entry.buildId === undefined) {
      throw new Error(
        `raw HTTP operation ${entry.operation} for service ${entry.serviceId} is missing buildId`
      );
    }
    const operation = entry.operationManifest;
    const serviceProtocolIdentity = entry.serviceProtocolIdentity;
    if (!serviceProtocolIdentity) {
      throw new Error(
        `raw HTTP operation ${operation.operation} for service ${entry.serviceId} is missing serviceProtocolIdentity`
      );
    }
    const [parameter] = operation.parameters;
    if (!parameter) {
      throw new Error(`raw HTTP operation ${operation.operation} must declare one parameter`);
    }
    return {
      buildId: entry.buildId,
      serviceId: entry.serviceId,
      serviceProtocolIdentity,
      gatewayTarget: entry.target,
      operation,
      operationAbiId: operation.operationAbiId,
      selector: `operation:${operation.operationAbiId}`,
    };
  }

  private resolveUniqueRawDispatchForService(
    snapshot: RouterActiveSnapshot,
    serviceId: string
  ): RawHttpOperation | undefined {
    const matches = Array.from(this.rawOperationsForSnapshot(snapshot).values()).filter(
      (operation) => operation.serviceId === serviceId
    );
    if (matches.length <= 1) {
      return matches[0];
    }
    throw new GatewayError(
      400,
      'VersionRequired',
      'version selector is required when multiple builds are loaded for a service'
    );
  }

  private writeGatewayError(
    response: ServerResponse,
    error: GatewayError,
    telemetry?: HttpRequestTelemetryContext
  ): void {
    if (telemetry !== undefined) {
      telemetry.routeKind ??= 'gateway';
      telemetry.errorCode = error.code;
    }
    if (response.headersSent) {
      if (!response.destroyed) {
        response.destroy(error);
      }
      return;
    }
    this.writeJson(response, error.statusCode, error.toHttpBody());
  }

  private writeCorsHeaders(request: IncomingMessage, response: ServerResponse): void {
    const origin = firstHeader(request.headers.origin)?.trim();
    if (!origin) {
      return;
    }

    response.setHeader('access-control-allow-origin', origin);
    response.setHeader('access-control-allow-credentials', 'true');
    addVaryHeader(response, 'Origin');
  }

  private writeCorsPreflightResponse(
    request: IncomingMessage,
    response: ServerResponse
  ): void {
    if (response.headersSent) {
      response.end();
      return;
    }

    response.statusCode = 204;
    response.setHeader('access-control-allow-methods', CORS_ALLOWED_METHODS.join(', '));
    response.setHeader('access-control-allow-headers', corsAllowedHeaders(request));
    response.setHeader('access-control-max-age', '600');
    addVaryHeader(response, 'Access-Control-Request-Method');
    addVaryHeader(response, 'Access-Control-Request-Headers');
    response.end();
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

  private startRequestTelemetry(request: IncomingMessage): HttpRequestTelemetryContext {
    return {
      startTime: process.hrtime.bigint(),
      method: normalizeRequestMethod(request.method),
      path: decodePathname(requestUrlPathname(request.url)),
      bytesIn: contentLengthBytes(request.headers['content-length']),
      completed: false
    };
  }

  private attachRequestTelemetryFinalizers(
    response: ServerResponse,
    telemetry: HttpRequestTelemetryContext
  ): void {
    const finalize = () => {
      this.finalizeRequestTelemetry(response, telemetry);
    };
    response.once('finish', finalize);
    response.once('close', finalize);
  }

  private markDispatchTelemetry(
    telemetry: HttpRequestTelemetryContext,
    header: RequestStartFrameHeader,
    routeKind: 'route' | 'raw',
    bytesIn: number
  ): void {
    telemetry.routeKind = routeKind;
    telemetry.bytesIn = bytesIn;
    telemetry.requestId = header.requestId;
    telemetry.traceId = header.trace.traceId;
    telemetry.spanId = header.trace.spanId;
    if (header.serviceId !== undefined) {
      telemetry.serviceId = header.serviceId;
    }
    if (header.buildId !== undefined) {
      telemetry.buildId = header.buildId;
    }
    if (header.activationIdentity !== undefined) {
      telemetry.activationIdentity = header.activationIdentity;
    }
    telemetry.target = header.target;
  }

  private finalizeRequestTelemetry(
    response: ServerResponse,
    telemetry: HttpRequestTelemetryContext
  ): void {
    if (telemetry.completed) {
      return;
    }
    telemetry.completed = true;
    const durationMs = Number(process.hrtime.bigint() - telemetry.startTime) / 1_000_000;
    const statusCode = telemetryStatusCode(response);
    const clientDisconnected = !response.writableEnded;
    const event: TelemetryEvent = {
      topic: 'trace',
      ts: new Date().toISOString(),
      source: 'router',
      name: 'http.request',
      durationMs,
      ...(telemetry.serviceId !== undefined ? { serviceId: telemetry.serviceId } : {}),
      ...(telemetry.buildId !== undefined ? { buildId: telemetry.buildId } : {}),
      ...(telemetry.activationIdentity !== undefined
        ? { activationIdentity: telemetry.activationIdentity }
        : {}),
      ...(telemetry.requestId !== undefined ? { requestId: telemetry.requestId } : {}),
      ...(telemetry.traceId !== undefined ? { traceId: telemetry.traceId } : {}),
      ...(telemetry.spanId !== undefined ? { spanId: telemetry.spanId } : {}),
      ...(telemetry.target !== undefined ? { target: telemetry.target } : {}),
      attrs: {
        method: telemetry.method,
        path: telemetry.path,
        status: statusCode,
        routeKind: telemetry.routeKind ?? 'gateway',
        bytesIn: telemetry.bytesIn,
        ended: response.writableEnded
      },
      ...(telemetry.errorCode !== undefined
        ? { error: { code: telemetry.errorCode } }
        : clientDisconnected
          ? { error: { code: 'ClientDisconnected' } }
          : statusCode >= 500
            ? { error: { code: 'HttpStatusError' } }
            : {})
    };
    this.options.telemetry?.emit(event);
  }

  private writeHttpFrameResponse(
    response: ServerResponse,
    runtimeResponse: RuntimeBinaryDispatchResponse
  ): void {
    if (response.headersSent) {
      response.end();
      return;
    }
    const httpResponse = runtimeResponse.header.httpResponse;
    if (httpResponse === undefined) {
      throw new GatewayError(
        502,
        'InvalidHttpResponse',
        'response.end frame must include httpResponse metadata for HTTP dispatch'
      );
    }
    response.statusCode = httpResponse.status;
    writeResponseHeaders(response, httpResponse.headers);
    response.end(
      Buffer.from(
        runtimeResponse.payloadBytes.buffer,
        runtimeResponse.payloadBytes.byteOffset,
        runtimeResponse.payloadBytes.byteLength
      )
    );
  }

  private clientDisconnectSignal(
    request: IncomingMessage,
    response: ServerResponse
  ): {
    signal: AbortSignal;
    complete(): void;
  } {
    const controller = new AbortController();
    let completed = false;
    const abort = () => {
      if (!completed && !controller.signal.aborted) {
        controller.abort();
      }
    };
    request.once('aborted', abort);
    response.once('close', abort);
    return {
      signal: controller.signal,
      complete: () => {
        completed = true;
        request.off('aborted', abort);
        response.off('close', abort);
      }
    };
  }
}

function buildHttpRequestMetadata(
  input: IncomingMessage,
  url: URL
): HttpRequestFrameMetadata {
  return {
    method: (input.method ?? 'GET').toUpperCase(),
    url: url.toString(),
    path: decodePathname(url.pathname),
    query: readQuery(url),
    headers: readHeaders(input)
  };
}

function toHttpAdapterFrameMetadata(
  adapter: HttpRouteAdapterManifest | undefined
): RequestStartFrameHeader['httpAdapter'] | undefined {
  if (adapter === undefined) {
    return undefined;
  }
  return {
    kind: adapter.kind,
    handler: adapter.handler,
    ...(adapter.guard !== undefined ? { guard: adapter.guard } : {}),
    ...(adapter.pre !== undefined ? { pre: adapter.pre } : {}),
    ...(adapter.adapterArgs !== undefined
      ? {
          adapterArgs: adapter.adapterArgs.map((arg) => ({
            param: arg.param,
            source: {
              kind: toHttpAdapterSourceKind(arg.source.kind)
            }
          }))
        }
      : {})
  };
}

function toHttpAdapterSourceKind(kind: string): 'http.request' | 'http.body' | 'http.context' {
  switch (kind) {
    case 'http.request':
    case 'http.body':
    case 'http.context':
      return kind;
    default:
      throw new Error(`unsupported HTTP adapter source ${kind}`);
  }
}

class HttpStreamWriteOwner {
  private closed = false;
  private endReceived = false;
  private queue: Promise<void> = Promise.resolve();
  private requestTerminalCallback: RuntimeStreamRequestTerminal | undefined;
  private terminalRequested = false;

  constructor(
    private readonly input: {
      response: ServerResponse;
      clientDisconnectSignal: AbortSignal;
      backpressureDrainTimeoutMs: number;
      counters: HttpStreamLifecycleCounters;
    }
  ) {
    this.input.counters.activeWriters += 1;
  }

  enqueueStart(
    runtimeResponse: RuntimeBinaryDispatchStart,
    requestTerminal: RuntimeStreamRequestTerminal
  ): void {
    this.bindRequestTerminal(requestTerminal);
    this.enqueue('callback_error', () => {
      if (this.input.response.headersSent) {
        throw new GatewayError(
          502,
          'InvalidHttpResponse',
          'response.start received after HTTP response headers were sent'
        );
      }
      const httpResponse = runtimeResponse.header.httpResponse;
      this.input.response.statusCode = httpResponse.status;
      writeResponseHeaders(this.input.response, httpResponse.headers);
      this.input.response.flushHeaders();
    });
  }

  enqueueChunk(
    runtimeResponse: RuntimeBinaryDispatchChunk,
    requestTerminal: RuntimeStreamRequestTerminal
  ): void {
    this.bindRequestTerminal(requestTerminal);
    this.enqueue('callback_error', async () => {
      if (!this.input.response.headersSent) {
        throw new GatewayError(
          502,
          'InvalidHttpResponse',
          'response.chunk received before response.start'
        );
      }
      await this.writeBuffer(
        Buffer.from(
          runtimeResponse.payloadBytes.buffer,
          runtimeResponse.payloadBytes.byteOffset,
          runtimeResponse.payloadBytes.byteLength
        )
      );
    });
  }

  enqueueEnd(
    runtimeResponse: RuntimeBinaryDispatchResponse,
    requestTerminal: RuntimeStreamRequestTerminal
  ): void {
    this.bindRequestTerminal(requestTerminal);
    this.enqueue('callback_error', async () => {
      if (!this.input.response.headersSent) {
        throw new GatewayError(
          502,
          'InvalidHttpResponse',
          'response.end received before response.start'
        );
      }
      if (runtimeResponse.payloadBytes.byteLength !== 0) {
        throw new GatewayError(
          502,
          'InvalidHttpResponse',
          'streaming response.end must not include a payload'
        );
      }
      await this.endResponse();
      this.requestTerminal('runtime_response_end');
    });
  }

  markEndReceived(): void {
    this.endReceived = true;
  }

  requestTerminal(source: PendingTerminalSource, error?: unknown): void {
    if (this.terminalRequested) {
      return;
    }
    if (source === 'runtime_response_end' && !this.endReceived) {
      return;
    }
    this.terminalRequested = true;
    if (source === 'backpressure') {
      this.input.counters.backpressureCancels += 1;
    }
    this.requestTerminalCallback?.(httpStreamPendingTerminal(source, error));
  }

  closeFromPendingTerminal(_terminal: PendingTerminal): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.input.counters.activeWriters = Math.max(0, this.input.counters.activeWriters - 1);
  }

  dispose(): void {
    this.closeFromPendingTerminal({ source: 'router_shutdown', kind: 'cancelled' });
  }

  private bindRequestTerminal(requestTerminal: RuntimeStreamRequestTerminal): void {
    this.requestTerminalCallback ??= requestTerminal;
  }

  private enqueue(source: PendingTerminalSource, write: () => void | Promise<void>): void {
    this.queue = this.queue.then(async () => {
      if (this.closed || this.terminalRequested) {
        return;
      }
      try {
        await write();
      } catch (error) {
        this.requestTerminal(source, error);
      }
    });
    void this.queue.catch((error: unknown) => {
      this.requestTerminal(source, error);
    });
  }

  private async writeBuffer(buffer: Buffer): Promise<void> {
    if (this.closed || this.terminalRequested) {
      return;
    }
    if (this.input.response.destroyed || this.input.clientDisconnectSignal.aborted) {
      this.requestTerminal('client_disconnect');
      return;
    }
    const accepted = this.input.response.write(buffer);
    if (!accepted) {
      await this.waitForDrain();
    }
  }

  private async waitForDrain(): Promise<void> {
    if (this.input.clientDisconnectSignal.aborted || this.input.response.destroyed) {
      this.requestTerminal('client_disconnect');
      return;
    }
    this.input.counters.backpressureWaiters += 1;
    try {
      await new Promise<void>((resolve, reject) => {
        let timeout: NodeJS.Timeout | undefined;
        const cleanup = () => {
          if (timeout) {
            clearTimeout(timeout);
          }
          this.input.response.off('drain', onDrain);
          this.input.response.off('error', onError);
          this.input.clientDisconnectSignal.removeEventListener('abort', onAbort);
        };
        const finish = (callback: () => void) => {
          cleanup();
          callback();
        };
        const onDrain = () => {
          finish(resolve);
        };
        const onError = (error: Error) => {
          finish(() => {
            this.requestTerminal('callback_error', error);
            reject(error);
          });
        };
        const onAbort = () => {
          finish(() => {
            this.requestTerminal('client_disconnect');
            reject(new Error('HTTP client disconnected while waiting for drain'));
          });
        };
        timeout = setTimeout(() => {
          finish(() => {
            this.requestTerminal('backpressure');
            reject(new Error('HTTP response drain timed out'));
          });
        }, this.input.backpressureDrainTimeoutMs);
        this.input.response.once('drain', onDrain);
        this.input.response.once('error', onError);
        this.input.clientDisconnectSignal.addEventListener('abort', onAbort, { once: true });
      });
    } finally {
      this.input.counters.backpressureWaiters = Math.max(
        0,
        this.input.counters.backpressureWaiters - 1
      );
    }
  }

  private async endResponse(): Promise<void> {
    if (this.closed || this.terminalRequested || this.input.response.writableEnded) {
      return;
    }
    if (this.input.clientDisconnectSignal.aborted || this.input.response.destroyed) {
      this.requestTerminal('client_disconnect');
      return;
    }
    await new Promise<void>((resolve, reject) => {
      const cleanup = () => {
        this.input.response.off('error', onError);
        this.input.clientDisconnectSignal.removeEventListener('abort', onAbort);
      };
      const onError = (error: Error) => {
        cleanup();
        this.requestTerminal('callback_error', error);
        reject(error);
      };
      const onAbort = () => {
        cleanup();
        this.requestTerminal('client_disconnect');
        reject(new Error('HTTP client disconnected while ending stream'));
      };
      this.input.response.once('error', onError);
      this.input.clientDisconnectSignal.addEventListener('abort', onAbort, { once: true });
      this.input.response.end(() => {
        cleanup();
        resolve();
      });
    });
  }
}

function httpStreamPendingTerminal(
  source: PendingTerminalSource,
  error: unknown
): PendingTerminal {
  switch (source) {
    case 'runtime_response_end':
      return { source, kind: 'completed' };
    case 'client_disconnect':
      return {
        source,
        kind: 'cancelled',
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.clientDisconnect)
      };
    case 'backpressure':
      return {
        source,
        kind: 'cancelled',
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.backpressure)
      };
    case 'timeout':
      return {
        source,
        kind: 'cancelled',
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
      };
    case 'caller_abort':
      return {
        source,
        kind: 'cancelled',
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.callerAbort)
      };
    case 'runtime_disconnect':
      return {
        source,
        kind: 'cancelled',
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.runtimeDisconnect)
      };
    case 'router_shutdown':
      return {
        source,
        kind: 'cancelled',
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.routerShutdown)
      };
    case 'runtime_response_error':
    case 'runtime_request_cancel':
      return { source, kind: 'failed', error: error ?? new Error(`HTTP stream ${source}`) };
    case 'protocol_error':
    case 'callback_error':
      return { source, kind: 'failed', error: error ?? new Error(`HTTP stream ${source}`) };
  }
}

function writeResponseHeaders(
  response: ServerResponse,
  value: HttpResponseFrameMetadata['headers']
): void {
  if (!Array.isArray(value)) {
    throw new GatewayError(502, 'InvalidHttpResponse', 'HttpResponse.headers must be an array');
  }
  for (const item of value) {
    if (!isRecord(item)) {
      throw new GatewayError(502, 'InvalidHttpResponse', 'HttpResponse header must be an object');
    }
    const name = readRequiredString(item.name, 'HttpResponse.header.name').toLowerCase();
    const headerValue = readRequiredString(item.value, `HttpResponse.header.${name}.value`);
    if (!isValidHeaderName(name)) {
      throw new GatewayError(502, 'InvalidHttpResponse', `invalid response header ${name}`);
    }
    if (isCorsResponseHeader(name) && response.hasHeader(name)) {
      continue;
    }
    appendResponseHeader(response, name, validateHeaderValue(name, headerValue));
  }
}

function isCorsPreflightRequest(request: IncomingMessage): boolean {
  return (
    normalizeRequestMethod(request.method) === 'OPTIONS' &&
    firstHeader(request.headers.origin) !== undefined &&
    firstHeader(request.headers['access-control-request-method']) !== undefined
  );
}

function corsAllowedHeaders(request: IncomingMessage): string {
  const requestedHeaders = firstHeader(request.headers['access-control-request-headers']);
  if (requestedHeaders === undefined || requestedHeaders.trim() === '') {
    return DEFAULT_CORS_ALLOWED_HEADERS.join(', ');
  }

  const headers: string[] = [];
  const seen = new Set<string>();
  for (const value of requestedHeaders.split(',')) {
    const header = value.trim().toLowerCase();
    if (!header || seen.has(header) || !isValidHeaderName(header)) {
      continue;
    }
    seen.add(header);
    headers.push(header);
  }
  return headers.length > 0 ? headers.join(', ') : DEFAULT_CORS_ALLOWED_HEADERS.join(', ');
}

function isCorsResponseHeader(name: string): boolean {
  return name.startsWith('access-control-');
}

function addVaryHeader(response: ServerResponse, value: string): void {
  const existing = response.getHeader('vary');
  const values = new Map<string, string>();
  const add = (item: string) => {
    for (const part of item.split(',')) {
      const name = part.trim();
      if (!name) {
        continue;
      }
      values.set(name.toLowerCase(), name);
    }
  };

  if (Array.isArray(existing)) {
    for (const item of existing) {
      add(String(item));
    }
  } else if (existing !== undefined) {
    add(String(existing));
  }
  add(value);
  response.setHeader('vary', Array.from(values.values()).join(', '));
}

function appendResponseHeader(response: ServerResponse, name: string, value: string): void {
  const existing = response.getHeader(name);
  if (existing === undefined) {
    response.setHeader(name, value);
    return;
  }
  if (Array.isArray(existing)) {
    response.setHeader(name, [...existing.map(String), value]);
    return;
  }
  response.setHeader(name, [String(existing), value]);
}

function readQuery(url: URL): HttpQueryParam[] {
  return Array.from(url.searchParams.entries()).map(([name, value]) => ({ name, value }));
}

function readHeaders(request: IncomingMessage): HttpHeader[] {
  const headers: HttpHeader[] = [];
  for (let index = 0; index + 1 < request.rawHeaders.length; index += 2) {
    const name = request.rawHeaders[index];
    const value = request.rawHeaders[index + 1];
    if (name === undefined || value === undefined) {
      continue;
    }
    headers.push({
      name: name.toLowerCase(),
      value
    });
  }
  if (headers.length > 0) {
    return headers;
  }
  for (const [name, value] of Object.entries(request.headers)) {
    if (value === undefined) {
      continue;
    }
    const values = Array.isArray(value) ? value : [value];
    for (const item of values) {
      headers.push({ name: name.toLowerCase(), value: item });
    }
  }
  return headers;
}

function requestUrl(request: IncomingMessage): URL {
  const protocol = firstForwardedValue(firstHeader(request.headers['x-forwarded-proto'])) ?? 'http';
  const host =
    firstForwardedValue(firstHeader(request.headers['x-forwarded-host'])) ??
    firstHeader(request.headers.host) ??
    'localhost';
  try {
    return new URL(request.url ?? '/', `${protocol}://${host}`);
  } catch (error) {
    throw new DecodeError('request URL is invalid', {
      cause: error instanceof Error ? error.message : String(error)
    });
  }
}

function requestUrlPathname(value: string | undefined): string {
  try {
    return new URL(value ?? '/', 'http://localhost').pathname;
  } catch {
    return '/';
  }
}

function contentLengthBytes(value: string | string[] | undefined): number {
  const header = firstHeader(value);
  if (header === undefined) {
    return 0;
  }
  const parsed = Number(header);
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : 0;
}

function telemetryStatusCode(response: ServerResponse): number {
  if (!response.writableEnded && !response.headersSent && response.statusCode === 200) {
    return 499;
  }
  return response.statusCode;
}

function requestServiceId(request: IncomingMessage, url: URL): string | undefined {
  const selector = requestSelectorValue({
    url,
    queryName: 'service',
    queryErrorCode: 'InvalidHttpServiceQuery',
    headerValue: requestServiceHeader(request),
    headerName: 'X-Skiff-Service'
  });
  const service = selector?.value.trim();
  if (!service) {
    if (selector?.source === 'query') {
      throw new GatewayError(
        400,
        'InvalidHttpServiceQuery',
        'service query must be a valid publication id'
      );
    }
    if (selector?.source === 'header') {
      throw new GatewayError(
        400,
        'InvalidHttpServiceHeader',
        'X-Skiff-Service must be a valid publication id'
      );
    }
    return undefined;
  }
  if (!isPublicationId(service)) {
    throw new GatewayError(
      400,
      selector?.source === 'query' ? 'InvalidHttpServiceQuery' : 'InvalidHttpServiceHeader',
      `${selector?.label ?? 'service selector'} must be a valid publication id`
    );
  }
  return service;
}

function requestVersion(request: IncomingMessage, url: URL): string | undefined {
  const selector = requestSelectorValue({
    url,
    queryName: 'version',
    queryErrorCode: 'InvalidVersionQuery',
    headerValue: requestVersionHeader(request),
    headerName: 'version header'
  });
  const version = selector?.value.trim();
  if (!version) {
    if (selector?.source === 'query') {
      throw new GatewayError(
        400,
        'InvalidVersionQuery',
        'version query must be a valid version'
      );
    }
    return undefined;
  }
  if (!/^[A-Za-z0-9._:-]+$/.test(version)) {
    throw new GatewayError(
      400,
      selector?.source === 'query' ? 'InvalidVersionQuery' : 'InvalidVersionHeader',
      `${selector?.label ?? 'version selector'} must be a valid version`
    );
  }
  return version;
}

interface RequestSelectorInput {
  url: URL;
  queryName: string;
  queryErrorCode: string;
  headerValue: string | undefined;
  headerName: string;
}

interface RequestSelectorValue {
  source: 'query' | 'header';
  label: string;
  value: string;
}

function requestSelectorValue(input: RequestSelectorInput): RequestSelectorValue | undefined {
  const headerValue = input.headerValue?.trim();
  if (headerValue !== undefined && headerValue !== '') {
    return {
      source: 'header',
      label: input.headerName,
      value: headerValue
    };
  }

  const queryValues = input.url.searchParams.getAll(input.queryName);
  if (queryValues.length > 1) {
    throw new GatewayError(
      400,
      input.queryErrorCode,
      `${input.queryName} query parameter must be singular`
    );
  }
  const queryValue = queryValues[0]?.trim();
  if (queryValue !== undefined) {
    return {
      source: 'query',
      label: `${input.queryName} query`,
      value: queryValue
    };
  }
  if (headerValue !== undefined) {
    return {
      source: 'header',
      label: input.headerName,
      value: headerValue
    };
  }
  return undefined;
}

function requestServiceHeader(request: IncomingMessage): string | undefined {
  return singleHeader(
    request.headers['x-skiff-service'],
    'X-Skiff-Service',
    'InvalidHttpServiceHeader'
  )?.trim();
}

function requestVersionHeader(request: IncomingMessage): string | undefined {
  const versionHeader = singleHeader(
    request.headers['x-skiff-version'],
    'X-Skiff-Version',
    'InvalidVersionHeader'
  )?.trim();
  const releaseHeader = singleHeader(
    request.headers['x-skiff-release'],
    'X-Skiff-Release',
    'InvalidVersionHeader'
  )?.trim();

  if (
    versionHeader !== undefined &&
    versionHeader !== '' &&
    releaseHeader !== undefined &&
    releaseHeader !== '' &&
    versionHeader !== releaseHeader
  ) {
    throw new GatewayError(
      400,
      'InvalidVersionHeader',
      'X-Skiff-Version conflicts with X-Skiff-Release'
    );
  }
  return versionHeader !== undefined && versionHeader !== '' ? versionHeader : releaseHeader;
}

function rawDispatchKey(serviceId: string, buildId: string | undefined): string {
  return `${serviceId}\0${buildId ?? ''}`;
}

function httpRouteDispatchKey(
  serviceId: string,
  buildId: string,
  method: string,
  path: string
): string {
  return `${serviceId}\0${buildId}\0${method}\0${path}`;
}

function normalizeRequestMethod(value: string | undefined): string {
  return (value ?? 'GET').toUpperCase();
}

function firstForwardedValue(value: string | undefined): string | undefined {
  return value?.split(',')[0]?.trim();
}

function firstHeader(value: string | string[] | undefined): string | undefined {
  if (Array.isArray(value)) {
    return value[0];
  }
  return value;
}

function singleHeader(
  value: string | string[] | undefined,
  headerName: string,
  errorCode: string
): string | undefined {
  if (Array.isArray(value)) {
    if (value.length > 1) {
      throw new GatewayError(400, errorCode, `${headerName} must be singular`);
    }
    return singleHeader(value[0], headerName, errorCode);
  }
  if (value !== undefined && value.includes(',')) {
    throw new GatewayError(400, errorCode, `${headerName} must be singular`);
  }
  return value;
}

function decodePathname(pathname: string): string {
  try {
    return decodeURIComponent(pathname);
  } catch {
    return pathname;
  }
}

function readRequiredString(value: unknown, name: string): string {
  if (typeof value !== 'string') {
    throw new GatewayError(502, 'InvalidHttpResponse', `${name} must be a string`);
  }
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isValidHeaderName(name: string): boolean {
  return /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name);
}

function validateHeaderValue(name: string, value: unknown): string {
  if (typeof value !== 'string') {
    throw new GatewayError(502, 'InvalidHttpResponse', `response header ${name} must be a string`);
  }
  if (value.includes('\r') || value.includes('\n')) {
    throw new GatewayError(
      502,
      'InvalidHttpResponse',
      `response header ${name} must not contain CRLF`
    );
  }
  return value;
}
