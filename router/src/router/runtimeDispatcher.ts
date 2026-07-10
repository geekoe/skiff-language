import { randomUUID } from 'node:crypto';

import WebSocket from 'ws';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type PackageTestStartFrameHeader,
  type RequestCancelEnvelope,
  type RequestCancelReason,
  type RequestStartFrameHeader,
  type ResponseChunkFrameHeader,
  type ResponseEndFrameHeader,
  type ResponseErrorFrameHeader,
  type ResponseStartFrameHeader,
  type RouterToRuntimeFrameHeader,
  type RuntimeErrorPayload
} from '../protocol/envelope.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation
} from '../protocol/cancelReason.js';
import type {
  RuntimeActorExecution,
  RuntimeDispatchConnection,
  RuntimeDispatchFrameHeader,
  RuntimeInFlightRequest,
  RuntimeRegistry,
  RuntimeRegistryRuntime
} from './runtimeRegistry.js';
import {
  GatewayError,
  ProviderUnavailableError,
  RuntimeResponseError,
  RuntimeTimeoutError,
  toGatewayError
} from './errors.js';

const DEFAULT_RUNTIME_ORIGINATED_TIMEOUT_MS = 2000;

export type RuntimeFrameSendCallback = (error?: Error) => void;

export interface RuntimeFrameSender {
  sendFrame(
    ws: WebSocket,
    header: RouterToRuntimeFrameHeader,
    payloadBytes?: Uint8Array,
    callback?: RuntimeFrameSendCallback
  ): void;
}

interface RuntimeInvocationBase extends RuntimeInFlightRequest {
  timeout: NodeJS.Timeout;
  reject(error: unknown): void;
  abortCleanup?: () => void;
}

export interface RuntimeUnaryInvocation extends RuntimeInvocationBase {
  kind: 'unary';
  request: RequestStartFrameHeader;
  resolve(response: RuntimeBinaryDispatchResponse): void;
}

export interface RuntimeUnaryFrameInvocation extends RuntimeInvocationBase {
  kind: 'unaryFrame';
  resolve(response: RuntimeBinaryDispatchResult): void;
}

export interface RuntimeStreamInvocation extends RuntimeInvocationBase {
  kind: 'stream';
  request: RequestStartFrameHeader;
  resolve(response: RuntimeBinaryDispatchResponse): void;
  started: boolean;
  nextSeq: number;
  onStart(response: RuntimeBinaryDispatchStart): void;
  onChunk(response: RuntimeBinaryDispatchChunk): void;
}

export interface RuntimeForwardInvocation extends RuntimeInvocationBase {
  kind: 'forward';
  request: RequestStartFrameHeader;
  callerRequestId: string;
  callerWs: WebSocket;
  started: boolean;
  nextSeq: number;
  actorExecution?: RuntimeActorExecution;
}

export type RuntimeInvocation =
  | RuntimeUnaryInvocation
  | RuntimeUnaryFrameInvocation
  | RuntimeStreamInvocation
  | RuntimeForwardInvocation;

export interface RuntimeBinaryDispatchResponse {
  header: ResponseEndFrameHeader;
  payloadBytes: Uint8Array;
}

export interface RuntimeBinaryDispatchError {
  header: ResponseErrorFrameHeader;
  payloadBytes: Uint8Array;
}

export type RuntimeBinaryDispatchResult =
  | RuntimeBinaryDispatchResponse
  | RuntimeBinaryDispatchError;

export interface RuntimeBinaryDispatchStart {
  header: ResponseStartFrameHeader;
}

export interface RuntimeBinaryDispatchChunk {
  header: ResponseChunkFrameHeader;
  payloadBytes: Uint8Array;
}

export interface RuntimeBinaryDispatchInput<
  THeader extends RuntimeDispatchFrameHeader = RequestStartFrameHeader
> {
  header: THeader;
  payloadBytes: Uint8Array;
}

export interface RuntimeBinaryDispatchOptions {
  signal?: AbortSignal;
  cancelReason?: RequestCancelReason;
}

export interface RuntimeBinaryStreamHandlers {
  onStart(response: RuntimeBinaryDispatchStart): void;
  onChunk(response: RuntimeBinaryDispatchChunk): void;
}

export interface RuntimeDispatcherOptions {
  frameSender: RuntimeFrameSender;
  registry: RuntimeRegistry;
}

export class RuntimeDispatcher {
  private readonly pending = new Map<string, RuntimeInvocation>();
  private readonly forwardedRequestIdsByCaller = new Map<WebSocket, Map<string, string>>();

  constructor(private readonly options: RuntimeDispatcherOptions) {
    this.options.registry.setInFlightCounter({
      countInFlight: (runtime) => this.countInFlight(runtime)
    });
  }

  dispatch(request: unknown, timeoutMs: number): Promise<unknown> {
    void request;
    void timeoutMs;
    return Promise.reject(
      new RuntimeResponseError({
        code: 'UnsupportedRuntimeTransport',
        message:
          'text JSON request.start is not supported; use typed binary runtime frames'
      })
    );
  }

  dispatchBinary(
    request: RuntimeBinaryDispatchInput<RequestStartFrameHeader>,
    timeoutMs: number,
    options: RuntimeBinaryDispatchOptions = {}
  ): Promise<RuntimeBinaryDispatchResponse> {
    const connection = this.options.registry.pickDispatchConnection(request.header);
    if (connection instanceof GatewayError) {
      return Promise.reject(connection);
    }
    if (!connection) {
      return Promise.reject(new ProviderUnavailableError());
    }
    const dispatchHeader = dispatchHeaderForConnection(request.header, connection);

    return new Promise<RuntimeBinaryDispatchResponse>((resolve, reject) => {
      const timeout = setTimeout(() => {
        const pending = this.pending.get(dispatchHeader.requestId);
        this.completePending(dispatchHeader.requestId, pending);
        this.sendCancel(connection.ws, {
          type: 'request.cancel',
          requestId: dispatchHeader.requestId,
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
        });
        this.options.registry.refreshRuntimeStatesForRequest(pending);
        reject(new RuntimeTimeoutError(timeoutMs));
      }, timeoutMs);

      const abortCleanup = this.attachAbortHandler(
        connection,
        dispatchHeader.requestId,
        options,
        reject
      );
      this.pending.set(dispatchHeader.requestId, {
        kind: 'unary',
        ...(connection.runtimeId !== undefined ? { runtimeId: connection.runtimeId } : {}),
        request: dispatchHeader,
        timeout,
        ws: connection.ws,
        resolve,
        reject,
        ...(abortCleanup ? { abortCleanup } : {})
      });

      this.options.frameSender.sendFrame(
        connection.ws,
        dispatchHeader,
        request.payloadBytes,
        (error) => {
          if (!error) {
            return;
          }
          const pending = this.pending.get(dispatchHeader.requestId);
          this.completePending(dispatchHeader.requestId, pending);
          this.options.registry.refreshRuntimeStatesForRequest(pending);
          reject(new ProviderUnavailableError(error.message));
        }
      );
    });
  }

  dispatchBinaryFrame(
    request: RuntimeBinaryDispatchInput<RuntimeDispatchFrameHeader>,
    timeoutMs: number,
    options: RuntimeBinaryDispatchOptions = {}
  ): Promise<RuntimeBinaryDispatchResult> {
    const connection = this.options.registry.pickDispatchConnection(request.header);
    if (connection instanceof GatewayError) {
      return Promise.reject(connection);
    }
    if (!connection) {
      return Promise.reject(new ProviderUnavailableError());
    }
    const dispatchHeader = dispatchHeaderForConnection(request.header, connection);

    return new Promise<RuntimeBinaryDispatchResult>((resolve, reject) => {
      const timeout = setTimeout(() => {
        const pending = this.pending.get(dispatchHeader.requestId);
        this.completePending(dispatchHeader.requestId, pending);
        this.sendCancel(connection.ws, {
          type: 'request.cancel',
          requestId: dispatchHeader.requestId,
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
        });
        this.options.registry.refreshRuntimeStatesForRequest(pending);
        reject(new RuntimeTimeoutError(timeoutMs));
      }, timeoutMs);

      const abortCleanup = this.attachAbortHandler(
        connection,
        dispatchHeader.requestId,
        options,
        reject
      );
      this.pending.set(dispatchHeader.requestId, {
        kind: 'unaryFrame',
        ...(connection.runtimeId !== undefined ? { runtimeId: connection.runtimeId } : {}),
        request: dispatchHeader,
        timeout,
        ws: connection.ws,
        resolve,
        reject,
        ...(abortCleanup ? { abortCleanup } : {})
      });

      this.options.frameSender.sendFrame(
        connection.ws,
        dispatchHeader,
        request.payloadBytes,
        (error) => {
          if (!error) {
            return;
          }
          const pending = this.pending.get(dispatchHeader.requestId);
          this.completePending(dispatchHeader.requestId, pending);
          this.options.registry.refreshRuntimeStatesForRequest(pending);
          reject(new ProviderUnavailableError(error.message));
        }
      );
    });
  }

  dispatchBinaryStream(
    request: RuntimeBinaryDispatchInput<RequestStartFrameHeader>,
    timeoutMs: number,
    handlers: RuntimeBinaryStreamHandlers,
    options: RuntimeBinaryDispatchOptions = {}
  ): Promise<RuntimeBinaryDispatchResponse> {
    const connection = this.options.registry.pickDispatchConnection(request.header);
    if (connection instanceof GatewayError) {
      return Promise.reject(connection);
    }
    if (!connection) {
      return Promise.reject(new ProviderUnavailableError());
    }

    if (request.header.mode !== 'serverStream') {
      return Promise.reject(
        new RuntimeResponseError({
          code: 'InvalidDispatchMode',
          message: `stream dispatch requires request.start mode serverStream, got ${request.header.mode}`
        })
      );
    }
    const dispatchHeader = dispatchHeaderForConnection(request.header, connection);

    return new Promise<RuntimeBinaryDispatchResponse>((resolve, reject) => {
      const timeout = setTimeout(() => {
        const pending = this.pending.get(dispatchHeader.requestId);
        this.completePending(dispatchHeader.requestId, pending);
        this.sendCancel(connection.ws, {
          type: 'request.cancel',
          requestId: dispatchHeader.requestId,
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
        });
        this.options.registry.refreshRuntimeStatesForRequest(pending);
        reject(new RuntimeTimeoutError(timeoutMs));
      }, timeoutMs);

      const abortCleanup = this.attachAbortHandler(
        connection,
        dispatchHeader.requestId,
        options,
        reject
      );
      this.pending.set(dispatchHeader.requestId, {
        kind: 'stream',
        ...(connection.runtimeId !== undefined ? { runtimeId: connection.runtimeId } : {}),
        request: dispatchHeader,
        timeout,
        ws: connection.ws,
        resolve,
        reject,
        started: false,
        nextSeq: 0,
        onStart: handlers.onStart,
        onChunk: handlers.onChunk,
        ...(abortCleanup ? { abortCleanup } : {})
      });

      this.options.frameSender.sendFrame(
        connection.ws,
        dispatchHeader,
        request.payloadBytes,
        (error) => {
          if (!error) {
            return;
          }
          const pending = this.pending.get(dispatchHeader.requestId);
          this.completePending(dispatchHeader.requestId, pending);
          this.options.registry.refreshRuntimeStatesForRequest(pending);
          reject(new ProviderUnavailableError(error.message));
        }
      );
    });
  }

  close(): void {
    for (const [requestId, pending] of Array.from(this.pending.entries())) {
      clearTimeout(pending.timeout);
      pending.abortCleanup?.();
      this.sendCancel(pending.ws, {
        type: 'request.cancel',
        requestId,
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.routerShutdown)
      });
      pending.reject(new ProviderUnavailableError('Runtime registry is closing'));
    }
    this.pending.clear();
    this.forwardedRequestIdsByCaller.clear();
    this.options.registry.refreshAllRuntimeStates();
  }

  countInFlight(runtime: RuntimeRegistryRuntime): number {
    let count = 0;
    for (const pending of this.pending.values()) {
      if (this.pendingBelongsToRuntime(pending, runtime)) {
        count += 1;
      }
    }
    return count;
  }

  handleRuntimeRequestStart(
    callerWs: WebSocket,
    request: RuntimeBinaryDispatchInput
  ): void {
    this.options.registry.validateRuntimeRequestStartSource(callerWs, request.header);
    const callerRequestId = request.header.requestId;
    if (this.forwardedRequestIdsByCaller.get(callerWs)?.has(callerRequestId)) {
      this.sendRuntimeErrorResponse(callerWs, callerRequestId, {
        code: 'DuplicateRequestId',
        message: `runtime-originated request.start requestId ${callerRequestId} is already pending`
      });
      return;
    }

    const timeoutMs = this.resolveRuntimeOriginatedTimeoutMs(request.header);
    const forwardedRequestId = this.createForwardedRequestId();
    const forwardedHeader: RequestStartFrameHeader = {
      ...request.header,
      requestId: forwardedRequestId
    };
    const connection = this.options.registry.pickDispatchConnection(forwardedHeader);
    if (connection instanceof GatewayError) {
      this.sendRuntimeErrorResponse(callerWs, callerRequestId, connection.toPayload());
      return;
    }
    if (!connection) {
      this.sendRuntimeErrorResponse(
        callerWs,
        callerRequestId,
        new ProviderUnavailableError().toPayload()
      );
      return;
    }
    const dispatchHeader = dispatchHeaderForConnection(forwardedHeader, connection);

    const timeout = setTimeout(() => {
      const pending = this.pending.get(forwardedRequestId);
      this.completePending(forwardedRequestId, pending);
      this.sendCancel(connection.ws, {
        type: 'request.cancel',
        requestId: forwardedRequestId,
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
      });
      this.options.registry.refreshRuntimeStatesForRequest(pending);
      this.sendRuntimeErrorResponse(
        callerWs,
        callerRequestId,
        new RuntimeTimeoutError(timeoutMs).toPayload()
      );
    }, timeoutMs);

    this.trackForwardedRequest(callerWs, callerRequestId, forwardedRequestId);
    this.pending.set(forwardedRequestId, {
      kind: 'forward',
      ...(connection.runtimeId !== undefined ? { runtimeId: connection.runtimeId } : {}),
      request: dispatchHeader,
      timeout,
      ws: connection.ws,
      callerRequestId,
      callerWs,
      started: false,
      nextSeq: 0,
      reject: (error: unknown) => {
        this.sendRuntimeErrorResponse(
          callerWs,
          callerRequestId,
          this.runtimeErrorPayloadFromUnknown(error)
        );
      }
    });

    this.options.frameSender.sendFrame(
      connection.ws,
      dispatchHeader,
      request.payloadBytes,
      (error) => {
        if (!error) {
          return;
        }
        const pending = this.pending.get(forwardedRequestId);
        if (!pending) {
          return;
        }
        this.completePending(forwardedRequestId, pending);
        this.options.registry.refreshRuntimeStatesForRequest(pending);
        this.sendRuntimeErrorResponse(
          callerWs,
          callerRequestId,
          new ProviderUnavailableError(error.message).toPayload()
        );
      }
    );
  }

  handleRuntimeCancel(ws: WebSocket, envelope: RequestCancelEnvelope): void {
    if (typeof envelope.requestId !== 'string') {
      throw new Error('invalid request.cancel envelope');
    }

    const forwardedRequestId = this.forwardedRequestIdsByCaller.get(ws)?.get(envelope.requestId);
    if (forwardedRequestId !== undefined) {
      const pending = this.pending.get(forwardedRequestId);
      if (!pending || pending.kind !== 'forward' || pending.callerWs !== ws) {
        return;
      }
      this.completePending(forwardedRequestId, pending);
      this.sendCancel(pending.ws, {
        type: 'request.cancel',
        requestId: forwardedRequestId,
        reason: envelope.reason
      });
      this.options.registry.refreshRuntimeStatesForRequest(pending);
      this.finishPendingActorExecution(pending, 'cancelled', envelope.reason);
      return;
    }

    const pending = this.pending.get(envelope.requestId);
    if (!pending) {
      return;
    }
    if (!this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }

    this.completePending(envelope.requestId, pending);
    this.options.registry.refreshRuntimeStatesForRequest(pending);
    pending.reject(
      new ProviderUnavailableError(`Runtime cancelled request: ${String(envelope.reason)}`)
    );
  }

  resolveRequest(
    ws: WebSocket,
    response: RuntimeBinaryDispatchResponse
  ): void {
    const requestId = response.header.requestId;
    const pending = this.pending.get(requestId);
    if (!pending) {
      return;
    }
    if (!this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }
    if (pending.kind === 'forward') {
      this.forwardResponseEnd(ws, response, pending);
      return;
    }
    if (pending.kind === 'stream') {
      if (!pending.started) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'StreamProtocolError',
          message: 'response.end received before response.start'
        });
        return;
      }
      if (response.header.payloadPresent || response.payloadBytes.byteLength !== 0) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'StreamProtocolError',
          message: 'streaming response.end must not include a payload'
        });
        return;
      }
      if (response.header.httpResponse !== undefined) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'StreamProtocolError',
          message: 'streaming response.end must not include httpResponse metadata'
        });
        return;
      }
    }
    this.completePending(requestId, pending);
    this.options.registry.refreshRuntimeStatesForRequest(pending);
    pending.resolve(response);
  }

  rejectRequest(
    ws: WebSocket,
    envelope: Pick<ResponseErrorFrameHeader, 'requestId' | 'error'>
  ): void {
    const pending = this.pending.get(envelope.requestId);
    if (!pending) {
      return;
    }
    if (!this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }
    if (pending.kind === 'forward') {
      this.completePending(envelope.requestId, pending);
      this.options.registry.refreshRuntimeStatesForRequest(pending);
      this.finishPendingActorExecution(pending, 'failed', envelope.error.message);
      this.sendRuntimeErrorResponse(
        pending.callerWs,
        pending.callerRequestId,
        envelope.error
      );
      return;
    }
    if (pending.kind === 'unaryFrame') {
      this.completePending(envelope.requestId, pending);
      this.options.registry.refreshRuntimeStatesForRequest(pending);
      pending.resolve({
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: envelope.requestId,
          error: envelope.error
        },
        payloadBytes: new Uint8Array()
      });
      return;
    }
    this.completePending(envelope.requestId, pending);
    this.options.registry.refreshRuntimeStatesForRequest(pending);
    pending.reject(new RuntimeResponseError(envelope.error));
  }

  handleResponseStart(
    ws: WebSocket,
    response: RuntimeBinaryDispatchStart,
    payloadBytes: Uint8Array
  ): void {
    const requestId = response.header.requestId;
    const pending = this.pending.get(requestId);
    if (!pending) {
      return;
    }
    if (!this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }
    if (pending.kind === 'forward') {
      this.forwardResponseStart(ws, response, payloadBytes, pending);
      return;
    }
    if (pending.kind !== 'stream') {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'UnexpectedStart',
        message: 'response.start is only valid for serverStream dispatch'
      });
      return;
    }
    if (pending.started) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: 'duplicate response.start frame'
      });
      return;
    }
    if (payloadBytes.byteLength !== 0) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: 'response.start payload must be empty'
      });
      return;
    }
    try {
      pending.onStart(response);
    } catch (error) {
      this.rejectPendingWithError(ws, requestId, error);
      return;
    }
    clearTimeout(pending.timeout);
    pending.started = true;
  }

  handleResponseChunk(
    ws: WebSocket,
    response: RuntimeBinaryDispatchChunk
  ): void {
    const requestId = response.header.requestId;
    const pending = this.pending.get(requestId);
    if (!pending) {
      return;
    }
    if (!this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }
    if (pending.kind === 'forward') {
      this.forwardResponseChunk(ws, response, pending);
      return;
    }
    if (pending.kind !== 'stream') {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'UnexpectedChunk',
        message: 'response.chunk is only valid for serverStream dispatch'
      });
      return;
    }
    if (!pending.started) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: 'response.chunk received before response.start'
      });
      return;
    }
    if (response.header.seq !== pending.nextSeq) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: `response.chunk seq ${response.header.seq} does not match expected seq ${pending.nextSeq}`
      });
      return;
    }
    try {
      pending.onChunk(response);
    } catch (error) {
      this.rejectPendingWithError(ws, requestId, error);
      return;
    }
    pending.nextSeq += 1;
  }

  handleRuntimeDisconnect(ws: WebSocket): void {
    for (const [requestId, pending] of Array.from(this.pending.entries())) {
      if (pending.ws === ws) {
        this.completePending(requestId, pending);
        if (pending.kind === 'forward') {
          this.finishPendingActorExecution(
            pending,
            'failed',
            'Runtime disconnected before responding'
          );
        }
        pending.reject(new ProviderUnavailableError('Runtime disconnected before responding'));
        continue;
      }
      if (pending.kind === 'forward' && pending.callerWs === ws) {
        this.completePending(requestId, pending);
        this.sendCancel(pending.ws, {
          type: 'request.cancel',
          requestId,
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.runtimeDisconnect)
        });
        this.options.registry.refreshRuntimeStatesForRequest(pending);
        this.finishPendingActorExecution(pending, 'cancelled', 'caller runtime disconnected');
      }
    }
    this.forwardedRequestIdsByCaller.delete(ws);
  }

  private resolveRuntimeOriginatedTimeoutMs(request: RequestStartFrameHeader): number {
    if (request.deadline === undefined) {
      return DEFAULT_RUNTIME_ORIGINATED_TIMEOUT_MS;
    }
    if (!Number.isFinite(request.deadline.timeoutMs) || request.deadline.timeoutMs <= 0) {
      throw new Error('runtime-originated request.start deadline.timeoutMs must be positive');
    }
    const expiresAtMs = Date.parse(request.deadline.expiresAt);
    if (!Number.isFinite(expiresAtMs)) {
      throw new Error(
        'runtime-originated request.start deadline.expiresAt must be an ISO timestamp'
      );
    }
    return Math.max(0, Math.min(request.deadline.timeoutMs, expiresAtMs - Date.now()));
  }

  private createForwardedRequestId(): string {
    let requestId: string;
    do {
      requestId = `router-forward:${randomUUID()}`;
    } while (this.pending.has(requestId));
    return requestId;
  }

  private trackForwardedRequest(
    callerWs: WebSocket,
    callerRequestId: string,
    forwardedRequestId: string
  ): void {
    let requests = this.forwardedRequestIdsByCaller.get(callerWs);
    if (!requests) {
      requests = new Map();
      this.forwardedRequestIdsByCaller.set(callerWs, requests);
    }
    requests.set(callerRequestId, forwardedRequestId);
  }

  private forwardResponseStart(
    ws: WebSocket,
    response: RuntimeBinaryDispatchStart,
    payloadBytes: Uint8Array,
    pending: RuntimeForwardInvocation
  ): void {
    const requestId = response.header.requestId;
    if (pending.request.mode !== 'serverStream') {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'UnexpectedStart',
        message: 'response.start is only valid for serverStream dispatch'
      });
      return;
    }
    if (pending.started) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: 'duplicate response.start frame'
      });
      return;
    }
    if (payloadBytes.byteLength !== 0) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: 'response.start payload must be empty'
      });
      return;
    }

    const header: ResponseStartFrameHeader = {
      ...response.header,
      requestId: pending.callerRequestId
    };
    this.options.frameSender.sendFrame(pending.callerWs, header);
    clearTimeout(pending.timeout);
    pending.started = true;
  }

  private forwardResponseChunk(
    ws: WebSocket,
    response: RuntimeBinaryDispatchChunk,
    pending: RuntimeForwardInvocation
  ): void {
    const requestId = response.header.requestId;
    if (pending.request.mode !== 'serverStream') {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'UnexpectedChunk',
        message: 'response.chunk is only valid for serverStream dispatch'
      });
      return;
    }
    if (!pending.started) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: 'response.chunk received before response.start'
      });
      return;
    }
    if (response.header.seq !== pending.nextSeq) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: `response.chunk seq ${response.header.seq} does not match expected seq ${pending.nextSeq}`
      });
      return;
    }

    const header: ResponseChunkFrameHeader = {
      ...response.header,
      requestId: pending.callerRequestId
    };
    this.options.frameSender.sendFrame(pending.callerWs, header, response.payloadBytes);
    pending.nextSeq += 1;
  }

  private forwardResponseEnd(
    ws: WebSocket,
    response: RuntimeBinaryDispatchResponse,
    pending: RuntimeForwardInvocation
  ): void {
    const requestId = response.header.requestId;
    if (pending.request.mode === 'serverStream') {
      if (!pending.started) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'StreamProtocolError',
          message: 'response.end received before response.start'
        });
        return;
      }
      if (response.header.payloadPresent || response.payloadBytes.byteLength !== 0) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'StreamProtocolError',
          message: 'streaming response.end must not include a payload'
        });
        return;
      }
      if (response.header.httpResponse !== undefined) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'StreamProtocolError',
          message: 'streaming response.end must not include httpResponse metadata'
        });
        return;
      }
    } else if (pending.started) {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'StreamProtocolError',
        message: 'response.start is not valid for unary request forwarding'
      });
      return;
    }

    this.completePending(requestId, pending);
    this.options.registry.refreshRuntimeStatesForRequest(pending);
    this.finishPendingActorExecution(pending, 'completed');
    const header: ResponseEndFrameHeader = {
      ...response.header,
      requestId: pending.callerRequestId
    };
    this.options.frameSender.sendFrame(pending.callerWs, header, response.payloadBytes);
  }

  private rejectPendingRuntimeError(
    ws: WebSocket,
    requestId: string,
    error: { code: string; message: string; details?: unknown }
  ): void {
    this.rejectPendingWithError(ws, requestId, new RuntimeResponseError(error));
  }

  private rejectPendingWithError(ws: WebSocket, requestId: string, error: unknown): void {
    const pending = this.pending.get(requestId);
    if (!pending || !this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }
    this.completePending(requestId, pending);
    this.options.registry.refreshRuntimeStatesForRequest(pending);
    if (pending.kind === 'forward') {
      this.finishPendingActorExecution(pending, 'failed', String(error));
    }
    pending.reject(error);
  }

  private completePending(requestId: string, pending: RuntimeInvocation | undefined): void {
    if (!pending) {
      return;
    }
    clearTimeout(pending.timeout);
    pending.abortCleanup?.();
    this.pending.delete(requestId);
    if (pending.kind === 'forward') {
      this.untrackForwardedRequest(pending.callerWs, pending.callerRequestId);
    }
  }

  private untrackForwardedRequest(callerWs: WebSocket, callerRequestId: string): void {
    const requests = this.forwardedRequestIdsByCaller.get(callerWs);
    if (!requests) {
      return;
    }
    requests.delete(callerRequestId);
    if (requests.size === 0) {
      this.forwardedRequestIdsByCaller.delete(callerWs);
    }
  }

  private attachAbortHandler(
    connection: RuntimeDispatchConnection,
    requestId: string,
    options: RuntimeBinaryDispatchOptions,
    reject: (error: unknown) => void
  ): (() => void) | undefined {
    const signal = options.signal;
    if (!signal) {
      return undefined;
    }
    const abort = () => {
      const pending = this.pending.get(requestId);
      if (!pending) {
        return;
      }
      this.completePending(requestId, pending);
      this.sendCancel(connection.ws, {
        type: 'request.cancel',
        requestId,
        reason:
          options.cancelReason ??
          requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.callerAbort)
      });
      this.options.registry.refreshRuntimeStatesForRequest(pending);
      reject(new ProviderUnavailableError('Runtime request was cancelled before completion'));
    };
    if (signal.aborted) {
      queueMicrotask(abort);
      return undefined;
    }
    signal.addEventListener('abort', abort, { once: true });
    return () => signal.removeEventListener('abort', abort);
  }

  private sendCancel(ws: WebSocket, cancel: RequestCancelEnvelope): void {
    if (ws.readyState !== WebSocket.OPEN) {
      return;
    }
    this.options.frameSender.sendFrame(ws, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.cancel',
      requestId: cancel.requestId,
      reason: cancel.reason
    });
  }

  private sendRuntimeErrorResponse(
    ws: WebSocket,
    requestId: string,
    error: RuntimeErrorPayload
  ): void {
    this.options.frameSender.sendFrame(ws, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId,
      error
    });
  }

  private runtimeErrorPayloadFromUnknown(error: unknown): RuntimeErrorPayload {
    if (error instanceof GatewayError) {
      return error.toPayload();
    }
    return toGatewayError(error).toPayload();
  }

  private isPendingRuntimeSocket(ws: WebSocket, pending: RuntimeInvocation): boolean {
    return pending.ws === ws;
  }

  private pendingBelongsToRuntime(
    pending: RuntimeInvocation,
    runtime: RuntimeRegistryRuntime
  ): boolean {
    if (pending.runtimeId !== undefined) {
      return pending.runtimeId === runtime.runtimeId;
    }
    if (pending.ws !== runtime.ws) {
      return false;
    }
    const request = pending.request;
    if (request.type === 'package-test.start') {
      return false;
    }
    if (request.serviceId !== undefined && request.serviceId !== runtime.serviceId) {
      return false;
    }
    return (
      runtime.buildId === request.buildId &&
      runtime.serviceProtocolIdentity === request.serviceProtocolIdentity &&
      runtime.targets.has(request.target) &&
      runtimeAcceptsGatewayEntry(runtime, request.gatewayEntryIdentity) &&
      (request.activationIdentity === undefined ||
        runtime.activationIdentity === request.activationIdentity)
    );
  }

  private finishPendingActorExecution(
    pending: RuntimeForwardInvocation,
    terminalState: 'completed' | 'failed' | 'cancelled',
    terminalReason?: string
  ): void {
    this.options.registry.finishActorExecution(
      pending.actorExecution,
      terminalState,
      terminalReason
    );
  }
}

function dispatchHeaderForConnection(
  header: RequestStartFrameHeader,
  connection: RuntimeDispatchConnection
): RequestStartFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeDispatchFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeDispatchFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeDispatchFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeDispatchFrameHeader {
  if (
    header.type !== 'request.start' ||
    connection.dispatchBuildId === undefined ||
    header.buildId === connection.dispatchBuildId
  ) {
    return header;
  }
  return {
    ...header,
    buildId: connection.dispatchBuildId
  };
}

function runtimeAcceptsGatewayEntry(
  runtime: RuntimeRegistryRuntime,
  gatewayEntryIdentity: string | undefined
): boolean {
  const hasGatewayEntryIdentityIndex = (runtime.gatewayEntryIdentities?.size ?? 0) > 0;
  return (
    gatewayEntryIdentity === undefined ||
    !hasGatewayEntryIdentityIndex ||
    runtime.gatewayEntryIdentities?.has(gatewayEntryIdentity) === true
  );
}
