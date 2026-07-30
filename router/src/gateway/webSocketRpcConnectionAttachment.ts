import { randomUUID } from 'node:crypto';
import { TextDecoder } from 'node:util';

import type WebSocket from 'ws';

import { GatewayError } from '../router/errors.js';
import type {
  RouterActiveAssemblySnapshot,
  RuntimeAssemblyDeploymentRef,
  RuntimeAssemblyIngressBinding
} from '../router/runtimeAssemblySnapshot.js';
import type {
  RuntimeAssemblyWebSocketMethodBinding,
  RuntimeAssemblyWebSocketRpcProfile
} from '../router/runtimeAssemblyWebSocketSnapshot.js';
import type {
  RuntimeDispatchConnectionReceipt
} from '../router/runtimeDispatcher.js';
import type {
  CapturedWebSocketRpcRuntimeOwner,
  CapturedWebSocketRpcConnection,
  WebSocketRpcBridge,
  WebSocketRpcBridgeConnectionHandle
} from './webSocketRpcBridge.js';

const PEER_TEXT_DECODER = new TextDecoder('utf-8', { fatal: true });

export interface WebSocketRpcConnectionAttachment {
  readonly bridgeHandle: WebSocketRpcBridgeConnectionHandle;
  finalize(): Promise<void>;
}

export interface WebSocketRpcIngressCapture {
  readonly serviceId: string;
  readonly deployment: RuntimeAssemblyDeploymentRef;
  readonly assemblyIdentity: string;
  readonly assemblyGeneration: number;
  readonly websocketEntryId: string;
  readonly path: string;
  readonly profile: RuntimeAssemblyWebSocketRpcProfile;
  readonly methodTable: ReadonlyMap<
    string,
    RuntimeAssemblyWebSocketMethodBinding
  >;
  readonly requiresRuntimePin: boolean;
}

export function captureWebSocketRpcIngress(input: {
  readonly snapshot: RouterActiveAssemblySnapshot;
  readonly binding: RuntimeAssemblyIngressBinding;
}): WebSocketRpcIngressCapture {
  const { binding, snapshot } = input;
  if (
    binding.selector.protocol !== 'webSocket' ||
    binding.adapterKind !== 'websocketConnect' ||
    binding.operationMode !== 'unary' ||
    binding.websocketEntryId === undefined ||
    binding.websocketRpcProfiles?.length !== 1 ||
    binding.websocketRpcProfiles[0] !== 'jsonrpc-2.0-text'
  ) {
    throw new GatewayError(
      500,
      'InvalidAssemblyIngress',
      'committed WebSocket ingress is not an exact current RPC binding'
    );
  }
  const methodTable =
    binding.websocketMethods?.capture() ??
    new Map<string, RuntimeAssemblyWebSocketMethodBinding>();
  return {
    serviceId: binding.deployment.serviceId,
    deployment: Object.freeze({ ...binding.deployment }),
    assemblyIdentity: snapshot.assembly.assemblyIdentity,
    assemblyGeneration: snapshot.generation,
    websocketEntryId: binding.websocketEntryId,
    path: binding.selector.path,
    profile: binding.websocketRpcProfiles[0],
    methodTable,
    requiresRuntimePin:
      binding.handler !== undefined || methodTable.size > 0
  };
}

export function attachWebSocketRpcConnection(input: {
  readonly socket: WebSocket;
  readonly bridge: Pick<
    WebSocketRpcBridge,
    'attach' | 'captureProfileAdapter'
  >;
  readonly capture: WebSocketRpcIngressCapture;
  readonly connectionId: string;
  readonly writer: CapturedWebSocketRpcConnection['writer'];
  readonly businessIdentity?: string;
  readonly routerRequestTimeoutMs: number;
  readonly runtimeReceipt?: RuntimeDispatchConnectionReceipt;
  readonly runtimeReplicaId?: string;
  readonly runtimeOwner: (
    source: Parameters<CapturedWebSocketRpcConnection['runtimeOwner']>[0]
  ) => CapturedWebSocketRpcRuntimeOwner | undefined;
  readonly releaseGeneration: () => void | Promise<void>;
}): WebSocketRpcConnectionAttachment {
  const capture = input.capture;
  const bridgeHandle = input.bridge.attach({
    socketGeneration: randomUUID(),
    connectionId: input.connectionId,
    serviceId: capture.serviceId,
    deployment: capture.deployment,
    assemblyIdentity: capture.assemblyIdentity,
    assemblyGeneration: capture.assemblyGeneration,
    websocketEntryId: capture.websocketEntryId,
    path: capture.path,
    profile: capture.profile,
    profileAdapter: input.bridge.captureProfileAdapter(capture.profile),
    methodTable: capture.methodTable,
    ...(input.businessIdentity === undefined
      ? {}
      : { businessIdentity: input.businessIdentity }),
    writer: input.writer,
    routerRequestTimeoutMs: input.routerRequestTimeoutMs,
    ...(input.runtimeReceipt === undefined
      ? {}
      : { runtimeReceipt: input.runtimeReceipt }),
    ...(input.runtimeReplicaId === undefined
      ? {}
      : { runtimeReplicaId: input.runtimeReplicaId }),
    runtimeOwner: input.runtimeOwner,
    releaseGeneration: input.releaseGeneration
  });
  let finalizePromise: Promise<void> | undefined;
  const onMessage = (data: WebSocket.RawData, isBinary: boolean): void => {
    if (isBinary) {
      bridgeHandle.handlePeerBinary();
      return;
    }
    try {
      bridgeHandle.handlePeerText(decodeTextFrame(data));
    } catch {
      bridgeHandle.handlePeerBinary();
    }
  };
  try {
    input.socket.on('message', onMessage);
  } catch (error) {
    void bridgeHandle.finalize().catch(() => undefined);
    throw error;
  }

  return Object.freeze({
    bridgeHandle,
    finalize: () => {
      if (finalizePromise !== undefined) {
        return finalizePromise;
      }
      input.socket.off('message', onMessage);
      finalizePromise = bridgeHandle.finalize();
      return finalizePromise;
    }
  });
}

function decodeTextFrame(data: WebSocket.RawData): string {
  if (Array.isArray(data)) {
    return PEER_TEXT_DECODER.decode(Buffer.concat(data));
  }
  if (data instanceof ArrayBuffer) {
    return PEER_TEXT_DECODER.decode(new Uint8Array(data));
  }
  return PEER_TEXT_DECODER.decode(data);
}
