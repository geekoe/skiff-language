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
  type SpawnSubmitRequestFrameHeader,
  type SpawnSubmitResponseFrameHeader,
  type RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader
} from '../protocol/envelope.js';
import type {
  RuntimeAssemblyRequestStartFrameHeader,
  RuntimeAssemblyRequestStartFrameWireHeader,
  RuntimeAssemblySpawnRequestStartFrameHeader,
  RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
  RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader
} from '../protocol/runtimeAssemblyRequest.js';
import {
  validateRuntimeAssemblyRequestStartFrame
} from '../protocol/runtimeAssemblyRequestFrame.js';
import {
  validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrame
} from '../protocol/runtimeAssemblyRequestResponseFrame.js';
import {
  type ValidatedResponseErrorFrame,
  validateRuntimeAssemblyRequestStartFrameWireHeader,
  validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader
} from '../protocol/runtimeProtocol.js';
import type {
  WebSocketGenerationLifecycleTuple
} from '../protocol/webSocketGenerationLifecycle.js';
import {
  isRequestCancelReason,
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
import type { RuntimeControlSource } from './actorSpawnRuntimeControl.js';
import {
  FixedServiceResponseError,
  GatewayError,
  ProviderUnavailableError,
  RuntimeResponseError,
  RuntimeTimeoutError,
  ServiceProtocolBoundaryError
} from './errors.js';

const DEFAULT_DERIVED_SPAWN_TIMEOUT_MS = 120_000;

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

interface RuntimeInvocationBase<
  TRequest extends
    | RuntimeDispatchFrameHeader
    | RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
    | RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader
    | RuntimeAssemblySpawnRequestStartFrameHeader
> {
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

interface RuntimeAssemblyWebSocketJsonRpcInvocation
  extends RuntimeInvocationBase<RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader> {
  kind: 'websocketJsonRpc';
  executionToken: object;
  resolve(response: RuntimeAssemblyWebSocketJsonRpcDispatchResponse): void;
}

interface RuntimeDerivedSpawnInvocation
  extends RuntimeInvocationBase<RuntimeAssemblySpawnRequestStartFrameHeader> {
  kind: 'derivedSpawn';
  admissionSettled: boolean;
  resolveAdmission(): void;
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
  | RuntimeAssemblyWebSocketJsonRpcInvocation
  | RuntimeDerivedSpawnInvocation
  | RuntimeStreamInvocation;

export interface RuntimeBinaryDispatchResponse {
  header: ResponseEndFrameHeader;
  payloadBytes: Uint8Array;
}

export interface RuntimeAssemblyWebSocketJsonRpcDispatchRequest {
  header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader;
  payloadBytes: Uint8Array;
}

export interface RuntimeAssemblyWebSocketJsonRpcDispatchResponse {
  header: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader;
  payloadBytes: Uint8Array;
}

export interface RuntimeAssemblyWebSocketJsonRpcDispatchOptions {
  signal: AbortSignal;
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
  active: boolean;
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
  maxConcurrency: number;
  registry: RuntimeDispatchRegistry;
}

export type RuntimeSpawnSubmitResult =
  | { header: SpawnSubmitResponseFrameHeader }
  | {
      header: {
        schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
        type: 'spawn.submit.error';
        rpcId: string;
        error: {
          code: string;
          message: string;
          status: number;
        };
      };
    };

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
  private readonly connectionReceiptsBySocket = new WeakMap<
    WebSocket,
    Set<RuntimeDispatchConnectionReceiptRecord>
  >();

  constructor(private readonly options: RuntimeDispatcherOptions) {
    if (
      !Number.isSafeInteger(options.maxConcurrency) ||
      options.maxConcurrency <= 0
    ) {
      throw new Error('runtime maxConcurrency must be a positive safe integer');
    }
    this.options.registry.setInFlightCounter({
      countInFlight: (runtime) => this.countInFlight(runtime),
      hasCapacity: (runtime) => this.hasConnectionCapacity(runtime.ws)
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

  async handleSpawnSubmit(
    ws: WebSocket,
    submit: SpawnSubmitRequestFrameHeader,
    payloadBytes: Uint8Array,
    source: RuntimeControlSource | undefined
  ): Promise<RuntimeSpawnSubmitResult> {
    try {
      const parent = this.requireSpawnParent(ws, submit, source);
      if (source === undefined) {
        throw new ServiceProtocolBoundaryError(
          'spawn submit source disappeared after parent authentication'
        );
      }
      const requestId = `spawn-request-${randomUUID()}`;
      const spawnId = submit.spawnId ?? `spawn-${randomUUID()}`;
      const timeoutMs = source.timeoutMs ?? DEFAULT_DERIVED_SPAWN_TIMEOUT_MS;
      const request = derivedSpawnRequest(parent.request, submit.target, requestId, timeoutMs);
      await this.dispatchDerivedSpawn(
        ws,
        request,
        payloadBytes,
        timeoutMs,
        source.runtimeId
      );
      return {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'spawn.submit.response',
          rpcId: submit.rpcId,
          spawnId,
          requestId,
          status: 'submitted'
        }
      };
    } catch (error) {
      return {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'spawn.submit.error',
          rpcId: submit.rpcId,
          error: spawnSubmitError(error)
        }
      };
    }
  }

  private requireSpawnParent(
    ws: WebSocket,
    submit: SpawnSubmitRequestFrameHeader,
    source: RuntimeControlSource | undefined
  ): RuntimeInvocation & {
    request: RuntimeAssemblyRequestStartFrameWireHeader;
  } {
    const pending = this.pending.get(submit.callerRequestId);
    if (
      pending === undefined ||
      pending.ws !== ws ||
      !isRuntimeAssemblyRequestDispatchHeader(pending.request)
    ) {
      throw new ServiceProtocolBoundaryError(
        'spawn callerRequestId must identify an active request on the same runtime connection'
      );
    }
    if (source === undefined) {
      throw new ServiceProtocolBoundaryError(
        'spawn submit requires an exact active or pinned-draining assembly registration'
      );
    }
    const deployment = pending.request.routing.deployment;
    const activation = submit.activationIdentity;
    if (
      submit.runtimeId !== source.runtimeId ||
      submit.runtimeId !== activation.runtimeReplicaId ||
      pending.runtimeId !== undefined && pending.runtimeId !== source.runtimeId ||
      submit.serviceId !== deployment.serviceId ||
      submit.serviceVersion !== deployment.contractVersion ||
      (submit.buildId !== undefined && submit.buildId !== source.buildId) ||
      submit.serviceProtocolIdentity !== source.serviceProtocolIdentity ||
      source.serviceId !== deployment.serviceId ||
      activation.assemblyIdentity !== pending.request.routing.assemblyIdentity ||
      activation.generation !== pending.request.routing.assemblyGeneration ||
      activation.deploymentRevision !== deployment.deploymentRevision
    ) {
      throw new ServiceProtocolBoundaryError(
        'spawn submit owner facts must exactly match its authenticated parent request'
      );
    }
    return pending as RuntimeInvocation & {
      request: RuntimeAssemblyRequestStartFrameWireHeader;
    };
  }

  private dispatchDerivedSpawn(
    ws: WebSocket,
    request: RuntimeAssemblySpawnRequestStartFrameHeader,
    payloadBytes: Uint8Array,
    timeoutMs: number,
    runtimeId: string | undefined
  ): Promise<void> {
    if (ws.readyState !== WebSocket.OPEN) {
      return Promise.reject(new ProviderUnavailableError('Pinned runtime disconnected'));
    }
    if (this.pending.has(request.requestId)) {
      return Promise.reject(
        new ServiceProtocolBoundaryError(
          `runtime dispatch requestId ${request.requestId} is already pending`
        )
      );
    }
    this.assertConnectionAdmission(ws);
    return new Promise<void>((resolve, rejectPromise) => {
      const settle = {
        done: false,
        resolve: () => {
          if (settle.done) return;
          settle.done = true;
          resolve();
        },
        reject: (error: unknown) => {
          if (settle.done) return;
          settle.done = true;
          rejectPromise(error);
        }
      };
      const timeout = setTimeout(() => {
        const pending = this.pending.get(request.requestId);
        this.finishPending(request.requestId, pending, {
          source: 'timeout',
          kind: 'cancelled',
          reason: requestCancelReasonForSituation(REQUEST_CANCEL_SITUATION.timeout)
        });
        settle.reject(new RuntimeTimeoutError(timeoutMs));
      }, timeoutMs);
      const pending: RuntimeDerivedSpawnInvocation = {
        kind: 'derivedSpawn',
        request,
        ...(runtimeId === undefined ? {} : { runtimeId }),
        timeout,
        ws,
        admissionSettled: false,
        resolveAdmission: () => {
          pending.admissionSettled = true;
          settle.resolve();
        },
        reject: (error) => {
          pending.admissionSettled = true;
          settle.reject(error);
        }
      };
      this.pending.set(request.requestId, pending);
      try {
        this.options.frameSender.sendFrame(ws, request, payloadBytes, (error) => {
          if (error != null) {
            const current = this.pending.get(request.requestId);
            const providerError = new ProviderUnavailableError(error.message);
            this.finishPending(request.requestId, current, {
              source: 'callback_error',
              kind: 'failed',
              error: providerError
            });
            pending.reject(providerError);
            return;
          }
          pending.resolveAdmission();
        });
      } catch (error) {
        const current = this.pending.get(request.requestId);
        const providerError = new ProviderUnavailableError(
          runtimeProtocolValidationMessage(error)
        );
        this.finishPending(request.requestId, current, {
          source: 'callback_error',
          kind: 'failed',
          error: providerError
        });
        pending.reject(providerError);
      }
    });
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
      !('websocketConnect' in validation.envelope) ||
      validation.envelope.routing.ingress.protocol !== 'webSocket' ||
      validation.envelope.routing.ingress.method !== null
    ) {
      return Promise.reject(
        new ServiceProtocolBoundaryError(
          validation.ok
            ? 'RuntimeAssembly WebSocket connect dispatch requires method-null webSocket ingress'
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

  dispatchAssemblyWebSocketJsonRpc(
    request: RuntimeAssemblyWebSocketJsonRpcDispatchRequest,
    timeoutMs: number,
    connectionReceipt: RuntimeDispatchConnectionReceipt,
    options: RuntimeAssemblyWebSocketJsonRpcDispatchOptions
  ): Promise<RuntimeAssemblyWebSocketJsonRpcDispatchResponse> {
    let header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader;
    try {
      const validated = validateRuntimeAssemblyRequestStartFrame(
        request.header,
        request.payloadBytes
      );
      if (
        !('websocketJsonRpc' in validated) ||
        validated.routing.ingress.protocol !== 'webSocket' ||
        validated.routing.ingress.method === null
      ) {
        return Promise.reject(
          new ServiceProtocolBoundaryError(
            'RuntimeAssembly WebSocket JSON-RPC dispatch requires method-bearing webSocket ingress'
          )
        );
      }
      header =
        validated as RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader;
    } catch (error) {
      return Promise.reject(
        new ServiceProtocolBoundaryError(runtimeProtocolValidationMessage(error))
      );
    }

    const receiptRecord = this.connectionByReceipt.get(connectionReceipt);
    if (receiptRecord === undefined) {
      return Promise.reject(
        new ServiceProtocolBoundaryError(
          'runtime dispatch connection receipt was not issued by this dispatcher'
        )
      );
    }
    if (!receiptRecord.active) {
      return Promise.reject(
        new ProviderUnavailableError(
          'Pinned runtime connection receipt has expired'
        )
      );
    }
    const connection = receiptRecord.connection;
    if (connection.ws.readyState !== WebSocket.OPEN) {
      this.expireConnectionReceipts(connection.ws);
      return Promise.reject(
        new ProviderUnavailableError('Pinned runtime disconnected')
      );
    }
    if (this.pending.has(header.requestId)) {
      return Promise.reject(
        new ServiceProtocolBoundaryError(
          `runtime dispatch requestId ${header.requestId} is already pending`
        )
      );
    }
    this.assertConnectionAdmission(connection.ws);

    const executionToken = {};
    return new Promise<RuntimeAssemblyWebSocketJsonRpcDispatchResponse>(
      (resolve, reject) => {
        const timeout = setTimeout(() => {
          const pending = this.pending.get(header.requestId);
          if (
            pending?.kind !== 'websocketJsonRpc' ||
            pending.executionToken !== executionToken
          ) {
            return;
          }
          this.finishPending(header.requestId, pending, {
            source: 'timeout',
            kind: 'cancelled',
            reason: requestCancelReasonForSituation(
              REQUEST_CANCEL_SITUATION.timeout
            )
          });
          pending.reject(new RuntimeTimeoutError(timeoutMs));
        }, timeoutMs);

        const pending: RuntimeAssemblyWebSocketJsonRpcInvocation = {
          kind: 'websocketJsonRpc',
          executionToken,
          ...(connection.runtimeId === undefined
            ? {}
            : { runtimeId: connection.runtimeId }),
          request: header,
          timeout,
          ws: connection.ws,
          resolve,
          reject
        };
        const abortCleanup = this.attachWebSocketJsonRpcAbortHandler(
          header.requestId,
          executionToken,
          options.signal
        );
        if (abortCleanup !== undefined) {
          pending.abortCleanup = abortCleanup;
        }
        this.pending.set(header.requestId, pending);

        try {
          this.options.frameSender.sendFrame(
            connection.ws,
            header,
            request.payloadBytes,
            (error) => {
              if (!error) {
                return;
              }
              const current = this.pending.get(header.requestId);
              if (
                current?.kind !== 'websocketJsonRpc' ||
                current.executionToken !== executionToken
              ) {
                return;
              }
              const providerError = new ProviderUnavailableError(error.message);
              this.finishPending(header.requestId, current, {
                source: 'callback_error',
                kind: 'failed',
                error: providerError
              });
              current.reject(providerError);
            }
          );
        } catch (error) {
          const current = this.pending.get(header.requestId);
          if (
            current?.kind !== 'websocketJsonRpc' ||
            current.executionToken !== executionToken
          ) {
            return;
          }
          const providerError = new ProviderUnavailableError(
            runtimeProtocolValidationMessage(error)
          );
          this.finishPending(header.requestId, current, {
            source: 'callback_error',
            kind: 'failed',
            error: providerError
          });
          current.reject(providerError);
        }
      }
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
    try {
      this.assertRequestIdAvailable(dispatchHeader.requestId);
      this.assertConnectionAdmission(connection.ws);
    } catch (error) {
      return Promise.reject(error);
    }
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
    const record = this.connectionByReceipt.get(receipt);
    return record?.active === true && record.connection.ws === sender;
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
    try {
      this.assertRequestIdAvailable(dispatchHeader.requestId);
      this.assertConnectionAdmission(connection.ws);
    } catch (error) {
      return Promise.reject(error);
    }

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
    try {
      this.assertRequestIdAvailable(dispatchHeader.requestId);
      this.assertConnectionAdmission(connection.ws);
    } catch (error) {
      return Promise.reject(error);
    }

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
    return this.countInFlightForConnection(runtime.ws);
  }

  private countInFlightForConnection(ws: WebSocket): number {
    let count = 0;
    for (const pending of this.pending.values()) {
      if (pending.ws === ws) {
        count += 1;
      }
    }
    return count;
  }

  private hasConnectionCapacity(ws: WebSocket): boolean {
    return (
      ws.readyState === WebSocket.OPEN &&
      this.countInFlightForConnection(ws) < this.options.maxConcurrency
    );
  }

  private assertConnectionAdmission(ws: WebSocket): void {
    if (!this.hasConnectionCapacity(ws)) {
      throw new ProviderUnavailableError(
        `Runtime connection has reached maxConcurrency ${this.options.maxConcurrency}`
      );
    }
  }

  private assertRequestIdAvailable(requestId: string): void {
    if (this.pending.has(requestId)) {
      throw new ServiceProtocolBoundaryError(
        `runtime dispatch requestId ${requestId} is already pending`
      );
    }
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
    const capturedConnection = Object.freeze({
      ...(connection.runtimeId === undefined
        ? {}
        : { runtimeId: connection.runtimeId }),
      ...(connection.dispatchBuildId === undefined
        ? {}
        : { dispatchBuildId: connection.dispatchBuildId }),
      ws: connection.ws
    }) satisfies RuntimeDispatchConnection;
    const receipt = Object.freeze({
      ...(capturedConnection.runtimeId !== undefined
        ? { runtimeId: capturedConnection.runtimeId }
        : {}),
      [runtimeDispatchConnectionReceiptBrand]: true as const
    }) as RuntimeDispatchConnectionReceipt;
    const record = {
      connection: capturedConnection,
      active: true
    } satisfies RuntimeDispatchConnectionReceiptRecord;
    this.connectionByReceipt.set(receipt, record);
    const records =
      this.connectionReceiptsBySocket.get(capturedConnection.ws) ?? new Set();
    records.add(record);
    this.connectionReceiptsBySocket.set(capturedConnection.ws, records);
    return receipt;
  }

  private expireConnectionReceipts(ws: WebSocket): void {
    const records = this.connectionReceiptsBySocket.get(ws);
    if (records === undefined) {
      return;
    }
    for (const record of records) {
      record.active = false;
    }
    records.clear();
    this.connectionReceiptsBySocket.delete(ws);
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
    if (pending.kind === 'derivedSpawn') {
      if (
        response.header.payloadPresent ||
        response.payloadBytes.byteLength !== 0 ||
        response.header.httpResponse !== undefined ||
        response.header.websocketConnect !== undefined ||
        response.header.websocketJsonRpc !== undefined
      ) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'SpawnResponseProtocolError',
          message: 'derived spawn response.end must be empty'
        });
        return;
      }
      this.finishPending(requestId, pending, {
        source: 'runtime_response_end',
        kind: 'completed'
      });
      pending.resolveAdmission();
      return;
    }
    if (pending.kind === 'websocketJsonRpc') {
      let header: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader;
      try {
        header = validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
          response.header,
          response.payloadBytes
        );
      } catch (error) {
        this.rejectPendingRuntimeError(ws, requestId, {
          code: 'WebSocketJsonRpcResponseProtocolError',
          message: runtimeProtocolValidationMessage(error)
        });
        return;
      }
      this.finishPending(requestId, pending, {
        source: 'runtime_response_end',
        kind: 'completed'
      });
      pending.resolve({
        header,
        payloadBytes: response.payloadBytes
      });
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
    if (pending.kind === 'derivedSpawn') {
      this.finishPending(response.header.requestId, pending, {
        source: 'runtime_response_error',
        kind: 'failed',
        error: response.header
      });
      pending.resolveAdmission();
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
    this.expireConnectionReceipts(ws);
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
    if (pending.kind === 'websocketJsonRpc') {
      this.options.registry.refreshAllRuntimeStates();
    } else if (pending.kind === 'unary') {
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

  private attachWebSocketJsonRpcAbortHandler(
    requestId: string,
    executionToken: object,
    signal: AbortSignal
  ): (() => void) | undefined {
    const abort = () => {
      const pending = this.pending.get(requestId);
      if (
        pending?.kind !== 'websocketJsonRpc' ||
        pending.executionToken !== executionToken
      ) {
        return;
      }
      const reason =
        typeof signal.reason === 'string' &&
        isRequestCancelReason(signal.reason)
          ? signal.reason
          : requestCancelReasonForSituation(
              REQUEST_CANCEL_SITUATION.callerAbort
            );
      this.finishPending(requestId, pending, {
        source: 'caller_abort',
        kind: 'cancelled',
        reason
      });
      pending.reject(
        new ProviderUnavailableError(
          'Runtime request was cancelled before completion'
        )
      );
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
    try {
      this.options.frameSender.sendFrame(ws, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'request.cancel',
        requestId: cancel.requestId,
        reason: cancel.reason
      });
    } catch {
      // Cancellation is advisory after the pending entry is detached.
    }
  }

  private isPendingRuntimeSocket(ws: WebSocket, pending: RuntimeInvocation): boolean {
    return pending.ws === ws;
  }

}

function validateCanonicalAssemblyUnaryResponse(
  request: RuntimeUnaryDispatchWireHeader,
  response: RuntimeBinaryDispatchResponse
): string | undefined {
  if (!hasRuntimeAssemblyRouting(request)) {
    return undefined;
  }
  if ('invocation' in request) {
    return response.header.payloadPresent || response.payloadBytes.byteLength !== 0
      ? 'derived spawn response.end must be empty'
      : undefined;
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
  header:
    | RuntimeDispatchFrameHeader
    | RuntimeAssemblyRequestStartFrameWireHeader
): header is RuntimeAssemblyRequestStartFrameWireHeader {
  return header.type === 'request.start' && 'routing' in header;
}

function isWebSocketConnectRequest(
  header: RuntimeUnaryDispatchWireHeader
): header is RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
  return (
    hasRuntimeAssemblyRouting(header) &&
    'ingress' in header.routing &&
    header.routing.ingress.protocol === 'webSocket' &&
    header.routing.ingress.method === null &&
    'websocketConnect' in header
  );
}

function runtimeProtocolValidationMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : 'RuntimeAssembly WebSocket JSON-RPC protocol validation failed';
}

function derivedSpawnRequest(
  parent: RuntimeAssemblyRequestStartFrameWireHeader,
  target: string,
  requestId: string,
  timeoutMs: number
): RuntimeAssemblySpawnRequestStartFrameHeader {
  const testCaseCapability =
    'testCaseCapability' in parent ? parent.testCaseCapability : undefined;
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId,
    mode: 'unary',
    caller: { kind: 'service' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: parent.routing.assemblyIdentity,
      assemblyGeneration: parent.routing.assemblyGeneration,
      deployment: { ...parent.routing.deployment }
    },
    invocation: {
      kind: 'spawn',
      targetKind: 'function',
      target
    },
    deadline: {
      timeoutMs,
      expiresAt: new Date(Date.now() + timeoutMs).toISOString()
    },
    trace: {
      traceId: parent.trace.traceId,
      spanId: randomUUID(),
      parentSpanId: parent.trace.spanId,
      ...(parent.trace.sampled === undefined
        ? {}
        : { sampled: parent.trace.sampled })
    },
    testEffectsEnabled: testCaseCapability !== undefined,
    ...(testCaseCapability === undefined ? {} : { testCaseCapability })
  };
}

function spawnSubmitError(error: unknown): {
  code: string;
  message: string;
  status: number;
} {
  if (error instanceof GatewayError) {
    return {
      code: 'SpawnSubmitRejected',
      message: error.message,
      status: error.statusCode
    };
  }
  return {
    code: 'SpawnSubmitRejected',
    message: error instanceof Error ? error.message : String(error),
    status: 500
  };
}
