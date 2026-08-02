import {
  actorLogicalKey,
  makeActorKey,
  type ActorInvocationLedger,
  type ActorManager,
  type ActorMethodAdmissionRejection,
  type ActorOwnerFence,
  type ActorUpgradeFence,
  type ActorUpgradeTransition,
} from '../actor/index.js';
import {
  type ActorDeclarationOwnerFrameHeader,
  type ActorMethodErrorFrameHeader,
  type ActorMethodInvokeFrameHeader,
  type ActorMethodFrameHeader,
} from '../protocol/actorMethodProtocol.js';
import type { ActorOwnerRouteAuthority } from '../protocol/actorOwnerProtocol.js';
import { RUNTIME_FRAME_SCHEMA_VERSION } from '../protocol/envelope.js';
import type WebSocket from 'ws';

export interface ActorMethodCatalog {
  hasMethod(input: {
    declarationOwner: ActorDeclarationOwnerFrameHeader;
    actorAbiIdentity: string;
    actorImplementationIdentity: string;
    methodIdentity: string;
  }): boolean | Promise<boolean>;
}

export interface ActorOwnerDispatch {
  ownerConnection: WebSocket;
  ownerFence: ActorOwnerFence;
}

export interface ActorOwnerConnectionBinding {
  unbind(): void;
}

export interface ActorOwnerTransport {
  dispatchToOwner(input: {
    ownerFence: ActorOwnerFence;
    header: ActorMethodInvokeFrameHeader;
    payloadBytes: Uint8Array;
    requiredOwnerConnection?: WebSocket;
    authority?: ActorOwnerRouteAuthority;
  }): void | ActorOwnerDispatch | Promise<void | ActorOwnerDispatch>;
  activateInitial?(input: {
    header: ActorMethodInvokeFrameHeader;
    requiredOwnerRuntimeId?: string;
    requiredOwnerConnection?: WebSocket;
    authority?: ActorOwnerRouteAuthority;
  }):
    | {
        ownerRuntimeId: string;
        ownerLeaseId: string;
        ownerLeaseExpiresAt: Date;
        ownerConnection: WebSocket;
      }
    | Promise<{
        ownerRuntimeId: string;
        ownerLeaseId: string;
        ownerLeaseExpiresAt: Date;
        ownerConnection: WebSocket;
      }>;
  markOwnerUpgrading?(input: {
    fence: ActorUpgradeFence;
    header: ActorMethodInvokeFrameHeader;
    authority?: ActorOwnerRouteAuthority;
  }): void | Promise<void>;
  discardOldInstance?(input: {
    fence: ActorUpgradeFence;
    header: ActorMethodInvokeFrameHeader;
    authority?: ActorOwnerRouteAuthority;
  }): void | Promise<void>;
  activateTarget?(input: {
    transition: ActorUpgradeTransition;
    header: ActorMethodInvokeFrameHeader;
    authority?: ActorOwnerRouteAuthority;
  }):
    | {
        ownerRuntimeId: string;
        ownerLeaseId: string;
        ownerLeaseExpiresAt: Date;
        ownerConnection: WebSocket;
      }
    | Promise<{
        ownerRuntimeId: string;
        ownerLeaseId: string;
        ownerLeaseExpiresAt: Date;
        ownerConnection: WebSocket;
      }>;
  pendingInitialActivation?(input: {
    actorKey: ReturnType<typeof makeActorKey>;
  }): Promise<boolean> | undefined;
  ownerConnectionMatches?(input: {
    ownerFence: ActorOwnerFence;
    requiredOwnerConnection: WebSocket;
  }): boolean;
  ownerConnectionAvailable?(input: {
    ownerRuntimeId: string;
    requiredOwnerConnection: WebSocket;
  }): boolean;
  bindOwnerConnection?(input: {
    ownerFence: ActorOwnerFence;
    requiredOwnerConnection: WebSocket;
  }): ActorOwnerConnectionBinding | undefined;
}

export type ActorMethodDispatchRejection =
  | 'NotActorMethodInvoke'
  | 'InvalidActorRef'
  | 'NotPresent'
  | 'AbiMismatch'
  | 'UnknownMethod'
  | 'OwnerUnavailable'
  | 'RequiredOwnerMismatch'
  | 'InvocationAlreadyExists'
  | 'DispatchFailed';

export type ActorMethodDispatchResult =
  | {
      ok: true;
      ownerFence: ActorOwnerFence;
      invocation: ActorInvocationLedger;
      ownerConnection?: WebSocket;
    }
  | {
      ok: false;
      reason: ActorMethodDispatchRejection | 'IncarnationReplaced' | 'VersionRejected' | 'Upgrading';
      errorFrame?: ActorMethodErrorFrameHeader | undefined;
    };

type ActorMethodDispatchContext = {
  requiredOwnerRuntimeId?: string;
  requiredOwnerConnection?: WebSocket;
  freshOwnerFence?: ActorOwnerFence;
  authority?: ActorOwnerRouteAuthority;
};

type InitialActorActivation = {
  fence: ActorOwnerFence;
  ownerConnectionBinding?: ActorOwnerConnectionBinding;
  requiredOwnerRuntimeId?: string;
  requiredOwnerConnection?: WebSocket;
};

export class ActorMethodDispatcher {
  private readonly upgrades = new Map<string, Promise<boolean>>();
  private readonly initialActivations = new Map<
    string,
    Promise<InitialActorActivation>
  >();

  constructor(
    private readonly actorManager: ActorManager,
    private readonly catalog: ActorMethodCatalog,
    private readonly transport: ActorOwnerTransport,
    private readonly now: () => Date = () => new Date()
  ) {}

  async dispatch(
    header: ActorMethodFrameHeader,
    payloadBytes: Uint8Array,
    context: ActorMethodDispatchContext = {}
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

    if (this.hasRequiredOwner(context)) {
      const current = await this.actorManager.registryStore().find(actorKey);
      if (!this.requiredOwnerMatches(context)) {
        return { ok: false, reason: 'RequiredOwnerMismatch' };
      }
      if (
        current !== undefined &&
        (current.lifecycleState === 'upgrading' ||
          current.actorImplementationIdentity !==
            header.actorImplementationIdentity)
      ) {
        return { ok: false, reason: 'RequiredOwnerMismatch' };
      }
    }

    const methodKnown = await this.catalog.hasMethod({
      declarationOwner: header.declarationOwner,
      actorAbiIdentity: header.actorAbiIdentity,
      actorImplementationIdentity: header.actorImplementationIdentity,
      methodIdentity: header.methodIdentity,
    });
    if (!this.requiredOwnerMatches(context)) {
      return { ok: false, reason: 'RequiredOwnerMismatch' };
    }
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
    if (!this.requiredOwnerMatches(context)) {
      if (admitted.ok) {
        await this.failAdmittedInvocation(
          header,
          admitted.ownerFence,
          'test capability origin Runtime connection changed during Actor admission'
        );
      }
      return { ok: false, reason: 'RequiredOwnerMismatch' };
    }
    if (!admitted.ok) {
      if (
        this.hasRequiredOwner(context) &&
        admitted.rejection.reason === 'Upgrading'
      ) {
        return { ok: false, reason: 'RequiredOwnerMismatch' };
      }
      if (
        admitted.rejection.reason === 'OwnerUnavailable' &&
        this.transport.activateInitial !== undefined
      ) {
        const pendingInitial = this.transport.pendingInitialActivation?.({
          actorKey,
        });
        if (pendingInitial !== undefined) {
          const completed = await waitUntilDeadline(
            pendingInitial,
            new Date(header.deadline.expiresAt)
          );
          if (!this.requiredOwnerMatches(context)) {
            return { ok: false, reason: 'RequiredOwnerMismatch' };
          }
          if (completed) {
            const current = await this.actorManager.registryStore().find(actorKey);
            if (!this.requiredOwnerMatches(context)) {
              return { ok: false, reason: 'RequiredOwnerMismatch' };
            }
            if (current !== undefined && current.status === 'present') {
              return this.dispatch(
                {
                  ...header,
                  actorRef: { ...header.actorRef, epoch: current.epoch },
                },
                payloadBytes,
                context
              );
            }
          }
        }
        let activation: InitialActorActivation;
        try {
          activation = await this.ensureInitialOwner(actorKey, header, context);
        } catch (error) {
          if (error instanceof RequiredOwnerConnectionChangedError) {
            return { ok: false, reason: 'RequiredOwnerMismatch' };
          }
          throw error;
        }
        if (
          this.hasRequiredOwner(context) &&
          (!this.activationBelongsToContext(activation, context) ||
            !this.requiredOwnerBoundMatches(context, activation.fence))
        ) {
          if (this.activationBelongsToContext(activation, context)) {
            await this.disconnectInitialOwner(
              activation.fence,
              'test capability origin Runtime connection changed during initial Actor activation',
              activation.ownerConnectionBinding
            );
          }
          return { ok: false, reason: 'RequiredOwnerMismatch' };
        }
        const current = await this.actorManager.registryStore().find(actorKey);
        if (
          this.hasRequiredOwner(context) &&
          !this.requiredOwnerBoundMatches(context, activation.fence)
        ) {
          if (this.activationBelongsToContext(activation, context)) {
            await this.disconnectInitialOwner(
              activation.fence,
              'test capability origin Runtime connection changed after initial Actor activation',
              activation.ownerConnectionBinding
            );
          }
          return { ok: false, reason: 'RequiredOwnerMismatch' };
        }
        if (current === undefined || current.status !== 'present') {
          return { ok: false, reason: 'OwnerUnavailable' };
        }
        const result = await this.dispatch(
          {
            ...header,
            actorRef: { ...header.actorRef, epoch: current.epoch },
          },
          payloadBytes,
          { ...context, freshOwnerFence: activation.fence }
        );
        if (
          !result.ok &&
          this.hasRequiredOwner(context) &&
          !this.requiredOwnerMatches(context, activation.fence) &&
          this.activationBelongsToContext(activation, context)
        ) {
          await this.disconnectInitialOwner(
            activation.fence,
            'test capability origin Runtime connection changed during initial Actor dispatch',
            activation.ownerConnectionBinding
          );
        }
        return result;
      }
      if (
        admitted.rejection.reason === 'Upgrading' &&
        header.actorImplementationIdentity ===
          (await this.actorManager.registryStore().find(actorKey))
            ?.targetImplementationIdentity
      ) {
        const completed = await this.advanceUpgrade(
          actorKey,
          new Date(header.deadline.expiresAt),
          header,
          context.authority
        );
        if (completed) {
          const current = await this.actorManager.registryStore().find(actorKey);
          if (current !== undefined) {
            return this.dispatch(
              {
                ...header,
                actorRef: { ...header.actorRef, epoch: current.epoch },
              },
              payloadBytes,
              context
            );
          }
        }
      }
      return admissionRejection(header, admitted.rejection);
    }

    if (
      this.hasRequiredOwner(context) &&
      (admitted.ownerFence.ownerRuntimeId !== context.requiredOwnerRuntimeId ||
        !this.requiredOwnerMatches(context, admitted.ownerFence))
    ) {
      await this.failAdmittedInvocation(
        header,
        admitted.ownerFence,
        'test capability Actor spawn owner differs from its origin Runtime'
      );
      return { ok: false, reason: 'RequiredOwnerMismatch' };
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
    if (!this.requiredOwnerMatches(context, admitted.ownerFence)) {
      await this.failAdmittedInvocation(
        header,
        admitted.ownerFence,
        'test capability origin Runtime connection changed before Actor dispatch'
      );
      return { ok: false, reason: 'RequiredOwnerMismatch' };
    }

    let ownerDispatch: void | ActorOwnerDispatch;
    try {
      ownerDispatch = await this.transport.dispatchToOwner({
        ownerFence: admitted.ownerFence,
        header,
        payloadBytes: new Uint8Array(payloadBytes),
        ...(context.authority === undefined
          ? {}
          : { authority: context.authority }),
        ...(context.requiredOwnerConnection === undefined
          ? {}
          : { requiredOwnerConnection: context.requiredOwnerConnection }),
      });
    } catch (error) {
      await this.actorManager.registryStore().transitionActorInvocation({
        ...transitionInput,
        nextState: 'failed',
        terminalReason: error instanceof Error ? error.message : String(error),
        now: this.now(),
      });
      let currentFence = admitted.ownerFence;
      const current = await this.actorManager.registryStore().find(actorKey);
      if (
        current !== undefined &&
        current.status === 'present' &&
        current.ownerRuntimeId === admitted.ownerFence.ownerRuntimeId &&
        current.ownerLeaseId === admitted.ownerFence.ownerLeaseId &&
        current.ownerLeaseExpiresAt !== undefined &&
        current.epoch === admitted.ownerFence.epoch &&
        current.actorImplementationIdentity ===
          admitted.ownerFence.implementationIdentity
      ) {
        currentFence = {
          actorKey: current.actorKey,
          epoch: current.epoch,
          implementationIdentity: current.actorImplementationIdentity,
          ownerRuntimeId: current.ownerRuntimeId,
          ownerLeaseId: current.ownerLeaseId,
          ownerLeaseExpiresAt: current.ownerLeaseExpiresAt,
        };
      }
      const requiredOwnerStillMatches = this.requiredOwnerMatches(
        context,
        currentFence
      );
      if (
        context.freshOwnerFence !== undefined &&
        sameActorOwnerFence(context.freshOwnerFence, currentFence) &&
        !requiredOwnerStillMatches
      ) {
        await this.disconnectInitialOwner(
          currentFence,
          error instanceof Error ? error.message : String(error)
        );
      }
      return {
        ok: false,
        reason: requiredOwnerStillMatches
          ? 'DispatchFailed'
          : 'RequiredOwnerMismatch',
      };
    }

    const dispatchedFence = ownerDispatch?.ownerFence ?? admitted.ownerFence;
    if (
      ownerDispatch !== undefined &&
      !sameActorOwnerFence(admitted.ownerFence, dispatchedFence)
    ) {
      await this.failAdmittedInvocation(
        header,
        admitted.ownerFence,
        'Actor owner transport returned a different owner fence'
      );
      return { ok: false, reason: 'DispatchFailed' };
    }
    if (
      this.hasRequiredOwner(context) &&
      (ownerDispatch === undefined ||
        ownerDispatch.ownerConnection !== context.requiredOwnerConnection)
    ) {
      await this.failAdmittedInvocation(
        header,
        admitted.ownerFence,
        'test capability Actor owner connection changed during dispatch'
      );
      return { ok: false, reason: 'RequiredOwnerMismatch' };
    }

    return {
      ok: true,
      ownerFence: dispatchedFence,
      invocation: dispatched.invocation,
      ...(ownerDispatch === undefined
        ? {}
        : { ownerConnection: ownerDispatch.ownerConnection }),
    };
  }

  private hasRequiredOwner(context: ActorMethodDispatchContext): boolean {
    return (
      context.requiredOwnerRuntimeId !== undefined ||
      context.requiredOwnerConnection !== undefined
    );
  }

  private requiredOwnerMatches(
    context: ActorMethodDispatchContext,
    ownerFence?: ActorOwnerFence
  ): boolean {
    if (!this.hasRequiredOwner(context)) return true;
    if (
      context.requiredOwnerRuntimeId === undefined ||
      context.requiredOwnerConnection === undefined
    ) {
      return false;
    }
    if (ownerFence === undefined) {
      return this.requiredOwnerAvailable(
        context,
        context.requiredOwnerRuntimeId
      );
    }
    if (ownerFence.ownerRuntimeId !== context.requiredOwnerRuntimeId) return false;
    return this.requiredOwnerBoundMatches(context, ownerFence);
  }

  private requiredOwnerAvailable(
    context: ActorMethodDispatchContext,
    ownerRuntimeId: string | undefined
  ): boolean {
    if (!this.hasRequiredOwner(context)) return true;
    if (
      context.requiredOwnerRuntimeId === undefined ||
      context.requiredOwnerConnection === undefined ||
      ownerRuntimeId !== context.requiredOwnerRuntimeId
    ) {
      return false;
    }
    return (
      this.transport.ownerConnectionAvailable?.({
        ownerRuntimeId,
        requiredOwnerConnection: context.requiredOwnerConnection,
      }) === true
    );
  }

  private requiredOwnerBoundMatches(
    context: ActorMethodDispatchContext,
    ownerFence: ActorOwnerFence
  ): boolean {
    if (!this.hasRequiredOwner(context)) return true;
    if (
      context.requiredOwnerRuntimeId === undefined ||
      context.requiredOwnerConnection === undefined ||
      ownerFence.ownerRuntimeId !== context.requiredOwnerRuntimeId
    ) {
      return false;
    }
    return this.ownerBoundToConnection(
      ownerFence,
      context.requiredOwnerConnection
    );
  }

  private ownerBoundToConnection(
    ownerFence: ActorOwnerFence,
    ownerConnection: WebSocket
  ): boolean {
    return (
      this.transport.ownerConnectionMatches?.({
        ownerFence,
        requiredOwnerConnection: ownerConnection,
      }) === true
    );
  }

  private activationBelongsToContext(
    activation: InitialActorActivation,
    context: ActorMethodDispatchContext
  ): boolean {
    return (
      activation.requiredOwnerRuntimeId !== undefined &&
      activation.requiredOwnerRuntimeId === context.requiredOwnerRuntimeId &&
      activation.requiredOwnerConnection !== undefined &&
      activation.requiredOwnerConnection === context.requiredOwnerConnection
    );
  }

  private async ensureInitialOwner(
    actorKey: ReturnType<typeof makeActorKey>,
    header: ActorMethodInvokeFrameHeader,
    context: ActorMethodDispatchContext
  ): Promise<InitialActorActivation> {
    const key = actorLogicalKey(actorKey);
    const existing = this.initialActivations.get(key);
    if (existing !== undefined) return existing;

    const activation = this.runInitialActivation(actorKey, header, context).finally(() => {
      if (this.initialActivations.get(key) === activation) {
        this.initialActivations.delete(key);
      }
    });
    this.initialActivations.set(key, activation);
    return activation;
  }

  private async runInitialActivation(
    actorKey: ReturnType<typeof makeActorKey>,
    header: ActorMethodInvokeFrameHeader,
    context: ActorMethodDispatchContext
  ): Promise<InitialActorActivation> {
    if (this.transport.activateInitial === undefined) {
      throw new Error('initial Actor activation transport is unavailable');
    }

    let acquiredFence: ActorOwnerFence | undefined;
    let ownerConnectionBinding: ActorOwnerConnectionBinding | undefined;
    try {
      let owner: Awaited<ReturnType<NonNullable<ActorOwnerTransport['activateInitial']>>>;
      try {
        owner = await this.transport.activateInitial({
          header,
          ...(context.authority === undefined
            ? {}
            : { authority: context.authority }),
          ...(context.requiredOwnerRuntimeId === undefined
            ? {}
            : { requiredOwnerRuntimeId: context.requiredOwnerRuntimeId }),
          ...(context.requiredOwnerConnection === undefined
            ? {}
            : { requiredOwnerConnection: context.requiredOwnerConnection }),
        });
      } catch (error) {
        if (this.hasRequiredOwner(context)) {
          throw new RequiredOwnerConnectionChangedError(
            error instanceof Error ? error.message : String(error)
          );
        }
        throw error;
      }
      if (!this.requiredOwnerAvailable(context, owner.ownerRuntimeId)) {
        throw new RequiredOwnerConnectionChangedError(
          'test capability origin Runtime connection changed during initial Actor activation'
        );
      }
      if (
        this.hasRequiredOwner(context) &&
        owner.ownerConnection !== undefined &&
        owner.ownerConnection !== context.requiredOwnerConnection
      ) {
        throw new RequiredOwnerConnectionChangedError(
          'initial Actor owner connection differs from the required Runtime session'
        );
      }

      const acquired = await this.actorManager.registryStore().acquireOwnerLease({
        actorKey,
        expectedEpoch: header.actorRef.epoch,
        actorImplementationIdentity: header.actorImplementationIdentity,
        ownerRuntimeId: owner.ownerRuntimeId,
        ownerLeaseId: owner.ownerLeaseId,
        ownerLeaseExpiresAt: owner.ownerLeaseExpiresAt,
        now: this.now(),
      });
      if (!acquired.ok) {
        throw new Error(`new Actor owner lease was rejected: ${acquired.reason}`);
      }
      acquiredFence = acquired.fence;
      if (!this.requiredOwnerAvailable(context, acquiredFence.ownerRuntimeId)) {
        throw new RequiredOwnerConnectionChangedError(
          'test capability origin Runtime connection changed while acquiring initial Actor owner'
        );
      }
      const ownerConnection = owner.ownerConnection;
      if (
        this.hasRequiredOwner(context) &&
        ownerConnection !== context.requiredOwnerConnection
      ) {
        throw new RequiredOwnerConnectionChangedError(
          'initial Actor owner connection differs from the required Runtime session'
        );
      }
      ownerConnectionBinding = this.transport.bindOwnerConnection?.({
        ownerFence: acquiredFence,
        requiredOwnerConnection: ownerConnection,
      });
      if (ownerConnectionBinding === undefined) {
        if (this.hasRequiredOwner(context)) {
          throw new RequiredOwnerConnectionChangedError(
            'test capability origin Runtime connection changed while binding initial Actor owner'
          );
        }
        throw new Error(
          'initial Actor owner could not be bound to its Runtime session'
        );
      }

      const markedLive = await this.actorManager.registryStore().markOwnerLive({
        actorKey,
        expectedEpoch: acquiredFence.epoch,
        actorImplementationIdentity: acquiredFence.implementationIdentity,
        ownerRuntimeId: acquiredFence.ownerRuntimeId,
        ownerLeaseId: acquiredFence.ownerLeaseId,
        now: this.now(),
      });
      if (!markedLive) {
        throw new Error('new Actor owner lease could not be marked live');
      }
      if (!this.ownerBoundToConnection(acquiredFence, ownerConnection)) {
        const message =
          'initial Actor owner Runtime connection changed while marking it live';
        if (this.hasRequiredOwner(context)) {
          throw new RequiredOwnerConnectionChangedError(message);
        }
        throw new Error(message);
      }

      return {
        fence: acquiredFence,
        ...(ownerConnectionBinding === undefined
          ? {}
          : { ownerConnectionBinding }),
        ...(context.requiredOwnerRuntimeId === undefined
          ? {}
          : { requiredOwnerRuntimeId: context.requiredOwnerRuntimeId }),
        ...(context.requiredOwnerConnection === undefined
          ? {}
          : { requiredOwnerConnection: context.requiredOwnerConnection }),
      };
    } catch (error) {
      if (acquiredFence !== undefined) {
        await this.disconnectInitialOwner(
          acquiredFence,
          error instanceof Error ? error.message : String(error),
          ownerConnectionBinding
        );
      }
      throw error;
    }
  }

  private async disconnectInitialOwner(
    fence: ActorOwnerFence,
    terminalReason: string,
    ownerConnectionBinding?: ActorOwnerConnectionBinding
  ): Promise<void> {
    try {
      await this.actorManager.registryStore().disconnectOwner({
        fence,
        now: this.now(),
        terminalReason,
      });
    } finally {
      ownerConnectionBinding?.unbind();
    }
  }

  private async failAdmittedInvocation(
    header: ActorMethodInvokeFrameHeader,
    ownerFence: ActorOwnerFence,
    terminalReason: string
  ): Promise<void> {
    await this.actorManager.registryStore().transitionActorInvocation({
      invocationId: header.invocationId,
      actorKey: ownerFence.actorKey,
      expectedEpoch: ownerFence.epoch,
      actorImplementationIdentity: ownerFence.implementationIdentity,
      ownerRuntimeId: ownerFence.ownerRuntimeId,
      ownerLeaseId: ownerFence.ownerLeaseId,
      nextState: 'failed',
      terminalReason,
      now: this.now(),
    });
  }

  private async advanceUpgrade(
    actorKey: ReturnType<typeof makeActorKey>,
    deadlineAt: Date,
    header: ActorMethodInvokeFrameHeader,
    authority?: ActorOwnerRouteAuthority
  ): Promise<boolean> {
    const key = actorLogicalKey(actorKey);
    const existing = this.upgrades.get(key);
    if (existing !== undefined) return waitUntilDeadline(existing, deadlineAt);
    const upgrade = this.runUpgrade(actorKey, header, authority).finally(() => {
      if (this.upgrades.get(key) === upgrade) this.upgrades.delete(key);
    });
    this.upgrades.set(key, upgrade);
    return waitUntilDeadline(upgrade, deadlineAt);
  }

  private async runUpgrade(
    actorKey: ReturnType<typeof makeActorKey>,
    header: ActorMethodInvokeFrameHeader,
    authority?: ActorOwnerRouteAuthority
  ): Promise<boolean> {
    const store = this.actorManager.registryStore();
    const fence = await store.actorUpgradeFence(actorKey);
    if (
      fence === undefined ||
      this.transport.markOwnerUpgrading === undefined ||
      this.transport.discardOldInstance === undefined ||
      this.transport.activateTarget === undefined
    ) {
      return false;
    }
    try {
      await this.transport.markOwnerUpgrading({
        fence,
        header,
        ...(authority === undefined ? {} : { authority }),
      });
      const drained = await store.waitForActorUpgradeDrain({ fence });
      if (drained !== 'Drained') return false;
      await this.transport.discardOldInstance({
        fence,
        header,
        ...(authority === undefined ? {} : { authority }),
      });
      const completed = await store.completeActorUpgrade({ fence, now: this.now() });
      if (!completed.ok) return false;
      const target = await this.transport.activateTarget({
        transition: completed.transition,
        header,
        ...(authority === undefined ? {} : { authority }),
      });
      const acquired = await store.acquireOwnerLease({
        actorKey: completed.transition.actorKey,
        expectedEpoch: completed.transition.newEpoch,
        actorImplementationIdentity:
          completed.transition.targetImplementationIdentity,
        ownerRuntimeId: target.ownerRuntimeId,
        ownerLeaseId: target.ownerLeaseId,
        ownerLeaseExpiresAt: target.ownerLeaseExpiresAt,
        now: this.now(),
      });
      if (!acquired.ok) return false;
      const binding = this.transport.bindOwnerConnection?.({
        ownerFence: acquired.fence,
        requiredOwnerConnection: target.ownerConnection,
      });
      if (binding === undefined) {
        await this.disconnectInitialOwner(
          acquired.fence,
          'upgraded Actor owner could not be bound to its Runtime session'
        );
        return false;
      }
      let markedLive: boolean;
      try {
        markedLive = await store.markOwnerLive({
          actorKey: completed.transition.actorKey,
          expectedEpoch: completed.transition.newEpoch,
          actorImplementationIdentity:
            completed.transition.targetImplementationIdentity,
          ownerRuntimeId: target.ownerRuntimeId,
          ownerLeaseId: target.ownerLeaseId,
          now: this.now(),
        });
      } catch (error) {
        await this.disconnectInitialOwner(
          acquired.fence,
          error instanceof Error ? error.message : String(error),
          binding
        );
        throw error;
      }
      if (!markedLive) {
        await this.disconnectInitialOwner(
          acquired.fence,
          'upgraded Actor owner lease could not be marked live',
          binding
        );
        return false;
      }
      if (!this.ownerBoundToConnection(acquired.fence, target.ownerConnection)) {
        await this.disconnectInitialOwner(
          acquired.fence,
          'upgraded Actor owner Runtime connection changed while marking it live',
          binding
        );
        return false;
      }
      return true;
    } catch {
      return false;
    }
  }
}

class RequiredOwnerConnectionChangedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'RequiredOwnerConnectionChangedError';
  }
}

async function waitUntilDeadline(
  upgrade: Promise<boolean>,
  deadlineAt: Date
): Promise<boolean> {
  const remainingMs = deadlineAt.getTime() - Date.now();
  if (remainingMs <= 0) return false;
  let timer: NodeJS.Timeout | undefined;
  try {
    return await Promise.race([
      upgrade,
      new Promise<false>((resolve) => {
        timer = setTimeout(() => resolve(false), remainingMs);
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
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
