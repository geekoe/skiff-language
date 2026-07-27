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
import type {
  RuntimeAssemblyRequestStartFrameHeader,
  RuntimeAssemblyRequestStartFrameWireHeader,
  RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
} from '../protocol/runtimeAssemblyRequest.js';
import {
  type ValidatedResponseErrorFrame,
  validateRuntimeAssemblyRequestStartFrameWireHeader,
  validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader
} from '../protocol/runtimeProtocol.js';
import type {
  WebSocketGenerationLifecycleTuple
} from '../protocol/webSocketGenerationLifecycle.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation
} from '../protocol/cancelReason.js';
import type {
  RuntimeDispatchConnection,
  RuntimeDispatchFrameHeader,
  RuntimeDispatchRuntimeIdentity,
  RuntimeRegistry,
  RuntimeUnaryDispatchFrameHeader
} from './runtimeRegistry.js';
import { isRuntimeAssemblyRequestDispatchHeader } from './runtimeRegistry.js';
import {
  FixedServiceResponseError,
  GatewayError,
  ProviderUnavailableError,
  RuntimeResponseError,
  RuntimeTimeoutError,
  ServiceProtocolBoundaryError
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
    header: RouterToRuntimeFrameHeader | RuntimeAssemblyRequestStartFrameWireHeader,
    payloadBytes?: Uint8Array,
    callback?: RuntimeFrameSendCallback
  ): void;
}

type RuntimeUnaryDispatchWireHeader =
  | RuntimeUnaryDispatchFrameHeader
  | RuntimeAssemblyWebSocketConnectRequestStartFrameHeader;

interface RuntimeUnaryDispatchWireInput {
  header: RuntimeUnaryDispatchWireHeader;
  payloadBytes: Uint8Array;
}

interface RuntimeInvocationBase<TRequest extends RuntimeDispatchFrameHeader | RuntimeAssemblyWebSocketConnectRequestStartFrameHeader> {
  request: TRequest;
  runtimeId?: string;
  ws: WebSocket;
  timeout: NodeJS.Timeout;
  reject(error: unknown): void;
  abortCleanup?: () => void;
}

export interface RuntimeUnaryInvocation
  extends RuntimeInvocationBase<RuntimeUnaryDispatchWireHeader> {
  kind: 'unary';
  request: RuntimeUnaryDispatchWireHeader;
  connectionReceipt: RuntimeDispatchConnectionReceipt;
  resolve(response: RuntimeBinaryDispatchResponseWithReceipt): void;
}

export interface RuntimeUnaryFrameInvocation
  extends RuntimeInvocationBase<RuntimeDispatchFrameHeader> {
  kind: 'unaryFrame';
  resolve(response: RuntimeBinaryDispatchResult): void;
}

export interface RuntimeStreamInvocation
  extends RuntimeInvocationBase<RuntimeUnaryDispatchFrameHeader> {
  kind: 'stream';
  request: RuntimeUnaryDispatchFrameHeader;
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

const runtimeDispatchConnectionReceiptBrand: unique symbol = Symbol(
  'RuntimeDispatchConnectionReceipt'
);

export interface RuntimeDispatchConnectionReceipt {
  readonly runtimeId?: string;
  readonly [runtimeDispatchConnectionReceiptBrand]: true;
}

export interface RuntimeBinaryDispatchResponseWithReceipt
  extends RuntimeBinaryDispatchResponse {
  connectionReceipt: RuntimeDispatchConnectionReceipt;
}

interface RuntimeDispatchConnectionReceiptRecord {
  connection: RuntimeDispatchConnection;
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
  /** Reuse a dispatcher-issued connection without exposing or accepting a raw socket. */
  connectionReceipt?: RuntimeDispatchConnectionReceipt;
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
> & {
  validateDispatchRequest?(request: RuntimeDispatchFrameHeader): GatewayError | undefined;
  pickAssemblyTestDispatchConnection?(
    request: RuntimeDispatchFrameHeader
  ): RuntimeDispatchConnection | GatewayError | null | undefined;
};

export interface RuntimeDispatcherPendingCounters {
  pendingUnary: number;
  pendingStream: number;
}

export class RuntimeDispatcher {
  private readonly pending = new Map<string, RuntimeInvocation>();
  private readonly connectionByReceipt = new WeakMap<
    RuntimeDispatchConnectionReceipt,
    RuntimeDispatchConnectionReceiptRecord
  >();

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
  ): Promise<RuntimeBinaryDispatchResponseWithReceipt> {
    const connection = this.resolveDispatchConnection(request.header, options);
    return this.dispatchBinaryWithConnection(
      request,
      timeoutMs,
      options,
      connection
    );
  }

  dispatchAssemblyTestBinary(
    request: RuntimeBinaryDispatchInput<RuntimeAssemblyRequestStartFrameHeader>,
    timeoutMs: number
  ): Promise<RuntimeBinaryDispatchResponseWithReceipt> {
    const pickConnection =
      this.options.registry.pickAssemblyTestDispatchConnection;
    if (pickConnection === undefined) {
      return Promise.reject(
        new ServiceProtocolBoundaryError(
          'runtime dispatcher does not provide the test RuntimeAssembly registry seam'
        )
      );
    }
    const connection = pickConnection.call(
      this.options.registry,
      request.header
    );
    return this.dispatchBinaryWithConnection(
      request,
      timeoutMs,
      {},
      connection
    );
  }

  dispatchAssemblyWebSocketConnect(
    request: {
      header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader;
      payloadBytes: Uint8Array;
    },
    timeoutMs: number,
    connection: RuntimeDispatchConnection,
    options: RuntimeBinaryDispatchOptions = {}
  ): Promise<RuntimeBinaryDispatchResponseWithReceipt> {
    const validation = validateRuntimeAssemblyRequestStartFrameWireHeader(
      request.header
    );
    if (
      !validation.ok ||
      validation.envelope.routing.ingress.protocol !== 'webSocket'
    ) {
      return Promise.reject(
        new ServiceProtocolBoundaryError(
          validation.ok
            ? 'RuntimeAssembly WebSocket connect dispatch requires webSocket ingress'
            : validation.error
        )
      );
    }
    if (request.payloadBytes.byteLength !== 0) {
      return Promise.reject(
        new ServiceProtocolBoundaryError(
          'RuntimeAssembly WebSocket connect dispatch payload must be empty'
        )
      );
    }
    return this.dispatchBinaryWithConnection(
      request,
      timeoutMs,
      options,
      connection
    );
  }

  private dispatchBinaryWithConnection(
    request: RuntimeUnaryDispatchWireInput,
    timeoutMs: number,
    options: RuntimeBinaryDispatchOptions,
    connection: RuntimeDispatchConnection | GatewayError | null | undefined
  ): Promise<RuntimeBinaryDispatchResponseWithReceipt> {
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
    const connectionReceipt =
      options.connectionReceipt ?? this.issueConnectionReceipt(connection);

    return new Promise<RuntimeBinaryDispatchResponseWithReceipt>((resolve, reject) => {
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
        connectionReceipt,
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

  isRuntimeConnectionReceiptSender(
    receipt: RuntimeDispatchConnectionReceipt,
    sender: WebSocket
  ): boolean {
    return this.connectionByReceipt.get(receipt)?.connection.ws === sender;
  }

  isPendingWebSocketAcquireSender(
    sender: WebSocket,
    tuple: WebSocketGenerationLifecycleTuple
  ): boolean {
    for (const pending of this.pending.values()) {
      if (
        pending.kind !== 'unary' ||
        pending.ws !== sender ||
        !isWebSocketConnectRequest(pending.request)
      ) {
        continue;
      }
      const request = pending.request;
      if (
        request.routing.assemblyIdentity === tuple.assemblyIdentity &&
        request.routing.assemblyGeneration === tuple.assemblyGeneration &&
        request.websocketConnect.connectionId === tuple.connectionId &&
        request.websocketConnect.websocketEntryId === tuple.websocketEntryId
      ) {
        return true;
      }
    }
    return false;
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
    request: RuntimeBinaryDispatchInput<RuntimeUnaryDispatchFrameHeader>,
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

  private resolveDispatchConnection(
    request: RuntimeUnaryDispatchFrameHeader,
    options: RuntimeBinaryDispatchOptions
  ): RuntimeDispatchConnection | GatewayError | null | undefined {
    if (options.connection !== undefined && options.connectionReceipt !== undefined) {
      return new ServiceProtocolBoundaryError(
        'runtime dispatch must use either a raw connection or a dispatcher receipt, not both'
      );
    }
    if (options.connectionReceipt !== undefined) {
      const record = this.connectionByReceipt.get(options.connectionReceipt);
      if (record === undefined) {
        return new ServiceProtocolBoundaryError(
          'runtime dispatch connection receipt was not issued by this dispatcher'
        );
      }
      return new ServiceProtocolBoundaryError(
        'connection receipt dispatch is unavailable until RuntimeAssembly WebSocket business routing is frozen'
      );
    }
    if (options.connection !== undefined) {
      const requestError = this.options.registry.validateDispatchRequest?.(request);
      return requestError ?? options.connection;
    }
    return this.options.registry.pickDispatchConnection(request);
  }

  private issueConnectionReceipt(
    connection: RuntimeDispatchConnection
  ): RuntimeDispatchConnectionReceipt {
    const receipt = Object.freeze({
      ...(connection.runtimeId !== undefined ? { runtimeId: connection.runtimeId } : {}),
      [runtimeDispatchConnectionReceiptBrand]: true as const
    }) as RuntimeDispatchConnectionReceipt;
    this.connectionByReceipt.set(receipt, { connection });
    return receipt;
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
    if (pending.kind === 'unary') {
      const responseError = validateCanonicalAssemblyUnaryResponse(
        pending.request,
        response
      );
      if (responseError !== undefined) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'HttpResponseProtocolError',
          message: responseError
        });
        return;
      }
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
      if (
        response.header.httpResponse !== undefined ||
        response.header.websocketConnect !== undefined
      ) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'StreamProtocolError',
          message: 'streaming response.end must not include response metadata'
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
    if (pending.kind === 'unary') {
      pending.resolve({
        ...response,
        connectionReceipt: pending.connectionReceipt
      });
    } else {
      pending.resolve(response);
    }
  }

  rejectRequest(
    ws: WebSocket,
    response: ValidatedResponseErrorFrame
  ): void {
    const pending = this.pending.get(response.header.requestId);
    if (!pending) {
      return;
    }
    if (!this.isPendingRuntimeSocket(ws, pending)) {
      return;
    }
    if (pending.kind === 'unaryFrame') {
      this.finishPending(response.header.requestId, pending, {
        source: 'runtime_response_error',
        kind: 'failed',
        error:
          'serviceError' in response
            ? new FixedServiceResponseError(response.serviceError)
            : response.header.error
      });
      pending.resolve({
        header: response.header,
        payloadBytes: response.payloadBytes
      });
      return;
    }
    const error =
      'serviceError' in response
        ? new FixedServiceResponseError(response.serviceError)
        : new RuntimeResponseError(response.header.error);
    this.finishPending(response.header.requestId, pending, {
      source: 'runtime_response_error',
      kind: 'failed',
      error
    });
    pending.reject(error);
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
    if (pending.kind === 'unary') {
      const request = pending.request;
      if (isWebSocketConnectRequest(request)) {
        this.options.registry.refreshAllRuntimeStates();
      } else {
        this.options.registry.refreshRuntimeStatesForRequest({
          request,
          ws: pending.ws,
          ...(pending.runtimeId === undefined
            ? {}
            : { runtimeId: pending.runtimeId })
        });
      }
    } else {
      this.options.registry.refreshRuntimeStatesForRequest(pending);
    }
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

function validateCanonicalAssemblyUnaryResponse(
  request: RuntimeUnaryDispatchWireHeader,
  response: RuntimeBinaryDispatchResponse
): string | undefined {
  if (!hasRuntimeAssemblyRouting(request)) {
    return undefined;
  }
  if (request.routing.ingress.protocol === 'webSocket') {
    if (response.payloadBytes.byteLength !== 0) {
      return 'RuntimeAssembly WebSocket connect response payload must be empty';
    }
    const validation =
      validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader(
        response.header
      );
    return validation.ok ? undefined : validation.error;
  }
  if (response.header.websocketConnect !== undefined) {
    return 'RuntimeAssembly HTTP unary response must not include WebSocket metadata';
  }
  if (response.header.payloadPresent !== (response.payloadBytes.byteLength > 0)) {
    return 'RuntimeAssembly HTTP unary payloadPresent must match response body bytes';
  }
  return undefined;
}

function dispatchHeaderForConnection(
  header: RequestStartFrameHeader,
  connection: RuntimeDispatchConnection
): RequestStartFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeAssemblyWebSocketConnectRequestStartFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeUnaryDispatchFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeUnaryDispatchFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeDispatchFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeDispatchFrameHeader;
function dispatchHeaderForConnection(
  header: RuntimeUnaryDispatchWireHeader,
  connection: RuntimeDispatchConnection
): RuntimeUnaryDispatchWireHeader;
function dispatchHeaderForConnection(
  header: RuntimeDispatchFrameHeader | RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
  connection: RuntimeDispatchConnection
): RuntimeDispatchFrameHeader | RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
  if (hasRuntimeAssemblyRouting(header)) {
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

function hasRuntimeAssemblyRouting(
  header: RuntimeDispatchFrameHeader | RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
): header is RuntimeAssemblyRequestStartFrameWireHeader {
  return header.type === 'request.start' && 'routing' in header;
}

function isWebSocketConnectRequest(
  header: RuntimeUnaryDispatchWireHeader
): header is RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
  return (
    hasRuntimeAssemblyRouting(header) &&
    header.routing.ingress.protocol === 'webSocket'
  );
}
