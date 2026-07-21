import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { registerHooks } from "node:module";
import { fileURLToPath } from "node:url";

import {
  loadActivationRawCases,
  rawActivationInput,
} from "./activationRawCorpus.mjs";

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (
      specifier.startsWith(".") &&
      specifier.endsWith(".js") &&
      context.parentURL?.startsWith("file:")
    ) {
      const sourceUrl = new URL(`${specifier.slice(0, -3)}.ts`, context.parentURL);
      if (existsSync(sourceUrl)) return nextResolve(sourceUrl.href, context);
    }
    return nextResolve(specifier, context);
  },
});

const {
  ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT,
  decodeAssemblyActivationControl,
  decodeAssemblyActivationControls,
  decodeAssemblyActivationRequest,
  decodeEnvironmentActivationState,
} = await import("../../router/src/protocol/assemblyActivationProtocol.ts");
const {
  decodeRawAssemblyActivationControl,
  decodeRawAssemblyActivationRequest,
  decodeRawEnvironmentActivationState,
} = await import("../../router/src/protocol/assemblyActivationRawCodec.ts");
const {
  decodeAssemblyActivationFrame,
  encodeAssemblyActivationFrame,
} = await import("../../router/src/protocol/assemblyActivationFrame.ts");
const {
  validateRouterToRuntimeFrameHeader,
  validateRuntimeAssemblyRequestStartFrameHeader,
  validateRuntimeToRouterFrameHeader,
} = await import("../../router/src/protocol/runtimeProtocol.ts");

const mode = process.argv[2];
if (
  process.argv.length !== 3 ||
  (mode !== "--self-test" &&
    mode !== "--combined-probe" &&
    mode !== "--runtime-wire-self-test")
) {
  throw new Error(
    "usage: node verify.mjs <--self-test|--combined-probe|--runtime-wire-self-test>",
  );
}

const fixtureRoot = new URL("./", import.meta.url);
const requestWire = await readJson("activation-request.json");
const stateWire = await readJson("activation-state.json");
const controlWire = await readJson("control-wire.json");
const rawCases = await loadActivationRawCases();
const checkpoint = await readJson("checkpoint.json");

if (mode === "--runtime-wire-self-test") {
  await runRuntimeWireSelfTest(controlWire, checkpoint);
  process.exit(0);
}

if (mode === "--combined-probe") {
  runCombinedProbe(rawCases, requestWire, stateWire);
  process.stdout.write(
    `${JSON.stringify({ ok: true, probe: "activation-parity" })}\n`,
  );
  process.exit(0);
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

const typedDecoderByTarget = {
  request: decodeAssemblyActivationRequest,
  state: decodeEnvironmentActivationState,
  control: decodeAssemblyActivationControl,
};
const productionRawDecoderByTarget = {
  request: decodeRawAssemblyActivationRequest,
  state: decodeRawEnvironmentActivationState,
  control: decodeRawAssemblyActivationControl,
};
const seenNames = new Set();
for (const rawCase of rawCases) {
  assert.equal(seenNames.has(rawCase.name), false, `duplicate case ${rawCase.name}`);
  seenNames.add(rawCase.name);
  assert.ok(rawCase.target in productionRawDecoderByTarget, rawCase.name);
  assert.ok(rawCase.outcome === "accept" || rawCase.outcome === "reject");
  const decode = productionRawDecoderByTarget[rawCase.target];
  if (rawCase.outcome === "accept") {
    assert.doesNotThrow(() => decode(rawActivationInput(rawCase)), rawCase.name);
  } else {
    assert.throws(() => decode(rawActivationInput(rawCase)), undefined, rawCase.name);
  }
}
assert.ok(rawCases.length >= 50, "raw corpus must stay exhaustive");

const negativeZeroRequest = structuredClone(requestWire);
negativeZeroRequest.expectedGeneration = -0;
assert.ok(Object.is(negativeZeroRequest.expectedGeneration, -0));
assert.throws(
  () => typedDecoderByTarget.request(negativeZeroRequest),
  undefined,
  "typed request decoder must reject Object.is(-0)",
);

const sparseState = structuredClone(stateWire);
const sparseParticipants = new Array(3);
sparseParticipants[0] = "runtime-a";
sparseParticipants[2] = "runtime-b";
sparseState.pending.participantReplicaIds = sparseParticipants;
assert.throws(
  () => typedDecoderByTarget.state(sparseState),
  undefined,
  "typed state decoder must reject sparse participant arrays",
);

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
    rawCases: rawCases.length,
  })}\n`,
);

async function readJson(name) {
  return JSON.parse(await readFile(new URL(name, fixtureRoot), "utf8"));
}

function runCombinedProbe(cases, typedRequest, typedState) {
  const requiredRejects = [
    "token FEFF",
    "duplicate request top key",
    "generation rounding",
    "state missing pending",
  ];
  for (const name of requiredRejects) {
    const rawCase = cases.find((candidate) => candidate.name === name);
    assert.ok(rawCase, `combined probe case ${name}`);
    assert.equal(rawCase.outcome, "reject");
    const decode = productionRawDecoder(rawCase.target);
    assert.throws(() => decode(rawActivationInput(rawCase)), undefined, name);
  }

  const negativeZero = structuredClone(typedRequest);
  negativeZero.expectedGeneration = -0;
  assert.throws(() => decodeAssemblyActivationRequest(negativeZero));

  const sparseState = structuredClone(typedState);
  const sparseParticipants = new Array(2);
  sparseParticipants[1] = "runtime-a";
  sparseState.pending.participantReplicaIds = sparseParticipants;
  assert.throws(() => decodeEnvironmentActivationState(sparseState));
}

function productionRawDecoder(target) {
  if (target === "request") return decodeRawAssemblyActivationRequest;
  if (target === "state") return decodeRawEnvironmentActivationState;
  if (target === "control") return decodeRawAssemblyActivationControl;
  throw new Error(`unknown raw activation target ${target}`);
}

async function runRuntimeWireSelfTest(controlMessages, frozenCheckpoint) {
  const frameCorpus = await readJson("runtime-wire.json");
  const requestCorpus = await readJson("runtime-request-wire.json");
  const storeCorpus = await readJson("ecosystem-store-cases.json");

  assert.equal(frameCorpus.assemblyActivationFrames.length, controlMessages.length);
  for (const golden of frameCorpus.assemblyActivationFrames) {
    const control = controlMessages[golden.controlIndex];
    assert.equal(control.type, golden.name);
    const encoded = encodeAssemblyActivationFrame(golden.direction, control);
    assert.equal(encoded.toString("hex"), golden.frameHex, golden.name);
    assert.deepEqual(
      decodeAssemblyActivationFrame(
        golden.direction,
        Buffer.from(golden.frameHex, "hex"),
      ),
      control,
      golden.name,
    );
    const reverseDirection =
      golden.direction === "routerToRuntime"
        ? "runtimeToRouter"
        : "routerToRuntime";
    assert.throws(
      () => encodeAssemblyActivationFrame(reverseDirection, control),
      undefined,
      `${golden.name} reverse encode direction`,
    );
    assert.throws(
      () =>
        decodeAssemblyActivationFrame(
          reverseDirection,
          Buffer.from(golden.frameHex, "hex"),
        ),
      undefined,
      `${golden.name} reverse decode direction`,
    );
  }
  for (const mutation of frameCorpus.assemblyActivationMutations) {
    assert.match(mutation.input, /^(?:[0-9a-f]{2})+$/);
    assert.throws(
      () =>
        decodeAssemblyActivationFrame(
          mutation.direction,
          Buffer.from(mutation.input, "hex"),
        ),
      undefined,
      mutation.name,
    );
  }

  for (const header of requestCorpus.requestStartHeaders) {
    const result = validateRouterToRuntimeFrameHeader(header);
    assert.equal(result.ok, true, result.ok ? header.requestId : result.error);
    const typed = validateRuntimeAssemblyRequestStartFrameHeader(header);
    assert.equal(typed.ok, true, typed.ok ? header.requestId : typed.error);
    assert.deepEqual(typed.envelope, header);
    assert.equal(
      validateRuntimeToRouterFrameHeader(header).ok,
      false,
      `${header.requestId} direction`,
    );
  }
  for (const mutation of requestCorpus.requestStartMutations) {
    const header = structuredClone(
      requestCorpus.requestStartHeaders[mutation.baseIndex],
    );
    applyMutation(header, mutation);
    const result = validateRouterToRuntimeFrameHeader(header);
    assert.equal(result.ok, false, mutation.name);
    assert.equal(
      validateRuntimeAssemblyRequestStartFrameHeader(header).ok,
      false,
      mutation.name,
    );
  }

  assert.deepEqual(frozenCheckpoint.runtimeAssemblyFrame.routerToRuntime, [
    "prepare",
    "commit",
    "abort",
  ]);
  assert.deepEqual(frozenCheckpoint.runtimeAssemblyFrame.runtimeToRouter, [
    "prepared",
    "reject",
    "register",
  ]);
  assert.deepEqual(
    storeCorpus.argv,
    frozenCheckpoint.ecosystemStoreAdapter.argv,
  );
  assert.deepEqual(
    storeCorpus.workflow.map((request) => request.operation),
    [
      "ensureEnvironmentBootstrap",
      "readEnvironment",
      "prepareEnvironment",
      "abortEnvironment",
      "prepareEnvironment",
      "commitEnvironment",
      "ensureEnvironmentBootstrap",
      "readRouterSnapshot",
    ],
  );
  assert.ok(storeCorpus.invalidRequests.length >= 5);
  assert.equal(
    Object.values(storeCorpus).some((value) =>
      JSON.stringify(value).includes('"latest"'),
    ),
    false,
  );

  process.stdout.write(
    `${JSON.stringify({
      ok: true,
      probe: "runtime-wire",
      activationFrames: frameCorpus.assemblyActivationFrames.length,
      activationMutations: frameCorpus.assemblyActivationMutations.length,
      requestHeaders: requestCorpus.requestStartHeaders.length,
      requestMutations: requestCorpus.requestStartMutations.length,
      storeOperations: frozenCheckpoint.ecosystemStoreAdapter.operations.length,
    })}\n`,
  );
}

function applyMutation(root, mutation) {
  const path = mutation.setPath ?? mutation.removePath;
  const segments = path.split(".");
  const leaf = segments.pop();
  let owner = root;
  for (const segment of segments) owner = owner[segment];
  if (mutation.removePath !== undefined) {
    delete owner[leaf];
  } else {
    owner[leaf] = mutation.value;
  }
}
