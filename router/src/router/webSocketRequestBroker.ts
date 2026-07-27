import type {
  OpaquePayload,
  OpaquePeerId,
  PlatformRpcError,
  ProfileId,
  ProfileResponse,
  WebSocketRpcProfileAdapter
} from '../protocol/jsonRpc20TextProfileContracts.js';
import {
  REQUEST_CANCEL_SITUATION,
  requestCancelReasonForSituation,
  type RequestCancelReason
} from '../protocol/cancelReason.js';
import {
  BrokerTombstoneStore,
  SYSTEM_BROKER_CLOCK,
  validateBrokerOptions
} from './webSocketRequestBrokerState.js';
import type {
  AttachBrokerGenerationOptions,
  BrokerConnectionGeneration,
  BrokerRuntimeRequest,
  BrokerRuntimeResponse,
  BrokerRuntimeSource,
  CapturedPeerWriter,
  InboundDispatchAction,
  InboundDispatchResult,
  InboundExecutionToken,
  WebSocketRequestBrokerClock,
  WebSocketRequestBrokerLimits,
  WebSocketRequestBrokerOptions,
  WebSocketRequestBrokerSnapshot
} from './webSocketRequestBrokerTypes.js';
import {
  encodeInboundTerminalFrame,
  encodeOutboundPeerRequest,
  mapInboundDispatchResultToTerminal,
  mapPeerTerminalToRuntimeResponse,
  materializeOutboundPeerParams,
  tryEncodePeerCancelFrame,
  type InboundTerminal
} from './webSocketRequestBrokerWire.js';

export {
  DEFAULT_WEB_SOCKET_REQUEST_BROKER_LIMITS
} from './webSocketRequestBrokerTypes.js';
export type {
  AttachBrokerGenerationOptions,
  BrokerConnectionGeneration,
  BrokerConnectionResponseOutcome,
  BrokerRuntimeRequest,
  BrokerRuntimeResponse,
  BrokerRuntimeSource,
  CapturedPeerWriter,
  InboundDispatchAction,
  InboundDispatchResult,
  InboundExecutionToken,
  InboundNotificationAction,
  WebSocketRequestBrokerClock,
  WebSocketRequestBrokerLimits,
  WebSocketRequestBrokerOptions,
  WebSocketRequestBrokerSnapshot
} from './webSocketRequestBrokerTypes.js';

interface GenerationState {
  readonly handle: BrokerConnectionGeneration;
  readonly identityKey: string;
  readonly uid: number;
  readonly ownerToken: unknown;
  readonly adapter: WebSocketRpcProfileAdapter;
  readonly inboundTimeoutMs: number;
  readonly writer: CapturedPeerWriter;
  readonly acceptInboundMethod?: (method: string) => boolean;
  readonly idGeneration: {
    readonly randomPrefix: string;
    sequence: bigint;
  };
  open: boolean;
  outboundActive: number;
  inboundActive: number;
}

interface OutboundEntry {
  readonly generation: GenerationState;
  readonly peerId: OpaquePeerId;
  readonly peerKey: string;
  readonly runtimeKey: string;
  readonly source: BrokerRuntimeSource;
  readonly requestId: string;
  timer?: unknown;
}

interface InboundEntry {
  readonly generation: GenerationState;
  readonly peerId: OpaquePeerId;
  readonly peerKey: string;
  readonly controller: AbortController;
  readonly executionToken: InboundExecutionToken;
  timer?: unknown;
}

const MAX_TIMER_DELAY_MS = 2_147_483_647;
const INBOUND_CALLER_CANCEL_REASON = requestCancelReasonForSituation(
  REQUEST_CANCEL_SITUATION.callerAbort
);
const INBOUND_DEADLINE_REASON = requestCancelReasonForSituation(
  REQUEST_CANCEL_SITUATION.deadlineExceeded
);
const INBOUND_PEER_DISCONNECT_REASON = requestCancelReasonForSituation(
  REQUEST_CANCEL_SITUATION.clientDisconnect
);
const INBOUND_PROTOCOL_REASON = requestCancelReasonForSituation(
  REQUEST_CANCEL_SITUATION.protocolError
);

export class WebSocketRequestBroker {
  private readonly adapters = new Map<
    ProfileId,
    WebSocketRpcProfileAdapter
  >();
  private readonly generationByIdentity = new Map<
    string,
    BrokerConnectionGeneration
  >();
  private readonly generations = new Map<
    BrokerConnectionGeneration,
    GenerationState
  >();
  private readonly outboundByPeer = new Map<string, OutboundEntry>();
  private readonly outboundByRuntime = new Map<string, OutboundEntry>();
  private readonly inboundByPeer = new Map<string, InboundEntry>();
  private readonly outboundTombstones: BrokerTombstoneStore;
  private readonly inboundTombstones: BrokerTombstoneStore;
  private readonly runtimeSenderIds = new WeakMap<object, number>();
  private readonly clock: WebSocketRequestBrokerClock;
  private nextGenerationUid = 1;
  private nextExecutionSequence = 1;
  private nextRuntimeSenderId = 1;
  private activeTimerCount = 0;

  constructor(private readonly options: WebSocketRequestBrokerOptions) {
    validateBrokerOptions(options);
    for (const adapter of options.profiles) {
      if (this.adapters.has(adapter.profile)) {
        throw new Error(`duplicate WebSocket RPC profile ${adapter.profile}`);
      }
      this.adapters.set(adapter.profile, adapter);
    }
    if (this.adapters.size === 0) {
      throw new Error('WebSocket request broker requires at least one profile');
    }
    this.clock = options.clock ?? SYSTEM_BROKER_CLOCK;
    this.outboundTombstones = new BrokerTombstoneStore(
      options.outboundTombstoneCapacity,
      options.outboundTombstoneTtlMs,
      () => this.clock.now()
    );
    this.inboundTombstones = new BrokerTombstoneStore(
      options.inboundTombstoneCapacity,
      options.inboundTombstoneTtlMs,
      () => this.clock.now()
    );
  }

  attachGeneration(
    input: AttachBrokerGenerationOptions
  ): BrokerConnectionGeneration {
    if (
      input.connectionId.length === 0 ||
      input.socketGeneration.length === 0 ||
      input.serviceId.length === 0 ||
      input.websocketEntryId.length === 0 ||
      input.outboundIdPrefix.length === 0
    ) {
      throw new Error('broker generation identity fields must be non-empty');
    }
    if (!this.adapters.has(input.profile)) {
      throw new Error(`unsupported WebSocket RPC profile ${input.profile}`);
    }
    const adapter = input.profileAdapter;
    if (adapter.profile !== input.profile) {
      throw new Error(
        'captured WebSocket RPC profile adapter does not match its profile'
      );
    }
    if (
      !Number.isSafeInteger(input.inboundTimeoutMs) ||
      input.inboundTimeoutMs <= 0
    ) {
      throw new Error('inboundTimeoutMs must be a positive safe integer');
    }
    const identityKey = JSON.stringify([
      input.connectionId,
      input.socketGeneration
    ]);
    if (this.generationByIdentity.has(identityKey)) {
      throw new Error('connection generation is already attached');
    }
    const handle = Object.freeze({
      connectionId: input.connectionId,
      socketGeneration: input.socketGeneration,
      serviceId: input.serviceId,
      websocketEntryId: input.websocketEntryId,
      profile: input.profile
    }) satisfies BrokerConnectionGeneration;
    const state: GenerationState = {
      handle,
      identityKey,
      uid: this.nextGenerationUid++,
      ownerToken: input.ownerToken,
      adapter,
      inboundTimeoutMs: input.inboundTimeoutMs,
      writer: input.writer,
      ...(input.acceptInboundMethod === undefined
        ? {}
        : { acceptInboundMethod: input.acceptInboundMethod }),
      idGeneration: {
        randomPrefix: input.outboundIdPrefix,
        sequence: 0n
      },
      open: true,
      outboundActive: 0,
      inboundActive: 0
    };
    this.generations.set(handle, state);
    this.generationByIdentity.set(identityKey, handle);
    return handle;
  }

  handleRuntimeRequest(
    generation: BrokerConnectionGeneration,
    request: BrokerRuntimeRequest
  ): void {
    this.sweepTombstones();
    const state = this.generations.get(generation);
    if (state === undefined || !state.open) {
      this.respond(request.source, {
        requestId: request.requestId,
        outcome: 'connectionUnavailable'
      });
      return;
    }
    if (
      request.serviceId !== state.handle.serviceId ||
      request.websocketEntryId !== state.handle.websocketEntryId ||
      request.ownerToken !== state.ownerToken
    ) {
      this.respond(request.source, {
        requestId: request.requestId,
        outcome: 'connectionUnavailable'
      });
      return;
    }
    if (
      request.requestId.length === 0 ||
      request.method.length === 0 ||
      request.profile !== state.handle.profile
    ) {
      this.respond(request.source, {
        requestId: request.requestId,
        outcome: 'protocolError'
      });
      this.runtimeProtocolViolation(
        request.source,
        'invalid connection.request broker metadata'
      );
      return;
    }
    if (
      request.deadlineAtMs !== undefined &&
      !Number.isFinite(request.deadlineAtMs)
    ) {
      this.respond(request.source, {
        requestId: request.requestId,
        outcome: 'protocolError'
      });
      this.runtimeProtocolViolation(
        request.source,
        'invalid connection.request deadline'
      );
      return;
    }

    const runtimeKey = this.runtimeKey(request.source, request.requestId);
    if (this.outboundByRuntime.has(runtimeKey)) {
      this.respond(request.source, {
        requestId: request.requestId,
        outcome: 'protocolError'
      });
      this.runtimeProtocolViolation(
        request.source,
        'duplicate active connection.request correlation'
      );
      return;
    }
    if (
      this.outboundByPeer.size >= this.options.outboundGlobalCapacity ||
      state.outboundActive >= this.options.outboundPerGenerationCapacity
    ) {
      this.respond(request.source, {
        requestId: request.requestId,
        outcome: 'resourceLimit'
      });
      return;
    }

    let params: OpaquePayload;
    let peerId: OpaquePeerId;
    let frame: string;
    try {
      params = materializeOutboundPeerParams({
        adapter: state.adapter,
        payloadBytes: request.payloadBytes,
        limits: this.options.profileLimits
      });
      peerId = state.adapter.nextOutboundId({
        randomPrefix: state.idGeneration.randomPrefix,
        takeSequence: () => {
          const sequence = state.idGeneration.sequence;
          state.idGeneration.sequence += 1n;
          return sequence;
        }
      });
      frame = encodeOutboundPeerRequest({
        adapter: state.adapter,
        id: peerId,
        method: request.method,
        params
      });
    } catch {
      this.respond(request.source, {
        requestId: request.requestId,
        outcome: 'resourceLimit'
      });
      return;
    }

    const peerKey = this.peerKey(state, peerId);
    if (
      this.outboundByPeer.has(peerKey) ||
      this.outboundTombstones.has(peerKey)
    ) {
      this.respond(request.source, {
        requestId: request.requestId,
        outcome: 'protocolError'
      });
      this.runtimeProtocolViolation(
        request.source,
        'outbound peer id generator reused an id'
      );
      return;
    }
    const entry: OutboundEntry = {
      generation: state,
      peerId,
      peerKey,
      runtimeKey,
      source: request.source,
      requestId: request.requestId
    };
    this.outboundByPeer.set(peerKey, entry);
    this.outboundByRuntime.set(runtimeKey, entry);
    state.outboundActive += 1;

    if (request.deadlineAtMs !== undefined) {
      this.armDeadline(entry, request.deadlineAtMs, () => {
        this.settleOutbound(entry, {
          response: {
            requestId: entry.requestId,
            outcome: 'deadlineExceeded'
          },
          cancelPeer: true
        });
      });
      if (!this.outboundIsActive(entry)) {
        return;
      }
    }

    this.writePeer(state, frame, () => {
      this.settleOutbound(entry, {
        response: {
          requestId: entry.requestId,
          outcome: 'transportUnavailable'
        }
      });
    });
  }

  handleRuntimeCancel(
    source: BrokerRuntimeSource,
    requestId: string
  ): boolean {
    this.sweepTombstones();
    const entry = this.outboundByRuntime.get(
      this.runtimeKey(source, requestId)
    );
    if (entry === undefined) {
      return false;
    }
    this.settleOutbound(entry, { cancelPeer: true });
    return true;
  }

  handleRuntimeDisconnect(source: BrokerRuntimeSource): number {
    this.sweepTombstones();
    const entries = [...this.outboundByPeer.values()].filter(
      (entry) =>
        entry.source.sender === source.sender &&
        entry.source.sessionToken === source.sessionToken
    );
    for (const entry of entries) {
      this.detachOutbound(entry);
    }
    for (const entry of entries) {
      this.bestEffortCancel(entry);
    }
    return entries.length;
  }

  handlePeerText(
    generation: BrokerConnectionGeneration,
    frame: string
  ): void {
    this.sweepTombstones();
    const state = this.generations.get(generation);
    if (state === undefined || !state.open) {
      return;
    }
    const action = state.adapter.classifyText(
      frame,
      this.options.profileLimits
    );
    switch (action.kind) {
      case 'request':
        this.handleInboundRequest(state, action);
        return;
      case 'response':
        this.handleOutboundResponse(state, action.id, action.terminal);
        return;
      case 'cancel':
        this.handleInboundCancel(state, action.id);
        return;
      case 'ignoredNotification':
        try {
          this.options.observeNotification?.({
            profile: state.handle.profile,
            connectionId: state.handle.connectionId,
            socketGeneration: state.handle.socketGeneration,
            method: action.method,
            ...(action.params === undefined ? {} : { params: action.params })
          });
        } catch {
          // Notification observation is diagnostic and has no RPC terminal.
        }
        return;
      case 'platformError':
        if (action.id === null) {
          this.writeTerminalFrame(
            state,
            state.adapter.encodePlatformError(null, action.error)
          );
          return;
        }
        this.handleInboundPredispatchError(
          state,
          action.id,
          action.error
        );
        return;
      case 'close':
        this.closeGeneration(
          state,
          'protocolError',
          INBOUND_PROTOCOL_REASON,
          action.code,
          action.reason
        );
        return;
    }
  }

  handlePeerBinary(generation: BrokerConnectionGeneration): void {
    const state = this.generations.get(generation);
    if (state === undefined || !state.open) {
      return;
    }
    this.closeGeneration(
      state,
      'protocolError',
      INBOUND_PROTOCOL_REASON,
      1003,
      'binary RPC frames are not supported'
    );
  }

  handlePeerDisconnect(generation: BrokerConnectionGeneration): void {
    const state = this.generations.get(generation);
    if (state === undefined || !state.open) {
      return;
    }
    this.closeGeneration(
      state,
      'transportUnavailable',
      INBOUND_PEER_DISCONNECT_REASON
    );
  }

  debugSnapshot(): WebSocketRequestBrokerSnapshot {
    this.sweepTombstones();
    return {
      generationCount: this.generations.size,
      outboundPeerEntries: this.outboundByPeer.size,
      outboundRuntimeEntries: this.outboundByRuntime.size,
      inboundActiveEntries: this.inboundByPeer.size,
      outboundGenerationActive: sumGenerationActive(
        this.generations.values(),
        'outboundActive'
      ),
      inboundGenerationActive: sumGenerationActive(
        this.generations.values(),
        'inboundActive'
      ),
      outboundTombstones: this.outboundTombstones.size,
      inboundTombstones: this.inboundTombstones.size,
      timerCount: this.activeTimerCount,
      terminalLeaseCount:
        this.outboundByPeer.size + this.inboundByPeer.size
    };
  }

  private handleOutboundResponse(
    state: GenerationState,
    peerId: OpaquePeerId,
    terminal: ProfileResponse
  ): void {
    const peerKey = this.peerKey(state, peerId);
    const entry = this.outboundByPeer.get(peerKey);
    if (entry === undefined) {
      if (this.outboundTombstones.has(peerKey)) {
        return;
      }
      this.closeGeneration(
        state,
        'protocolError',
        INBOUND_PROTOCOL_REASON,
        1002,
        'unknown JSON-RPC response id'
      );
      return;
    }

    let response: BrokerRuntimeResponse;
    try {
      response = mapPeerTerminalToRuntimeResponse({
        adapter: state.adapter,
        limits: this.options.profileLimits,
        requestId: entry.requestId,
        terminal
      });
    } catch {
      this.closeGeneration(
        state,
        'protocolError',
        INBOUND_PROTOCOL_REASON,
        1002,
        'invalid JSON-RPC response payload'
      );
      return;
    }
    this.settleOutbound(entry, { response });
  }

  private handleInboundRequest(
    state: GenerationState,
    action: {
      readonly id: OpaquePeerId;
      readonly method: string;
      readonly params: OpaquePayload;
    }
  ): void {
    const peerKey = this.peerKey(state, action.id);
    if (
      this.inboundByPeer.has(peerKey) ||
      this.inboundTombstones.has(peerKey)
    ) {
      this.closeGeneration(
        state,
        'protocolError',
        INBOUND_PROTOCOL_REASON,
        1002,
        'duplicate JSON-RPC request id'
      );
      return;
    }

    let methodAccepted = false;
    try {
      methodAccepted =
        state.acceptInboundMethod?.(action.method) ?? false;
    } catch {
      methodAccepted = false;
    }
    if (!methodAccepted) {
      this.handleInboundPredispatchError(
        state,
        action.id,
        { kind: 'methodNotFound' }
      );
      return;
    }
    if (
      this.inboundByPeer.size >= this.options.inboundGlobalCapacity ||
      state.inboundActive >= this.options.inboundPerGenerationCapacity
    ) {
      this.handleInboundPredispatchError(
        state,
        action.id,
        { kind: 'serverBusy' }
      );
      return;
    }

    const controller = new AbortController();
    const executionToken = Object.freeze({
      connectionId: state.handle.connectionId,
      socketGeneration: state.handle.socketGeneration,
      sequence: this.nextExecutionSequence++
    }) satisfies InboundExecutionToken;
    const entry: InboundEntry = {
      generation: state,
      peerId: action.id,
      peerKey,
      controller,
      executionToken
    };
    this.inboundByPeer.set(peerKey, entry);
    state.inboundActive += 1;
    this.armDeadline(
      entry,
      this.clock.now() + state.inboundTimeoutMs,
      () => {
        this.finishInbound(
          entry,
          { kind: 'timeout' },
          INBOUND_DEADLINE_REASON
        );
      }
    );

    const dispatchAction = Object.freeze({
      profile: state.handle.profile,
      connectionId: state.handle.connectionId,
      socketGeneration: state.handle.socketGeneration,
      peerId: action.id,
      method: action.method,
      params: action.params,
      executionToken,
      signal: controller.signal
    }) satisfies InboundDispatchAction;
    let result: InboundDispatchResult | Promise<InboundDispatchResult>;
    try {
      result = this.options.dispatchInbound(dispatchAction);
    } catch {
      this.finishInbound(entry, { kind: 'internal' });
      return;
    }
    void Promise.resolve(result).then(
      (terminal) => this.completeInbound(entry, terminal),
      () => this.finishInbound(entry, { kind: 'internal' })
    );
  }

  private handleInboundPredispatchError(
    state: GenerationState,
    peerId: OpaquePeerId,
    error: PlatformRpcError
  ): void {
    const peerKey = this.peerKey(state, peerId);
    if (
      this.inboundByPeer.has(peerKey) ||
      this.inboundTombstones.has(peerKey)
    ) {
      this.closeGeneration(
        state,
        'protocolError',
        INBOUND_PROTOCOL_REASON,
        1002,
        'duplicate JSON-RPC request id'
      );
      return;
    }
    this.inboundTombstones.add(peerKey, state.uid);
    this.writeTerminalFrame(
      state,
      state.adapter.encodePlatformError(peerId, error)
    );
  }

  private handleInboundCancel(
    state: GenerationState,
    peerId: OpaquePeerId
  ): void {
    const entry = this.inboundByPeer.get(this.peerKey(state, peerId));
    if (entry === undefined) {
      return;
    }
    this.finishInbound(
      entry,
      { kind: 'cancelled' },
      INBOUND_CALLER_CANCEL_REASON
    );
  }

  private completeInbound(
    entry: InboundEntry,
    result: InboundDispatchResult
  ): void {
    const plan = mapInboundDispatchResultToTerminal(result);
    if (plan === undefined) {
      return;
    }
    this.finishInbound(
      entry,
      plan.terminal,
      plan.abort ? INBOUND_DEADLINE_REASON : undefined
    );
  }

  private finishInbound(
    entry: InboundEntry,
    terminal: InboundTerminal,
    abortReason?: RequestCancelReason
  ): void {
    if (!this.inboundIsActive(entry)) {
      return;
    }

    const frame = encodeInboundTerminalFrame({
      adapter: entry.generation.adapter,
      id: entry.peerId,
      terminal
    });
    this.detachInbound(entry);
    if (abortReason !== undefined) {
      entry.controller.abort(abortReason);
    }
    this.writeTerminalFrame(entry.generation, frame);
  }

  private settleOutbound(
    entry: OutboundEntry,
    terminal: {
      readonly response?: BrokerRuntimeResponse;
      readonly cancelPeer?: boolean;
    }
  ): void {
    if (!this.outboundIsActive(entry)) {
      return;
    }
    this.detachOutbound(entry);
    if (terminal.cancelPeer === true) {
      this.bestEffortCancel(entry);
    }
    if (terminal.response !== undefined) {
      this.respond(entry.source, terminal.response);
    }
  }

  private detachOutbound(entry: OutboundEntry): void {
    if (!this.outboundIsActive(entry)) {
      return;
    }
    this.outboundByPeer.delete(entry.peerKey);
    this.outboundByRuntime.delete(entry.runtimeKey);
    entry.generation.outboundActive -= 1;
    this.clearEntryTimer(entry);
    this.outboundTombstones.add(entry.peerKey, entry.generation.uid);
  }

  private detachInbound(entry: InboundEntry): void {
    if (!this.inboundIsActive(entry)) {
      return;
    }
    this.inboundByPeer.delete(entry.peerKey);
    entry.generation.inboundActive -= 1;
    this.clearEntryTimer(entry);
    this.inboundTombstones.add(entry.peerKey, entry.generation.uid);
  }

  private closeGeneration(
    state: GenerationState,
    outboundOutcome: 'transportUnavailable' | 'protocolError',
    abortReason: RequestCancelReason,
    closeCode?: number,
    closeReason?: string
  ): void {
    if (!state.open) {
      return;
    }
    state.open = false;
    const outboundEntries = [...this.outboundByPeer.values()].filter(
      (entry) => entry.generation === state
    );
    const inboundEntries = [...this.inboundByPeer.values()].filter(
      (entry) => entry.generation === state
    );
    for (const entry of outboundEntries) {
      this.detachOutbound(entry);
    }
    for (const entry of inboundEntries) {
      this.detachInbound(entry);
    }
    this.generations.delete(state.handle);
    this.generationByIdentity.delete(state.identityKey);
    this.outboundTombstones.removeGeneration(state.uid);
    this.inboundTombstones.removeGeneration(state.uid);

    for (const entry of inboundEntries) {
      entry.controller.abort(abortReason);
    }
    for (const entry of outboundEntries) {
      this.respond(entry.source, {
        requestId: entry.requestId,
        outcome: outboundOutcome
      });
    }
    if (closeCode !== undefined && closeReason !== undefined) {
      try {
        state.writer.close(closeCode, closeReason);
      } catch {
        // State is already detached; close failure cannot reopen it.
      }
    }
  }

  private bestEffortCancel(entry: OutboundEntry): void {
    const frame = tryEncodePeerCancelFrame({
      adapter: entry.generation.adapter,
      id: entry.peerId
    });
    if (frame === undefined) {
      return;
    }
    this.writePeer(entry.generation, frame, () => undefined);
  }

  private writeTerminalFrame(state: GenerationState, frame: string): void {
    this.writePeer(state, frame, () => {
      this.closeGeneration(
        state,
        'transportUnavailable',
        'gateway_disconnect'
      );
    });
  }

  private writePeer(
    state: GenerationState,
    frame: string,
    onFailure: () => void
  ): void {
    if (!state.open) {
      onFailure();
      return;
    }
    let result: void | Promise<void>;
    try {
      result = state.writer.writeText(frame);
    } catch {
      onFailure();
      return;
    }
    if (result !== undefined) {
      void Promise.resolve(result).catch(() => onFailure());
    }
  }

  private respond(
    source: BrokerRuntimeSource,
    response: BrokerRuntimeResponse
  ): void {
    try {
      const result = source.respond(response);
      if (result !== undefined) {
        void Promise.resolve(result).catch(() => undefined);
      }
    } catch {
      // Broker state was detached before the captured runtime write.
    }
  }

  private runtimeProtocolViolation(
    source: BrokerRuntimeSource,
    reason: string
  ): void {
    try {
      this.options.onRuntimeProtocolViolation?.(source, reason);
    } catch {
      // Runtime isolation remains the endpoint owner's responsibility.
    }
  }

  private runtimeKey(
    source: BrokerRuntimeSource,
    requestId: string
  ): string {
    let senderId = this.runtimeSenderIds.get(source.sender);
    if (senderId === undefined) {
      senderId = this.nextRuntimeSenderId++;
      this.runtimeSenderIds.set(source.sender, senderId);
    }
    return JSON.stringify([senderId, source.sessionToken, requestId]);
  }

  private peerKey(state: GenerationState, id: OpaquePeerId): string {
    return JSON.stringify([
      state.uid,
      state.handle.profile,
      state.adapter.peerIdKey(id)
    ]);
  }

  private outboundIsActive(entry: OutboundEntry): boolean {
    return (
      this.outboundByPeer.get(entry.peerKey) === entry &&
      this.outboundByRuntime.get(entry.runtimeKey) === entry
    );
  }

  private inboundIsActive(entry: InboundEntry): boolean {
    return this.inboundByPeer.get(entry.peerKey) === entry;
  }

  private armDeadline(
    entry: OutboundEntry | InboundEntry,
    deadlineAtMs: number,
    onDeadline: () => void
  ): void {
    const schedule = (): void => {
      const remaining = deadlineAtMs - this.clock.now();
      if (remaining <= 0) {
        onDeadline();
        return;
      }
      const delay = Math.min(remaining, MAX_TIMER_DELAY_MS);
      entry.timer = this.clock.setTimeout(() => {
        if (entry.timer === undefined) {
          return;
        }
        entry.timer = undefined;
        this.activeTimerCount -= 1;
        if (
          ('runtimeKey' in entry && !this.outboundIsActive(entry)) ||
          (!('runtimeKey' in entry) && !this.inboundIsActive(entry))
        ) {
          return;
        }
        schedule();
      }, delay);
      this.activeTimerCount += 1;
    };
    schedule();
  }

  private clearEntryTimer(entry: OutboundEntry | InboundEntry): void {
    if (entry.timer === undefined) {
      return;
    }
    this.clock.clearTimeout(entry.timer);
    entry.timer = undefined;
    this.activeTimerCount -= 1;
  }

  private sweepTombstones(): void {
    this.outboundTombstones.sweep();
    this.inboundTombstones.sweep();
  }
}

function sumGenerationActive(
  generations: Iterable<GenerationState>,
  field: 'outboundActive' | 'inboundActive'
): number {
  let total = 0;
  for (const generation of generations) {
    total += generation[field];
  }
  return total;
}
