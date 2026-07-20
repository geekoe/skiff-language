import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import {
  decodeAssemblyActivationControl,
  decodeAssemblyActivationControls,
} from "../../router/src/protocol/assemblyActivationProtocol.ts";

if (process.argv.length !== 3 || process.argv[2] !== "--self-test") {
  throw new Error("usage: node verify.mjs --self-test");
}

const fixtureRoot = new URL("./", import.meta.url);
const controlWire = JSON.parse(
  await readFile(new URL("control-wire.json", fixtureRoot), "utf8"),
);
const checkpoint = JSON.parse(
  await readFile(new URL("checkpoint.json", fixtureRoot), "utf8"),
);

const decoded = decodeAssemblyActivationControls(controlWire);
assert.deepEqual(
  decoded.map((message) => message.type),
  ["prepare", "prepared", "reject", "commit", "abort", "register"],
);

for (const forbidden of [
  "artifactRoots",
  "serviceConfig",
  "serviceId",
  "buildId",
  "target",
]) {
  for (const message of controlWire) {
    assert.throws(
      () => decodeAssemblyActivationControl({ ...message, [forbidden]: "legacy" }),
      undefined,
      `${message.type} must reject ${forbidden}`,
    );
  }
}

for (const message of controlWire) {
  const required = Object.keys(message).find((field) => field !== "type");
  const missing = { ...message };
  delete missing[required];
  assert.throws(() => decodeAssemblyActivationControl(missing));
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
  ".": "~",
  "/": "~~",
  "~": "rejected",
});
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
    fixture: fileURLToPath(new URL("control-wire.json", fixtureRoot)),
    messages: decoded.length,
    legacyMutations: 5 * decoded.length,
  })}\n`,
);
