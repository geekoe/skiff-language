import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import {
  ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT,
  decodeAssemblyActivationControl,
  decodeAssemblyActivationControls,
  decodeAssemblyActivationRequest,
  decodeEnvironmentActivationState,
} from "../../router/src/protocol/assemblyActivationProtocol.ts";

if (process.argv.length !== 3 || process.argv[2] !== "--self-test") {
  throw new Error("usage: node verify.mjs --self-test");
}

const fixtureRoot = new URL("./", import.meta.url);
const requestWire = await readJson("activation-request.json");
const stateWire = await readJson("activation-state.json");
const controlWire = await readJson("control-wire.json");
const mutations = await readJson("activation-mutations.json");
const checkpoint = await readJson("checkpoint.json");

for (const fixture of ["request", "state", "control"]) {
  const overlong = mutations[fixture].find((mutation) =>
    mutation.name.startsWith("overlong"),
  );
  const token = Array.isArray(overlong.value) ? overlong.value[0] : overlong.value;
  assert.equal(Buffer.byteLength(token, "utf8"), 201);
}

const request = decodeAssemblyActivationRequest(requestWire);
assert.equal(request.expectedGeneration, 41);
assert.deepEqual(request, requestWire);
assert.deepEqual(Object.keys(request), [
  "schemaVersion",
  "environment",
  "activationId",
  "expectedGeneration",
  "assembly",
]);

const state = decodeEnvironmentActivationState(stateWire);
assert.equal(state.committed.generation, 41);
assert.equal(state.pending?.candidateGeneration, 42);
assert.deepEqual(state.pending?.participantReplicaIds, ["runtime-a", "runtime-b"]);
assert.deepEqual(state, stateWire);

const controls = decodeAssemblyActivationControls(controlWire);
assert.deepEqual(controls, controlWire);
assert.deepEqual(
  controls.map((message) => message.type),
  ["prepare", "prepared", "reject", "commit", "abort", "register"],
);

const decoderByFixture = {
  request: decodeAssemblyActivationRequest,
  state: decodeEnvironmentActivationState,
  control: decodeAssemblyActivationControl,
};
const goldenByFixture = {
  request: requestWire,
  state: stateWire,
  control: controlWire[0],
};
let mutationCount = 0;
for (const fixture of ["request", "state", "control"]) {
  for (const mutation of mutations[fixture]) {
    assert.throws(
      () => decoderByFixture[fixture](applyMutation(goldenByFixture[fixture], mutation)),
      undefined,
      `${fixture} mutation ${mutation.name} must fail closed`,
    );
    mutationCount += 1;
  }
}

assert.deepEqual(checkpoint.authoringFields["package.yml contracts[]"], [
  "alias",
  "serviceId",
  "contractVersion",
]);
assert.deepEqual(checkpoint.authoringFields["assembly.yml"], [
  "environment",
  "rootDeployments",
]);
assert.deepEqual(checkpoint.activationStateFields.committed, [
  "generation",
  "assembly",
]);
assert.deepEqual(checkpoint.coordinateEncoding, {
  ".": "~d",
  "/": "~s",
  "~": "rejected",
});
assert.deepEqual(checkpoint.coordinateExamples, {
  "a.b/c/d": "a~db~sc~sd",
  "a.b/c..d": "a~db~sc~d~dd",
});
assert.notEqual(
  checkpoint.coordinateExamples["a.b/c/d"],
  checkpoint.coordinateExamples["a.b/c..d"],
);
assert.deepEqual(checkpoint.activationRequest.fields, [
  "schemaVersion",
  "environment",
  "activationId",
  "expectedGeneration",
  "assembly",
]);
assert.equal(
  checkpoint.activationRequest.controlEndpoint,
  ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT,
);
assert.equal(
  checkpoint.pointerPaths.EnvironmentActivationState,
  "environments/<environment>/activation.json",
);
assert.equal(
  Object.values(checkpoint.recordPaths).some((path) =>
    /serviceAssembly|packageUnit|artifactRoots/.test(path),
  ),
  false,
);

process.stdout.write(
  `${JSON.stringify({
    ok: true,
    fixture: fileURLToPath(
      new URL("activation-request.json", fixtureRoot),
    ),
    controls: controls.length,
    mutations: mutationCount,
  })}\n`,
);

async function readJson(name) {
  return JSON.parse(await readFile(new URL(name, fixtureRoot), "utf8"));
}

function applyMutation(base, mutation) {
  const candidate = structuredClone(base);
  const path = mutation.path;
  assert.ok(path.length > 0, "mutation path must not be empty");
  let parent = candidate;
  for (const segment of path.slice(0, -1)) {
    assert.equal(typeof parent, "object");
    assert.notEqual(parent, null);
    parent = parent[segment];
  }
  const field = path.at(-1);
  if (mutation.operation === "replace") {
    assert.ok(Object.hasOwn(parent, field), "replace path must exist");
    parent[field] = mutation.value;
  } else if (mutation.operation === "remove") {
    assert.ok(Object.hasOwn(parent, field), "remove path must exist");
    delete parent[field];
  } else if (mutation.operation === "add") {
    assert.equal(Object.hasOwn(parent, field), false, "add path must be new");
    parent[field] = mutation.value;
  } else {
    throw new Error(`unknown mutation operation ${mutation.operation}`);
  }
  return candidate;
}
