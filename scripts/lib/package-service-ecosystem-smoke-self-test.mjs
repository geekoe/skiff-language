import assert from 'node:assert/strict';

/// Pure state-machine coverage for the WebSocket cutover invariant. It does not
/// duplicate Router framing, parsing, or runtime execution.
export async function runPackageServiceSmokeSelfTest(replicaCount) {
  const replicas = Array.from({ length: replicaCount }, (_, index) => ({
    id: `runtime-${index}`,
    healthy: true,
    runtimeHome: `/tmp/skiff-cutover-self-test/runtime-${index}`,
  }));
  const state = new CutoverState(replicas);
  assert.equal(state.registrationsAt(0, 'assembly-empty'), replicaCount);

  state.activate('assembly-a', 'A');
  const connectionA = state.connect();
  assert.deepEqual(state.receive(connectionA), {
    assembly: 'assembly-a',
    generation: 1,
    marker: 'A',
  });

  state.rejectNext = true;
  assert.throws(() => state.activate('assembly-rejected', 'rejected'), /prepare rejected/);
  assert.equal(state.assembly, 'assembly-a');
  assert.equal(state.generation, 1);

  state.activate('assembly-b', 'B');
  const connectionB = state.connect();
  assert.deepEqual(state.receive(connectionB), {
    assembly: 'assembly-b',
    generation: 2,
    marker: 'B',
  });
  assert.deepEqual(state.receive(connectionA), {
    assembly: 'assembly-a',
    generation: 1,
    marker: 'A',
  });
  state.close(connectionA);
  assert.equal(connectionA.closed, true);

  let replicaFailover = false;
  if (replicas.length === 2) {
    const first = state.pickReplica();
    const second = state.pickReplica();
    assert.notEqual(first.id, second.id);
    replicas[0].healthy = false;
    assert.equal(state.pickReplica().id, replicas[1].id);
    replicaFailover = true;
  }

  return {
    status: 'PASS',
    probe: 'skiff-cutover-self-test',
    replicas: replicaCount,
    assembly: state.assembly,
    generation: state.generation,
    webSocketGenerationPin: {
      connectionA: 'assembly-a@1',
      connectionB: 'assembly-b@2',
      oldReceiveAfterCommit: 'A',
      closedNaturally: true,
    },
    activationAbortRollback: true,
    replicaFailover,
    temporaryRuntimeHomes: new Set(replicas.map((replica) => replica.runtimeHome)).size
      === replicaCount,
  };
}

class CutoverState {
  constructor(replicas) {
    this.replicas = replicas;
    this.assembly = 'assembly-empty';
    this.generation = 0;
    this.marker = 'empty';
    this.cursor = 0;
    this.rejectNext = false;
    this.registrations = replicas.map((replica) => ({
      replica: replica.id,
      assembly: this.assembly,
      generation: this.generation,
    }));
  }

  registrationsAt(generation, assembly) {
    return this.registrations.filter((entry) => (
      entry.generation === generation && entry.assembly === assembly
    )).length;
  }

  activate(assembly, marker) {
    if (this.rejectNext) {
      this.rejectNext = false;
      throw new Error('prepare rejected');
    }
    this.generation += 1;
    this.assembly = assembly;
    this.marker = marker;
    this.registrations = this.replicas
      .filter((replica) => replica.healthy)
      .map((replica) => ({ replica: replica.id, assembly, generation: this.generation }));
  }

  connect() {
    return {
      assembly: this.assembly,
      generation: this.generation,
      marker: this.marker,
      replica: this.pickReplica().id,
      closed: false,
    };
  }

  receive(connection) {
    assert.equal(connection.closed, false);
    return {
      assembly: connection.assembly,
      generation: connection.generation,
      marker: connection.marker,
    };
  }

  close(connection) {
    connection.closed = true;
  }

  pickReplica() {
    const healthy = this.replicas.filter((replica) => replica.healthy);
    assert.ok(healthy.length > 0, 'no healthy replica');
    const replica = healthy[this.cursor % healthy.length];
    this.cursor += 1;
    return replica;
  }
}
