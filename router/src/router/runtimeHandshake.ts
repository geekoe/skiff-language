// Per-connection handshake phase state machine
// (C-model-registration §2, authority design §3.5).
//
// This is the single phase authority for one physical Runtime connection. It
// is deliberately free of sockets, directories and time: the RuntimeEndpoint
// drives it, and AssemblyRuntimeRegistry performs the corresponding pending
// publish / commit / rollback mutations. The corpus reference model
// (`runtime/transport/testdata/registration-handshake/`) and this machine
// implement the same frozen semantics; post-commit re-register follows
// authority design §3.2 / C-session §3.3.

export type RuntimeHandshakePhase =
  | 'accepted'
  | 'bootstrap-sent'
  | 'capabilities-bound'
  | 'register-validated'
  | 'registered'
  | 'closed';

/**
 * Strict terminal classification (C-model-registration §2.3).
 * Names match the frozen corpus scenario outcomes exactly.
 */
export type RuntimeHandshakeTerminalKind =
  | 'WrongOrder'
  | 'IdentityChange'
  | 'DuplicateRegister'
  | 'StaleRegister'
  | 'NewGenerationBeforeEpochSwap'
  | 'LegacyRegisterRejected'
  | 'BootstrapWriteFail'
  | 'AckLoss'
  | 'BootstrapTimeout'
  | 'CapabilitiesTimeout'
  | 'RegisterTimeout'
  | 'Disconnect'
  | 'PreAuthLimitRejected'
  | 'RegistrationRefused';

export interface RuntimeRegisteredAssemblyTuple {
  environment: string;
  generation: number;
  assembly: {
    assemblyIdentity: string;
  };
  configSnapshot: {
    snapshotId: string;
  };
}

export interface RuntimeHandshakeRegisterControl
  extends RuntimeRegisteredAssemblyTuple {
  replicaId: string;
}

/**
 * Captured routing epoch context for register validation. `pending` is the
 * activation epoch before durable commit/swap.
 */
export interface RuntimeHandshakeEpochContext {
  current?: RuntimeRegisteredAssemblyTuple;
  pending?: RuntimeRegisteredAssemblyTuple;
}

export type RuntimeHandshakeCapabilitiesEvent =
  | { kind: 'bound' }
  | { kind: 'terminal'; terminal: RuntimeHandshakeTerminalKind };

export type RuntimeHandshakeRegisterEvent =
  | { kind: 'validated'; tuple: RuntimeRegisteredAssemblyTuple }
  | { kind: 'idempotent' }
  | { kind: 'transition'; tuple: RuntimeRegisteredAssemblyTuple }
  | { kind: 'terminal'; terminal: RuntimeHandshakeTerminalKind };

export type RuntimeHandshakeHealthEvent =
  | { kind: 'observed' }
  | { kind: 'droppedBeforeAck' }
  | { kind: 'terminal'; terminal: RuntimeHandshakeTerminalKind };

export type RuntimeHandshakeTimeoutKind =
  | 'bootstrap'
  | 'capabilities'
  | 'register';

export class RuntimeHandshakeState {
  private currentPhase: RuntimeHandshakePhase = 'accepted';
  private currentTerminal: RuntimeHandshakeTerminalKind | undefined;
  private boundReplica: string | undefined;
  private currentRegisteredTuple: RuntimeRegisteredAssemblyTuple | undefined;
  private healthBeforeAckCount = 0;

  phase(): RuntimeHandshakePhase {
    return this.currentPhase;
  }

  terminal(): RuntimeHandshakeTerminalKind | undefined {
    return this.currentTerminal;
  }

  replica(): string | undefined {
    return this.boundReplica;
  }

  registeredTuple(): RuntimeRegisteredAssemblyTuple | undefined {
    return this.currentRegisteredTuple;
  }

  healthBeforeAck(): number {
    return this.healthBeforeAckCount;
  }

  isClosed(): boolean {
    return this.currentPhase === 'closed';
  }

  outcomeName(): string {
    if (this.currentPhase === 'closed' && this.currentTerminal !== undefined) {
      return this.currentTerminal;
    }
    if (this.currentPhase !== 'closed' && this.currentTerminal === undefined) {
      // Corpus/reference-model outcome names use capitalized phase labels
      // (`Registered`, `Accepted`, ...).
      return this.currentPhase
        .split('-')
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join('');
    }
    return 'Closed';
  }

  private setTerminal(kind: RuntimeHandshakeTerminalKind): void {
    if (this.currentTerminal !== undefined) {
      throw new Error(`handshake terminal already set to ${this.currentTerminal}`);
    }
    this.currentTerminal = kind;
    this.currentPhase = 'closed';
  }

  /**
   * The `router.bootstrap` frame was written successfully
   * (Accepted -> BootstrapSent).
   */
  onBootstrapWritten(): RuntimeHandshakeTerminalKind | undefined {
    if (this.currentPhase !== 'accepted') {
      const terminal: RuntimeHandshakeTerminalKind = 'WrongOrder';
      this.setTerminal(terminal);
      return terminal;
    }
    this.currentPhase = 'bootstrap-sent';
    return undefined;
  }

  onBootstrapWriteFailed(): RuntimeHandshakeTerminalKind {
    const terminal: RuntimeHandshakeTerminalKind = 'BootstrapWriteFail';
    this.setTerminal(terminal);
    return terminal;
  }

  onCapabilities(runtimeId: string): RuntimeHandshakeCapabilitiesEvent {
    switch (this.currentPhase) {
      case 'accepted': {
        const terminal: RuntimeHandshakeTerminalKind = 'WrongOrder';
        this.setTerminal(terminal);
        return { kind: 'terminal', terminal };
      }
      case 'bootstrap-sent': {
        if (this.boundReplica !== undefined) {
          const terminal: RuntimeHandshakeTerminalKind =
            this.boundReplica === runtimeId ? 'WrongOrder' : 'IdentityChange';
          this.setTerminal(terminal);
          return { kind: 'terminal', terminal };
        }
        this.boundReplica = runtimeId;
        this.currentPhase = 'capabilities-bound';
        return { kind: 'bound' };
      }
      case 'capabilities-bound':
      case 'register-validated':
      case 'registered': {
        const terminal: RuntimeHandshakeTerminalKind =
          this.boundReplica === runtimeId ? 'WrongOrder' : 'IdentityChange';
        this.setTerminal(terminal);
        return { kind: 'terminal', terminal };
      }
      case 'closed': {
        const terminal: RuntimeHandshakeTerminalKind = 'WrongOrder';
        this.setTerminal(terminal);
        return { kind: 'terminal', terminal };
      }
    }
  }

  onRegister(
    register: RuntimeHandshakeRegisterControl,
    context: RuntimeHandshakeEpochContext
  ): RuntimeHandshakeRegisterEvent {
    const snapshot = {
      phase: this.currentPhase,
      replica: this.boundReplica,
      tuple: this.currentRegisteredTuple
    };
    const tuple: RuntimeRegisteredAssemblyTuple = {
      environment: register.environment,
      generation: register.generation,
      assembly: { ...register.assembly },
      configSnapshot: { ...register.configSnapshot }
    };
    const tupleEquals = (candidate: RuntimeRegisteredAssemblyTuple): boolean =>
      candidate.environment === tuple.environment &&
      candidate.generation === tuple.generation &&
      candidate.assembly.assemblyIdentity === tuple.assembly.assemblyIdentity &&
      candidate.configSnapshot.snapshotId === tuple.configSnapshot.snapshotId;
    switch (snapshot.phase) {
      case 'accepted':
      case 'bootstrap-sent': {
        const terminal: RuntimeHandshakeTerminalKind = 'WrongOrder';
        this.setTerminal(terminal);
        return { kind: 'terminal', terminal };
      }
      case 'capabilities-bound': {
        if (snapshot.replica !== register.replicaId) {
          const terminal: RuntimeHandshakeTerminalKind = 'IdentityChange';
          this.setTerminal(terminal);
          return { kind: 'terminal', terminal };
        }
        if (context.current !== undefined && tupleEquals(context.current)) {
          this.currentRegisteredTuple = tuple;
          this.currentPhase = 'register-validated';
          return { kind: 'validated', tuple };
        }
        if (context.pending !== undefined && tupleEquals(context.pending)) {
          const terminal: RuntimeHandshakeTerminalKind =
            'NewGenerationBeforeEpochSwap';
          this.setTerminal(terminal);
          return { kind: 'terminal', terminal };
        }
        const stale: RuntimeHandshakeTerminalKind = 'StaleRegister';
        this.setTerminal(stale);
        return { kind: 'terminal', terminal: stale };
      }
      case 'register-validated': {
        const terminal: RuntimeHandshakeTerminalKind = 'DuplicateRegister';
        this.setTerminal(terminal);
        return { kind: 'terminal', terminal };
      }
      case 'registered': {
        if (snapshot.replica !== register.replicaId) {
          const terminal: RuntimeHandshakeTerminalKind = 'IdentityChange';
          this.setTerminal(terminal);
          return { kind: 'terminal', terminal };
        }
        if (snapshot.tuple !== undefined && tupleEquals(snapshot.tuple)) {
          return { kind: 'idempotent' };
        }
        if (context.current !== undefined && tupleEquals(context.current)) {
          // Post-commit re-register on the same physical session:
          // RuntimeRegistrationTransition publishes the new revision
          // (authority design §3.2, C-session §3.3).
          this.currentRegisteredTuple = tuple;
          return { kind: 'transition', tuple };
        }
        if (context.pending !== undefined && tupleEquals(context.pending)) {
          const terminal: RuntimeHandshakeTerminalKind =
            'NewGenerationBeforeEpochSwap';
          this.setTerminal(terminal);
          return { kind: 'terminal', terminal };
        }
        const stale: RuntimeHandshakeTerminalKind = 'StaleRegister';
        this.setTerminal(stale);
        return { kind: 'terminal', terminal: stale };
      }
      case 'closed': {
        const terminal: RuntimeHandshakeTerminalKind = 'WrongOrder';
        this.setTerminal(terminal);
        return { kind: 'terminal', terminal };
      }
    }
  }

  onLegacyRegister(): RuntimeHandshakeTerminalKind {
    const terminal: RuntimeHandshakeTerminalKind = 'LegacyRegisterRejected';
    this.setTerminal(terminal);
    return terminal;
  }

  onHealth(runtimeId: string): RuntimeHandshakeHealthEvent {
    switch (this.currentPhase) {
      case 'registered': {
        if (this.boundReplica !== runtimeId) {
          const terminal: RuntimeHandshakeTerminalKind = 'IdentityChange';
          this.setTerminal(terminal);
          return { kind: 'terminal', terminal };
        }
        return { kind: 'observed' };
      }
      case 'register-validated': {
        if (this.boundReplica !== runtimeId) {
          const terminal: RuntimeHandshakeTerminalKind = 'IdentityChange';
          this.setTerminal(terminal);
          return { kind: 'terminal', terminal };
        }
        this.healthBeforeAckCount += 1;
        return { kind: 'droppedBeforeAck' };
      }
      default: {
        const terminal: RuntimeHandshakeTerminalKind = 'WrongOrder';
        this.setTerminal(terminal);
        return { kind: 'terminal', terminal };
      }
    }
  }

  /**
   * The `runtime.registered` ACK was written (RegisterValidated ->
   * Registered).
   */
  onAckWritten(): RuntimeHandshakeTerminalKind | undefined {
    if (this.currentPhase !== 'register-validated') {
      const terminal: RuntimeHandshakeTerminalKind = 'WrongOrder';
      this.setTerminal(terminal);
      return terminal;
    }
    this.currentPhase = 'registered';
    return undefined;
  }

  onAckWriteFailed(): RuntimeHandshakeTerminalKind {
    const terminal: RuntimeHandshakeTerminalKind = 'AckLoss';
    this.setTerminal(terminal);
    return terminal;
  }

  onTimeout(kind: RuntimeHandshakeTimeoutKind): RuntimeHandshakeTerminalKind {
    const terminal: RuntimeHandshakeTerminalKind = (() => {
      switch (kind) {
        case 'bootstrap':
          return this.currentPhase === 'accepted'
            ? 'BootstrapTimeout'
            : 'Disconnect';
        case 'capabilities':
          return this.currentPhase === 'bootstrap-sent'
            ? 'CapabilitiesTimeout'
            : 'Disconnect';
        case 'register':
          return this.currentPhase === 'capabilities-bound'
            ? 'RegisterTimeout'
            : 'Disconnect';
      }
    })();
    this.setTerminal(terminal);
    return terminal;
  }

  onDisconnect(): RuntimeHandshakeTerminalKind {
    const terminal: RuntimeHandshakeTerminalKind = 'Disconnect';
    this.setTerminal(terminal);
    return terminal;
  }

  terminalWith(kind: RuntimeHandshakeTerminalKind): RuntimeHandshakeTerminalKind {
    this.setTerminal(kind);
    return kind;
  }
}

/** Handshake deadlines (C-session §4 defaults, process-level constants). */
export const RUNTIME_HANDSHAKE_TIMEOUTS: {
  bootstrapMs: number;
  capabilitiesMs: number;
  registerMs: number;
  ackWriteMs: number;
} = {
  bootstrapMs: 10_000,
  capabilitiesMs: 10_000,
  registerMs: 30_000,
  ackWriteMs: 5_000
};

export function runtimeHandshakeTerminalDescription(
  kind: RuntimeHandshakeTerminalKind
): string {
  switch (kind) {
    case 'WrongOrder':
      return 'frame arrived outside its handshake phase';
    case 'IdentityChange':
      return 'replica identity changed on one connection';
    case 'DuplicateRegister':
      return 'second register before the ACK';
    case 'StaleRegister':
      return 'register tuple does not match the committed epoch';
    case 'NewGenerationBeforeEpochSwap':
      return 'register matches a pending epoch that is not yet committed';
    case 'LegacyRegisterRejected':
      return 'legacy runtime registration frame is not a handshake frame';
    case 'BootstrapWriteFail':
      return 'router.bootstrap write failed';
    case 'AckLoss':
      return 'runtime.registered ACK write failed';
    case 'BootstrapTimeout':
      return 'bootstrap deadline expired';
    case 'CapabilitiesTimeout':
      return 'capabilities deadline expired';
    case 'RegisterTimeout':
      return 'register deadline expired';
    case 'Disconnect':
      return 'physical connection closed';
    case 'PreAuthLimitRejected':
      return 'pre-auth connection limit reached';
    case 'RegistrationRefused':
      return 'registration is not accepted by this runtime endpoint';
  }
}
