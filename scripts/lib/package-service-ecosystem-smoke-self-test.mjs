import assert from 'node:assert/strict';
import http from 'node:http';

export async function runPackageServiceSmokeSelfTest(replicaCount) {
  const replicas = Array.from(
    { length: replicaCount },
    (_, index) => new Replica(`runtime-${index}`),
  );
  const coordinator = new Coordinator(replicas);
  const gateway = await startGateway(coordinator);
  try {
    assert.equal(coordinator.capabilityConnections.size, replicaCount);
    assert.equal(coordinator.registrationsAt(0, 'assembly-empty').length, replicaCount);

    await coordinator.activate('assembly-old', 'old');
    const oldUnary = JSON.parse(await hostRequest(gateway.port));
    assert.equal(oldUnary.result, 'old');
    assert.equal(oldUnary.spawnTypedResponse, true);

    const pinned = await openHostStream(gateway.port);
    assert.match(pinned.firstChunk, /"result":"old"/);
    assert.equal(pinned.isOpen(), true);
    assert.equal(coordinator.pinnedGenerationCount(1), 1);

    replicas.at(-1).rejectNext = true;
    await assert.rejects(
      coordinator.activate('assembly-rejected', 'rejected'),
      /prepare rejected/,
    );
    assert.equal(coordinator.assembly, 'assembly-old');
    assert.equal(coordinator.generation, 1);
    assert.ok(replicas.every((replica) => replica.pending === null));
    assert.equal(JSON.parse(await hostRequest(gateway.port)).result, 'old');

    await coordinator.activate('assembly-new', 'new');
    assert.equal(pinned.isOpen(), true, 'A stream must remain open after B commit');
    assert.equal(coordinator.pinnedGenerationCount(1), 1);
    const newUnary = JSON.parse(await hostRequest(gateway.port));
    assert.equal(newUnary.result, 'new');

    coordinator.releasePinnedStreams();
    const oldStream = await pinned.completion;
    assert.match(oldStream, /"phase":"start","result":"old"/);
    assert.match(oldStream, /"phase":"end","result":"old"/);
    assert.doesNotMatch(oldStream, /"result":"new"/);
    assert.equal(coordinator.pinnedGenerationCount(1), 0);

    let replicaFailover = false;
    if (replicas.length === 2) {
      const first = JSON.parse(await hostRequest(gateway.port));
      const second = JSON.parse(await hostRequest(gateway.port));
      assert.notEqual(first.replicaId, second.replicaId);
      replicas[0].healthy = false;
      const failover = JSON.parse(await hostRequest(gateway.port));
      assert.equal(failover.replicaId, replicas[1].id);
      replicaFailover = true;
    }

    assert.ok(coordinator.routerToRuntimeTypes.has('assembly.activation.prepare'));
    assert.ok(coordinator.routerToRuntimeTypes.has('assembly.activation.commit'));
    assert.ok(coordinator.routerToRuntimeTypes.has('assembly.activation.abort'));
    assert.ok(coordinator.runtimeToRouterTypes.has('runtime.capabilities'));
    assert.ok(coordinator.runtimeToRouterTypes.has('assembly.activation.prepared'));
    assert.ok(coordinator.runtimeToRouterTypes.has('assembly.activation.register'));
    assert.ok(coordinator.runtimeToRouterTypes.has('spawn.submit'));
    assert.ok(coordinator.routerToRuntimeTypes.has('spawn.submit.response'));

    return {
      status: 'PASS',
      probe: 'skiff-cutover-self-test',
      replicas: replicaCount,
      assembly: coordinator.assembly,
      generation: coordinator.generation,
      hostResult: 'new',
      capabilitiesHandshake: true,
      coldStartupRegistration: true,
      binaryActivationRoundTrip: {
        transportCodec: 'production-owned; exercised only by the real smoke',
        routerToRuntime: [...coordinator.routerToRuntimeTypes].sort(),
        runtimeToRouter: [...coordinator.runtimeToRouterTypes].sort(),
      },
      spawnTypedResponse: coordinator.spawnTypedResponseObserved,
      oldGenerationStreamPin: true,
      activationAbortRollback: true,
      replicaFailover,
      temporaryRuntimeHomes: new Set(replicas.map((replica) => replica.runtimeHome)).size
        === replicaCount,
    };
  } finally {
    coordinator.releasePinnedStreams();
    await gateway.close();
    await Promise.all(replicas.map((replica) => replica.close()));
  }
}

class Coordinator {
  constructor(replicas) {
    this.replicas = replicas;
    this.assembly = 'assembly-empty';
    this.generation = 0;
    this.cursor = 0;
    this.pins = new Set();
    this.capabilityConnections = new Set();
    this.registrations = new Map();
    this.routerToRuntimeTypes = new Set();
    this.runtimeToRouterTypes = new Set();
    this.spawnTypedResponseObserved = false;
    for (const replica of replicas) {
      const capabilities = this.receiveRuntimeMessage(replica.capabilitiesMessage());
      assert.equal(capabilities.type, 'runtime.capabilities');
      assert.equal(capabilities.runtimeId, replica.id);
      this.capabilityConnections.add(replica.id);
      this.recordRegistration(this.receiveRuntimeMessage(replica.registrationMessage()));
    }
  }

  async activate(assembly, result) {
    const candidateGeneration = this.generation + 1;
    const activationId = `activation-${candidateGeneration}`;
    const prepared = [];
    try {
      for (const replica of this.replicas.filter((candidate) => candidate.healthy)) {
        const response = this.sendRouterMessage(replica, {
          type: 'assembly.activation.prepare',
          activationId,
          candidateGeneration,
          assembly,
          result,
        });
        if (response.type === 'assembly.activation.reject') {
          throw new Error(`prepare rejected by ${replica.id}`);
        }
        assert.equal(response.type, 'assembly.activation.prepared');
        prepared.push(replica);
      }
    } catch (error) {
      for (const replica of this.replicas.filter((candidate) => candidate.healthy)) {
        const aborted = this.sendRouterMessage(replica, {
          type: 'assembly.activation.abort',
          activationId,
        });
        assert.equal(aborted.type, 'assembly.activation.aborted');
      }
      throw error;
    }
    for (const replica of prepared) {
      const registration = this.sendRouterMessage(replica, {
        type: 'assembly.activation.commit',
        activationId,
      });
      this.recordRegistration(registration);
    }
    this.assembly = assembly;
    this.generation = candidateGeneration;
  }

  sendRouterMessage(replica, message) {
    this.routerToRuntimeTypes.add(message.type);
    return this.receiveRuntimeMessage(replica.receiveRouterMessage(structuredClone(message)));
  }

  receiveRuntimeMessage(message) {
    assert.equal(typeof message?.type, 'string');
    this.runtimeToRouterTypes.add(message.type);
    return structuredClone(message);
  }

  recordRegistration(header) {
    assert.equal(header.type, 'assembly.activation.register');
    this.registrations.set(header.replicaId, header);
  }

  registrationsAt(generation, assembly) {
    return [...this.registrations.values()].filter((registration) => (
      registration.generation === generation && registration.assembly === assembly
    ));
  }

  healthyReplica() {
    const healthy = this.replicas.filter((replica) => replica.healthy);
    assert.ok(healthy.length > 0, 'no healthy assembly replica');
    const replica = healthy[this.cursor % healthy.length];
    this.cursor += 1;
    return replica;
  }

  executeUnary() {
    const replica = this.healthyReplica();
    const submit = this.receiveRuntimeMessage(replica.spawnSubmitMessage());
    assert.equal(submit.type, 'spawn.submit');
    const response = {
      type: 'spawn.submit.response',
      requestId: submit.requestId,
      accepted: true,
    };
    this.routerToRuntimeTypes.add(response.type);
    replica.receiveSpawnResponse(structuredClone(response));
    this.spawnTypedResponseObserved = true;
    return {
      replicaId: replica.id,
      result: replica.active.result,
      generation: replica.active.generation,
      spawnTypedResponse: true,
    };
  }

  openPinnedStream() {
    const replica = this.healthyReplica();
    const gate = deferred();
    const pin = {
      replicaId: replica.id,
      result: replica.active.result,
      generation: replica.active.generation,
      release: gate.resolve,
      released: gate.promise,
    };
    this.pins.add(pin);
    gate.promise.finally(() => this.pins.delete(pin));
    return pin;
  }

  pinnedGenerationCount(generation) {
    return [...this.pins].filter((pin) => pin.generation === generation).length;
  }

  releasePinnedStreams() {
    for (const pin of this.pins) pin.release();
  }
}

class Replica {
  constructor(id) {
    this.id = id;
    this.runtimeHome = `/tmp/skiff-cutover-self-test/${id}`;
    this.healthy = true;
    this.rejectNext = false;
    this.pending = null;
    this.active = { assembly: 'assembly-empty', generation: 0, result: 'empty' };
  }

  capabilitiesMessage() {
    return {
      type: 'runtime.capabilities',
      runtimeId: this.id,
      modes: ['unary', 'serverStream', 'spawn'],
    };
  }

  registrationMessage() {
    return {
      type: 'assembly.activation.register',
      replicaId: this.id,
      assembly: this.active.assembly,
      generation: this.active.generation,
    };
  }

  receiveRouterMessage(message) {
    if (message.type === 'assembly.activation.prepare') {
      if (this.rejectNext) {
        this.rejectNext = false;
        return {
          type: 'assembly.activation.reject',
          activationId: message.activationId,
          replicaId: this.id,
        };
      }
      this.pending = message;
      return {
        type: 'assembly.activation.prepared',
        activationId: message.activationId,
        replicaId: this.id,
      };
    }
    if (message.type === 'assembly.activation.abort') {
      if (this.pending?.activationId === message.activationId) this.pending = null;
      return {
        type: 'assembly.activation.aborted',
        activationId: message.activationId,
        replicaId: this.id,
      };
    }
    assert.equal(message.type, 'assembly.activation.commit');
    assert.equal(this.pending?.activationId, message.activationId);
    this.active = {
      assembly: this.pending.assembly,
      generation: this.pending.candidateGeneration,
      result: this.pending.result,
    };
    this.pending = null;
    return this.registrationMessage();
  }

  spawnSubmitMessage() {
    return {
      type: 'spawn.submit',
      requestId: `spawn-${this.id}-${this.active.generation}`,
      target: 'typedSpawn',
    };
  }

  receiveSpawnResponse(message) {
    assert.deepEqual(message, {
      type: 'spawn.submit.response',
      requestId: `spawn-${this.id}-${this.active.generation}`,
      accepted: true,
    });
  }

  async close() {}
}

async function startGateway(coordinator) {
  const server = http.createServer(async (request, response) => {
    if (request.headers.host !== 'smoke.test') {
      response.writeHead(404).end();
      return;
    }
    try {
      if (request.url === '/probe') {
        response.writeHead(200, { 'content-type': 'application/json' });
        response.end(JSON.stringify(coordinator.executeUnary()));
        return;
      }
      if (request.url === '/stream') {
        const pin = coordinator.openPinnedStream();
        response.writeHead(200, { 'content-type': 'application/x-ndjson' });
        response.write(`${JSON.stringify({ phase: 'start', result: pin.result })}\n`);
        await pin.released;
        response.end(`${JSON.stringify({ phase: 'end', result: pin.result })}\n`);
        return;
      }
      response.writeHead(404).end();
    } catch (error) {
      response.writeHead(503).end(String(error));
    }
  });
  return { port: await listen(server), close: () => closeServer(server) };
}

function hostRequest(port) {
  return requestText(port, 'POST', '/probe');
}

function openHostStream(port) {
  let open = true;
  let completeResolve;
  let completeReject;
  const completion = new Promise((resolve, reject) => {
    completeResolve = resolve;
    completeReject = reject;
  });
  completion.catch(() => undefined);
  return new Promise((resolve, reject) => {
    const chunks = [];
    const request = http.request(
      { host: '127.0.0.1', port, path: '/stream', method: 'GET', headers: { host: 'smoke.test' } },
      (response) => {
        response.on('data', (chunk) => {
          chunks.push(chunk);
          if (chunks.length === 1) {
            resolve({
              firstChunk: chunk.toString('utf8'),
              isOpen: () => open,
              completion,
            });
          }
        });
        response.on('end', () => {
          open = false;
          completeResolve(Buffer.concat(chunks).toString('utf8'));
        });
        response.on('error', (error) => {
          open = false;
          reject(error);
          completeReject(error);
        });
      },
    );
    request.once('error', (error) => {
      open = false;
      reject(error);
      completeReject(error);
    });
    request.end();
  });
}

function requestText(port, method, requestPath) {
  return new Promise((resolve, reject) => {
    const request = http.request(
      {
        host: '127.0.0.1',
        port,
        path: requestPath,
        method,
        headers: { host: 'smoke.test' },
      },
      (response) => {
        const chunks = [];
        response.on('data', (chunk) => chunks.push(chunk));
        response.on('end', () => {
          const body = Buffer.concat(chunks).toString('utf8');
          if ((response.statusCode ?? 500) >= 400) reject(new Error(body));
          else resolve(body);
        });
      },
    );
    request.once('error', reject);
    request.end();
  });
}

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject);
      resolve(server.address().port);
    });
  });
}

function closeServer(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}
