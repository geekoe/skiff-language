import {
  actorLogicalKey,
  makeActorKey,
  type ActorInvocationLedger,
  type ActorManager,
  type ActorMethodAdmissionRejection,
  type ActorOwnerFence,
} from '../actor/index.js';
import {
  type ActorDeclarationOwnerFrameHeader,
  type ActorMethodErrorFrameHeader,
  type ActorMethodInvokeFrameHeader,
  type ActorMethodFrameHeader,
} from '../protocol/actorMethodProtocol.js';
import { RUNTIME_FRAME_SCHEMA_VERSION } from '../protocol/envelope.js';

export interface ActorMethodCatalog {
  hasMethod(input: {
    declarationOwner: ActorDeclarationOwnerFrameHeader;
    actorAbiIdentity: string;
    actorImplementationIdentity: string;
    methodIdentity: string;
  }): boolean | Promise<boolean>;
}

export interface ActorOwnerTransport {
  dispatchToOwner(input: {
    ownerFence: ActorOwnerFence;
    header: ActorMethodInvokeFrameHeader;
    payloadBytes: Uint8Array;
  }): void | Promise<void>;
}

export type ActorMethodDispatchRejection =
  | 'NotActorMethodInvoke'
  | 'InvalidActorRef'
  | 'NotPresent'
  | 'AbiMismatch'
  | 'UnknownMethod'
  | 'OwnerUnavailable'
  | 'InvocationAlreadyExists'
  | 'DispatchFailed';

export type ActorMethodDispatchResult =
  | {
      ok: true;
      ownerFence: ActorOwnerFence;
      invocation: ActorInvocationLedger;
    }
  | {
      ok: false;
      reason: ActorMethodDispatchRejection | 'IncarnationReplaced' | 'VersionRejected' | 'Upgrading';
      errorFrame?: ActorMethodErrorFrameHeader | undefined;
    };

export class ActorMethodDispatcher {
  constructor(
    private readonly actorManager: ActorManager,
    private readonly catalog: ActorMethodCatalog,
    private readonly transport: ActorOwnerTransport,
    private readonly now: () => Date = () => new Date()
  ) {}

  async dispatch(
    header: ActorMethodFrameHeader,
    payloadBytes: Uint8Array
  ): Promise<ActorMethodDispatchResult> {
    if (header.type !== 'actor.method.invoke') {
      return { ok: false, reason: 'NotActorMethodInvoke' };
    }

    const actorKey = makeActorKey({
      serviceId: header.actorRef.serviceId,
      actorTypeIdentity: header.actorRef.actorTypeIdentity,
      actorIdTypeIdentity: header.actorRef.actorIdTypeIdentity,
      actorIdEncodingVersion: header.actorRef.actorIdEncodingVersion,
      canonicalActorIdKeyBytes: Buffer.from(
        header.actorRef.canonicalActorIdKeyBytesBase64,
        'base64'
      ),
    });
    if (actorKey.actorIdHash !== header.actorRef.actorIdHash) {
      return { ok: false, reason: 'InvalidActorRef' };
    }

    const methodKnown = await this.catalog.hasMethod({
      declarationOwner: header.declarationOwner,
      actorAbiIdentity: header.actorAbiIdentity,
      actorImplementationIdentity: header.actorImplementationIdentity,
      methodIdentity: header.methodIdentity,
    });
    const admitted = await this.actorManager.registryStore().admitActorMethod({
      invocationId: header.invocationId,
      actorKey,
      expectedEpoch: header.actorRef.epoch,
      actorAbiIdentity: header.actorAbiIdentity,
      requestedImplementationIdentity: header.actorImplementationIdentity,
      methodIdentity: header.methodIdentity,
      methodKnown,
      now: this.now(),
    });
    if (!admitted.ok) {
      return admissionRejection(header, admitted.rejection);
    }

    const transitionInput = {
      invocationId: header.invocationId,
      actorKey,
      expectedEpoch: admitted.ownerFence.epoch,
      actorImplementationIdentity: admitted.ownerFence.implementationIdentity,
      ownerRuntimeId: admitted.ownerFence.ownerRuntimeId,
      ownerLeaseId: admitted.ownerFence.ownerLeaseId,
    };
    const dispatched = await this.actorManager.registryStore().transitionActorInvocation({
      ...transitionInput,
      nextState: 'dispatched',
      now: this.now(),
    });
    if (!dispatched.ok) {
      return { ok: false, reason: 'DispatchFailed' };
    }

    try {
      await this.transport.dispatchToOwner({
        ownerFence: admitted.ownerFence,
        header,
        payloadBytes: new Uint8Array(payloadBytes),
      });
    } catch (error) {
      await this.actorManager.registryStore().transitionActorInvocation({
        ...transitionInput,
        nextState: 'failed',
        terminalReason: error instanceof Error ? error.message : String(error),
        now: this.now(),
      });
      return { ok: false, reason: 'DispatchFailed' };
    }

    return {
      ok: true,
      ownerFence: admitted.ownerFence,
      invocation: dispatched.invocation,
    };
  }
}

function admissionRejection(
  header: ActorMethodInvokeFrameHeader,
  rejection: ActorMethodAdmissionRejection
): ActorMethodDispatchResult {
  switch (rejection.reason) {
    case 'IncarnationReplaced':
      return {
        ok: false,
        reason: 'IncarnationReplaced',
        errorFrame: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.error',
          invocationId: header.invocationId,
          error: {
            name: 'actorIncarnationReplacedError',
            actorRef: header.actorRef,
            currentEpoch: rejection.currentEpoch,
          },
        },
      };
    case 'Upgrading':
      return {
        ok: false,
        reason: 'Upgrading',
        errorFrame: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.error',
          invocationId: header.invocationId,
          error: {
            name: 'actorUpgradingError',
            actorRef: header.actorRef,
            retryAfterMs: rejection.retryAfterMs,
          },
        },
      };
    case 'VersionRejected':
      return {
        ok: false,
        reason: 'VersionRejected',
        errorFrame: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'actor.method.error',
          invocationId: header.invocationId,
          error: {
            name: 'actorVersionRejectedError',
            actorRef: header.actorRef,
            requestedImplementationIdentity: header.actorImplementationIdentity,
            acceptedImplementationIdentity: rejection.acceptedImplementationIdentity,
          },
        },
      };
    case 'NotPresent':
    case 'AbiMismatch':
    case 'UnknownMethod':
    case 'OwnerUnavailable':
    case 'InvocationAlreadyExists':
      return { ok: false, reason: rejection.reason };
  }
}

export function sameActorOwnerFence(
  left: ActorOwnerFence,
  right: ActorOwnerFence
): boolean {
  return (
    actorLogicalKey(left.actorKey) === actorLogicalKey(right.actorKey) &&
    left.epoch === right.epoch &&
    left.implementationIdentity === right.implementationIdentity &&
    left.ownerRuntimeId === right.ownerRuntimeId &&
    left.ownerLeaseId === right.ownerLeaseId
  );
}
