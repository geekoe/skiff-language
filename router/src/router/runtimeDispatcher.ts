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
  type RouterToRuntimeFrameHeader
} from '../protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../protocol/runtimeAssemblyRequest.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation
} from '../protocol/cancelReason.js';
import type {
  RuntimeDispatchConnection,
  RuntimeDispatchFrameHeader,
  RuntimeDispatchRuntimeIdentity,
  RuntimeInFlightRequest,
  RuntimeRegistry,
  RuntimeUnaryDispatchFrameHeader
} from './runtimeRegistry.js';
import { isRuntimeAssemblyRequestDispatchHeader } from './runtimeRegistry.js';
import {
  GatewayError,
  ProviderUnavailableError,
  RuntimeResponseError,
  RuntimeTimeoutError
} from './errors.js';

export type RuntimeFrameSendCallback = (error?: Error) => void;

export type StreamPendingState = 'waitingStart' | 'streaming' | 'terminal';

export type PendingTerminalSource =
  | 'runtime_response_end'
  | 'runtime_response_error'
  | 'runtime_request_cancel'
  | 'timeout'
  | 'caller_abort'
  | 'client_disconnect'
  | 'backpressure'
  | 'protocol_error'
  | 'callback_error'
  | 'runtime_disconnect'
  | 'router_shutdown';

export type PendingTerminal =
  | { source: PendingTerminalSource; kind: 'completed' }
  | { source: PendingTerminalSource; kind: 'failed'; error: unknown }
  | { source: PendingTerminalSource; kind: 'cancelled'; reason?: RequestCancelReason };

export interface RuntimeFrameSender {
  sendFrame(
    ws: WebSocket,
    header: RouterToRuntimeFrameHeader | RuntimeAssemblyRequestStartFrameHeader,
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
  request: RuntimeUnaryDispatchFrameHeader;
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
  streamState: StreamPendingState;
  nextSeq: number;
  onStart(response: RuntimeBinaryDispatchStart, requestTerminal: RuntimeStreamRequestTerminal): void;
  onChunk(response: RuntimeBinaryDispatchChunk, requestTerminal: RuntimeStreamRequestTerminal): void;
  onEnd(response: RuntimeBinaryDispatchResponse, requestTerminal: RuntimeStreamRequestTerminal): void;
  closeFromPendingTerminal?(terminal: PendingTerminal): void;
}

export type RuntimeInvocation =
  | RuntimeUnaryInvocation
  | RuntimeUnaryFrameInvocation
  | RuntimeStreamInvocation;

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
  THeader extends RuntimeDispatchFrameHeader = RuntimeUnaryDispatchFrameHeader
> {
  header: THeader;
  payloadBytes: Uint8Array;
}

export interface RuntimeBinaryDispatchOptions {
  signal?: AbortSignal;
  cancelReason?: RequestCancelReason;
  /** Keep an already-established stream/connection on its selected runtime. */
  connection?: RuntimeDispatchConnection;
}

export type RuntimeStreamRequestTerminal = (terminal: PendingTerminal) => void;

export interface RuntimeBinaryStreamHandlers {
  onStart(response: RuntimeBinaryDispatchStart, requestTerminal: RuntimeStreamRequestTerminal): void;
  onChunk(response: RuntimeBinaryDispatchChunk, requestTerminal: RuntimeStreamRequestTerminal): void;
  onEnd(response: RuntimeBinaryDispatchResponse, requestTerminal: RuntimeStreamRequestTerminal): void;
  closeFromPendingTerminal?(terminal: PendingTerminal): void;
}

export interface RuntimeDispatcherOptions {
  frameSender: RuntimeFrameSender;
  registry: RuntimeDispatchRegistry;
}

export type RuntimeDispatchRegistry = Pick<
  RuntimeRegistry,
  | 'setInFlightCounter'
  | 'pickDispatchConnection'
  | 'refreshAllRuntimeStates'
  | 'refreshRuntimeStatesForRequest'
>;

export interface RuntimeDispatcherPendingCounters {
  pendingUnary: number;
  pendingStream: number;
}

export class RuntimeDispatcher {
  private readonly pending = new Map<string, RuntimeInvocation>();

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
    request: RuntimeBinaryDispatchInput<RuntimeUnaryDispatchFrameHeader>,
    timeoutMs: number,
    options: RuntimeBinaryDispatchOptions = {}
  ): Promise<RuntimeBinaryDispatchResponse> {
    const connection =
      options.connection ?? this.options.registry.pickDispatchConnection(request.header);
    if (connection instanceof GatewayError) {
      return Promise.reject(connection);
    }
    if (!connection) {
      return Promise.reject(new ProviderUnavailableError());
    }
    if (connection.ws.readyState !== WebSocket.OPEN) {
      return Promise.reject(new ProviderUnavailableError('Pinned runtime disconnected'));
    }
    const dispatchHeader = dispatchHeaderForConnection(request.header, connection);

    return new Promise<RuntimeBinaryDispatchResponse>((resolve, reject) => {
      const timeout = setTimeout(() => {
        const pending = this.pending.get(dispatchHeader.requestId);
        this.finishPending(dispatchHeader.requestId, pending, {
          source: 'timeout',
          kind: 'cancelled',
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
        });
        reject(new RuntimeTimeoutError(timeoutMs));
      }, timeoutMs);

      const abortCleanup = this.attachAbortHandler(
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
          const providerError = new ProviderUnavailableError(error.message);
          this.finishPending(dispatchHeader.requestId, pending, {
            source: 'callback_error',
            kind: 'failed',
            error: providerError
          });
          reject(providerError);
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
        this.finishPending(dispatchHeader.requestId, pending, {
          source: 'timeout',
          kind: 'cancelled',
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
        });
        reject(new RuntimeTimeoutError(timeoutMs));
      }, timeoutMs);

      const abortCleanup = this.attachAbortHandler(
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
          const providerError = new ProviderUnavailableError(error.message);
          this.finishPending(dispatchHeader.requestId, pending, {
            source: 'callback_error',
            kind: 'failed',
            error: providerError
          });
          reject(providerError);
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
        this.finishPending(dispatchHeader.requestId, pending, {
          source: 'timeout',
          kind: 'cancelled',
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
        });
        reject(new RuntimeTimeoutError(timeoutMs));
      }, timeoutMs);

      const abortCleanup = this.attachAbortHandler(
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
        streamState: 'waitingStart',
        nextSeq: 0,
        onStart: handlers.onStart,
        onChunk: handlers.onChunk,
        onEnd: handlers.onEnd,
        ...(handlers.closeFromPendingTerminal
          ? { closeFromPendingTerminal: handlers.closeFromPendingTerminal }
          : {}),
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
          const providerError = new ProviderUnavailableError(error.message);
          this.finishPending(dispatchHeader.requestId, pending, {
            source: 'callback_error',
            kind: 'failed',
            error: providerError
          });
          reject(providerError);
        }
      );
    });
  }

  close(): void {
    for (const [requestId, pending] of Array.from(this.pending.entries())) {
      this.finishPending(requestId, pending, {
        source: 'router_shutdown',
        kind: 'cancelled',
        reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.routerShutdown)
      });
      pending.reject(new ProviderUnavailableError('Runtime registry is closing'));
    }
    this.options.registry.refreshAllRuntimeStates();
  }

  countInFlight(runtime: RuntimeDispatchRuntimeIdentity): number {
    let count = 0;
    for (const pending of this.pending.values()) {
      if (this.pendingBelongsToRuntime(pending, runtime)) {
        count += 1;
      }
    }
    return count;
  }

  pendingLifecycleCounters(): RuntimeDispatcherPendingCounters {
    const counters: RuntimeDispatcherPendingCounters = {
      pendingUnary: 0,
      pendingStream: 0
    };
    for (const pending of this.pending.values()) {
      if (pending.kind === 'stream') {
        counters.pendingStream += 1;
      } else {
        counters.pendingUnary += 1;
      }
    }
    return counters;
  }

  handleRuntimeCancel(ws: WebSocket, envelope: RequestCancelEnvelope): void {
    if (typeof envelope.requestId !== 'string') {
      throw new Error('invalid request.cancel envelope');
    }

    const pending = this.pending.get(envelope.requestId);
    if (!pending) {
      return;
    }
    if (!this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }

    this.finishPending(envelope.requestId, pending, {
      source: 'runtime_request_cancel',
      kind: 'cancelled',
      reason: envelope.reason
    });
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
    if (pending.kind === 'stream') {
      if (pending.streamState !== 'streaming') {
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
      pending.streamState = 'terminal';
      try {
        pending.onEnd(response, (terminal) => {
          this.finishStreamPending(requestId, pending, terminal, response);
        });
      } catch (error) {
        this.rejectPendingWithError(ws, requestId, error);
      }
      return;
    }
    this.finishPending(requestId, pending, {
      source: 'runtime_response_end',
      kind: 'completed'
    });
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
    if (pending.kind === 'unaryFrame') {
      this.finishPending(envelope.requestId, pending, {
        source: 'runtime_response_error',
        kind: 'failed',
        error: envelope.error
      });
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
    this.finishPending(envelope.requestId, pending, {
      source: 'runtime_response_error',
      kind: 'failed',
      error: envelope.error
    });
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
    if (pending.kind !== 'stream') {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'UnexpectedStart',
        message: 'response.start is only valid for serverStream dispatch'
      });
      return;
    }
    if (pending.streamState !== 'waitingStart') {
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
      pending.onStart(response, (terminal) => {
        this.finishStreamPending(requestId, pending, terminal);
      });
    } catch (error) {
      this.rejectPendingWithError(ws, requestId, error);
      return;
    }
    clearTimeout(pending.timeout);
    pending.streamState = 'streaming';
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
    if (pending.kind !== 'stream') {
      this.rejectPendingRuntimeError(ws, requestId, {
        code: 'UnexpectedChunk',
        message: 'response.chunk is only valid for serverStream dispatch'
      });
      return;
    }
    if (pending.streamState !== 'streaming') {
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
      pending.onChunk(response, (terminal) => {
        this.finishStreamPending(requestId, pending, terminal);
      });
    } catch (error) {
      this.rejectPendingWithError(ws, requestId, error);
      return;
    }
    pending.nextSeq += 1;
  }

  handleRuntimeDisconnect(ws: WebSocket): void {
    for (const [requestId, pending] of Array.from(this.pending.entries())) {
      if (pending.ws === ws) {
        this.finishPending(requestId, pending, {
          source: 'runtime_disconnect',
          kind: 'cancelled',
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.runtimeDisconnect)
        });
        pending.reject(new ProviderUnavailableError('Runtime disconnected before responding'));
        continue;
      }
    }
  }

  private rejectPendingRuntimeError(
    ws: WebSocket,
    requestId: string,
    error: { code: string; message: string; details?: unknown }
  ): void {
    this.rejectPendingWithError(ws, requestId, new RuntimeResponseError(error), 'protocol_error');
  }

  private rejectPendingWithError(
    ws: WebSocket,
    requestId: string,
    error: unknown,
    source: 'callback_error' | 'protocol_error' = 'callback_error'
  ): void {
    const pending = this.pending.get(requestId);
    if (!pending || !this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }
    this.finishPending(requestId, pending, {
      source,
      kind: 'failed',
      error
    });
    pending.reject(error);
  }

  private detachPending(requestId: string, pending: RuntimeInvocation | undefined): void {
    if (!pending) {
      return;
    }
    clearTimeout(pending.timeout);
    pending.abortCleanup?.();
    if (pending.kind === 'stream') {
      pending.streamState = 'terminal';
    }
    this.pending.delete(requestId);
  }

  private finishPending(
    requestId: string,
    pending: RuntimeInvocation | undefined,
    terminal: PendingTerminal
  ): void {
    if (!pending || !this.pending.has(requestId)) {
      return;
    }
    this.detachPending(requestId, pending);
    pending.kind === 'stream' && pending.closeFromPendingTerminal?.(terminal);
    this.maybeSendPendingCancel(requestId, pending, terminal);
    this.options.registry.refreshRuntimeStatesForRequest(pending);
  }

  private finishStreamPending(
    requestId: string,
    pending: RuntimeStreamInvocation,
    terminal: PendingTerminal,
    response?: RuntimeBinaryDispatchResponse
  ): void {
    this.finishPending(requestId, pending, terminal);
    if (terminal.kind === 'completed') {
      if (response) {
        pending.resolve(response);
        return;
      }
      pending.reject(
        new RuntimeResponseError({
          code: 'StreamProtocolError',
          message: 'stream completed without response.end'
        })
      );
      return;
    }
    if (terminal.kind === 'failed') {
      pending.reject(terminal.error);
      return;
    }
    pending.reject(
      new ProviderUnavailableError(`Runtime stream request cancelled: ${terminal.source}`)
    );
  }

  private maybeSendPendingCancel(
    requestId: string,
    pending: RuntimeInvocation,
    terminal: PendingTerminal
  ): void {
    const reason = this.cancelReasonForTerminal(terminal);
    if (reason === undefined) {
      return;
    }
    this.sendCancel(pending.ws, {
      type: 'request.cancel',
      requestId,
      reason
    });
  }

  private cancelReasonForTerminal(terminal: PendingTerminal): RequestCancelReason | undefined {
    switch (terminal.source) {
      case 'runtime_response_end':
      case 'runtime_response_error':
      case 'runtime_request_cancel':
      case 'runtime_disconnect':
        return undefined;
      case 'timeout':
        return requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout);
      case 'caller_abort':
        return terminal.kind === 'cancelled' && terminal.reason
          ? terminal.reason
          : requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.callerAbort);
      case 'client_disconnect':
        return requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.clientDisconnect);
      case 'backpressure':
        return requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.backpressure);
      case 'protocol_error':
      case 'callback_error':
        return requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.protocolError);
      case 'router_shutdown':
        return requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.routerShutdown);
    }
  }

  private attachAbortHandler(
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
      const reason =
        options.cancelReason ??
        requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.callerAbort);
      this.finishPending(requestId, pending, {
        source:
          reason === requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.clientDisconnect)
            ? 'client_disconnect'
            : 'caller_abort',
        kind: 'cancelled',
        reason:
          options.cancelReason ??
          requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.callerAbort)
      });
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

  private isPendingRuntimeSocket(ws: WebSocket, pending: RuntimeInvocation): boolean {
    return pending.ws === ws;
  }

  private pendingBelongsToRuntime(
    pending: RuntimeInvocation,
    runtime: RuntimeDispatchRuntimeIdentity
  ): boolean {
    if (pending.runtimeId !== undefined) {
      return pending.runtimeId === runtime.runtimeId;
    }
    return pending.ws === runtime.ws;
  }
}

function dispatchHeaderForConnection(
  header: RequestStartFrameHeader,
  connection: RuntimeDispatchConnection
): RequestStartFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeUnaryDispatchFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeUnaryDispatchFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeDispatchFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeDispatchFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeDispatchFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeDispatchFrameHeader {
  if (isRuntimeAssemblyRequestDispatchHeader(header)) {
    return header;
  }
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
