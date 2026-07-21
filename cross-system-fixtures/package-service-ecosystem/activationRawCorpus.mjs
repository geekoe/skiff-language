import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

export async function loadActivationRawCases() {
  return JSON.parse(
    await readFile(new URL("activation-raw-cases.json", import.meta.url), "utf8"),
  );
}

export function rawActivationInput(rawCase) {
  assert.equal(
    Object.hasOwn(rawCase, "text") === Object.hasOwn(rawCase, "bytesHex"),
    false,
    `${rawCase.name} must have exactly one raw input`,
  );
  if (Object.hasOwn(rawCase, "text")) return rawCase.text;
  assert.match(rawCase.bytesHex, /^(?:[0-9a-f]{2})+$/);
  return Buffer.from(rawCase.bytesHex, "hex");
}
