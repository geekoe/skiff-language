import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import textmate from "vscode-textmate";
import oniguruma from "vscode-oniguruma";

const { Registry } = textmate;
const { loadWASM, OnigScanner, OnigString } = oniguruma;

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, "..");
const grammarPath = path.join(rootDir, "syntaxes", "skiff.tmLanguage.json");
const wasmPath = path.join(rootDir, "node_modules", "vscode-oniguruma", "release", "onig.wasm");

async function createRegistry() {
  const wasm = await fs.readFile(wasmPath);
  await loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));

  return new Registry({
    onigLib: Promise.resolve({
      createOnigScanner(patterns) {
        return new OnigScanner(patterns);
      },
      createOnigString(text) {
        return new OnigString(text);
      }
    }),
    loadGrammar: async (scopeName) => {
      if (scopeName !== "source.skiff") {
        return null;
      }

      const rawGrammar = JSON.parse(await fs.readFile(grammarPath, "utf8"));
      return rawGrammar;
    }
  });
}

function scopesForLine(ruleStack, grammar, line) {
  const result = grammar.tokenizeLine(line, ruleStack);
  return {
    ruleStack: result.ruleStack,
    tokens: result.tokens.map((token) => ({
      text: line.slice(token.startIndex, token.endIndex),
      scopes: token.scopes
    }))
  };
}

function findToken(tokens, text) {
  const token = tokens.find((entry) => entry.text === text);
  if (!token) {
    throw new Error(`Missing token "${text}"`);
  }

  return token;
}

function findTokenWithScope(tokens, text, expectedScope) {
  const token = tokens.find((entry) => entry.text === text && entry.scopes.includes(expectedScope));
  if (!token) {
    const candidates = tokens
      .filter((entry) => entry.text === text)
      .map((entry) => entry.scopes.join(", "))
      .join("\n");
    throw new Error(`Missing token "${text}" with scope "${expectedScope}". Candidates:\n${candidates}`);
  }

  return token;
}

function expectScope(token, expectedScope) {
  if (!token.scopes.includes(expectedScope)) {
    throw new Error(`Token "${token.text}" missing scope "${expectedScope}". Got: ${token.scopes.join(", ")}`);
  }
}

function expectNoScope(token, unexpectedScope) {
  if (token.scopes.includes(unexpectedScope)) {
    throw new Error(`Token "${token.text}" unexpectedly had scope "${unexpectedScope}". Got: ${token.scopes.join(", ")}`);
  }
}

async function main() {
  const registry = await createRegistry();
  const grammar = await registry.loadGrammar("source.skiff");

  if (!grammar) {
    throw new Error("Unable to load Skiff grammar");
  }

  let ruleStack = null;

  const sampleLines = [
    "import std",
    "import mongo",
    "provider mongo",
    "export interface RawHttp {",
    "  function handle(request: HttpRequest) -> HttpResponse",
    "}",
    'export alias Role = "admin" | "user"',
    "type Service implements root.api.raw_http.RawHttp {",
    "  retries: integer,",
    "  active: bool,",
    "  legacyFlag: boolean,",
    "}",
    "export impl Service {",
    "  native static function empty() -> Service",
    "  provider function save<T>(doc: T) -> bool",
    "  function handle(request: HttpRequest) -> HttpResponse {",
    "    /* timeout block modifier */",
    "    const headers: Map<string, string> = Map.empty<string, string>()",
    "    let active = true",
    "    const mapper: fn(item: string) -> string = fn(item: string) -> string { return item }",
    "    timeout(200ms) {",
    '      emit({ tag: "tick" })',
    "    }",
    '    const filter = mongo.Filter<root.app.prompt_model.PromptDocument> { ["$or"]: filters, [dynamicKey]: "value" }',
    "    for header in request.headers {",
    '      if active && header.name == "x-debug" {',
    "        break",
    "      } else {",
    "        continue",
    "      }",
    "    }",
    '    const result = catch<Exception<ErrorPayload>>(throw ErrorPayload { code: "x" })',
    '    const decoded = catch<Exception<std.json.DecodeError>>(timeout(200ms) value { throw std.json.DecodeError { target: "x", message: "bad" } })',
    "    rethrow result",
    '    return HttpResponse { status: number.assertSafeInteger(200), headers: headers, body: bytes.fromUtf8("ok") }',
    "  }",
    "}",
    "test defaultRun false",
    'test "handles request" {',
    "  assert true",
    "}"
  ];

  const collected = [];
  for (const line of sampleLines) {
    const tokenized = scopesForLine(ruleStack, grammar, line);
    ruleStack = tokenized.ruleStack;
    collected.push(...tokenized.tokens);
  }

  expectScope(findToken(collected, "import"), "keyword.control.import.skiff");
  expectScope(findToken(collected, "std"), "entity.name.namespace.skiff");
  expectScope(findTokenWithScope(collected, "mongo", "entity.name.namespace.skiff"), "entity.name.namespace.skiff");
  expectScope(findTokenWithScope(collected, "provider", "keyword.other.provider.skiff"), "keyword.other.provider.skiff");
  expectScope(findTokenWithScope(collected, "mongo", "entity.name.namespace.provider.skiff"), "entity.name.namespace.provider.skiff");
  expectScope(findToken(collected, "export"), "keyword.other.modifier.skiff");
  expectScope(findToken(collected, "interface"), "storage.type.interface.skiff");
  expectScope(findToken(collected, "RawHttp"), "entity.name.type.interface.skiff");
  expectScope(findToken(collected, "function"), "storage.type.function.skiff");
  expectScope(findTokenWithScope(collected, "request", "variable.parameter.skiff"), "variable.parameter.skiff");
  expectScope(findToken(collected, "HttpRequest"), "support.type.core.skiff");
  expectScope(findToken(collected, "alias"), "storage.type.alias.skiff");
  expectScope(findToken(collected, "Role"), "entity.name.type.alias.skiff");
  expectScope(findToken(collected, "type"), "storage.type.skiff");
  expectScope(findToken(collected, "Service"), "entity.name.type.skiff");
  expectScope(findToken(collected, "implements"), "keyword.operator.context.skiff");
  expectScope(findToken(collected, "root"), "support.variable.root.skiff");
  expectScope(findToken(collected, "integer"), "support.type.primitive.skiff");
  expectScope(findToken(collected, "bool"), "support.type.primitive.skiff");
  expectScope(findToken(collected, "boolean"), "support.type.primitive.legacy.skiff");
  expectScope(findToken(collected, "impl"), "storage.type.impl.skiff");
  expectScope(findToken(collected, "native"), "keyword.other.modifier.skiff");
  expectScope(findToken(collected, "static"), "keyword.other.modifier.skiff");
  expectScope(findTokenWithScope(collected, "provider", "keyword.other.modifier.skiff"), "keyword.other.modifier.skiff");
  expectScope(findToken(collected, " timeout block modifier "), "comment.block.skiff");
  expectScope(findToken(collected, "const"), "storage.type.variable.skiff");
  expectScope(findToken(collected, "headers"), "variable.other.readwrite.skiff");
  expectScope(findToken(collected, "Map"), "support.type.core.skiff");
  expectScope(findToken(collected, "let"), "storage.type.variable.skiff");
  expectScope(findToken(collected, "fn"), "storage.type.function.skiff");
  expectScope(findToken(collected, "timeout"), "keyword.control.timeout.skiff");
  expectScope(findToken(collected, "200ms"), "constant.numeric.duration.skiff");
  expectScope(findTokenWithScope(collected, "Filter", "entity.name.type.qualified.skiff"), "entity.name.type.qualified.skiff");
  expectScope(findTokenWithScope(collected, "$or", "string.quoted.double.skiff"), "string.quoted.double.skiff");
  expectScope(findTokenWithScope(collected, "dynamicKey", "variable.other.member.computed-key.skiff"), "variable.other.member.computed-key.skiff");
  expectScope(findToken(collected, "emit"), "keyword.control.flow.skiff");
  expectScope(findToken(collected, "for"), "keyword.control.loop.skiff");
  expectScope(findToken(collected, "in"), "keyword.operator.context.skiff");
  expectScope(findToken(collected, "if"), "keyword.control.conditional.skiff");
  expectScope(findToken(collected, "else"), "keyword.control.conditional.skiff");
  expectScope(findToken(collected, "catch"), "keyword.control.catch.skiff");
  expectScope(findToken(collected, "Exception"), "support.type.core.skiff");
  expectScope(findTokenWithScope(collected, "DecodeError", "entity.name.type.qualified.skiff"), "entity.name.type.qualified.skiff");
  expectScope(findToken(collected, "throw"), "keyword.control.flow.skiff");
  expectScope(findToken(collected, "rethrow"), "keyword.control.flow.skiff");
  expectScope(findToken(collected, "return"), "keyword.control.flow.skiff");
  expectScope(findTokenWithScope(collected, "status", "variable.other.member.field.skiff"), "variable.other.member.field.skiff");
  expectScope(findToken(collected, "number"), "support.type.primitive.skiff");
  expectScope(findToken(collected, "assertSafeInteger"), "variable.other.property.skiff");
  expectScope(findToken(collected, "bytes"), "support.type.primitive.skiff");
  expectScope(findToken(collected, "fromUtf8"), "variable.other.property.skiff");
  expectScope(findToken(collected, "test"), "keyword.other.test.skiff");
  expectScope(findToken(collected, "defaultRun"), "support.variable.test.directive.skiff");
  expectScope(findToken(collected, "assert"), "keyword.other.test.skiff");

  for (const line of [
    "import std.http",
    "import std as standard",
    "import google.com/cloud",
    "import google.com/cloud as gcloud"
  ]) {
    const invalidImport = scopesForLine(null, grammar, line);
    for (const token of invalidImport.tokens) {
      expectNoScope(token, "entity.name.namespace.skiff");
    }
  }

  const obsoleteLines = [
    "yield item",
    "timeout[200ms]",
    'const text = "hello ${name}"'
  ];
  for (const line of obsoleteLines) {
    const obsolete = scopesForLine(null, grammar, line);
    for (const token of obsolete.tokens) {
      expectNoScope(token, "keyword.control.flow.skiff");
      expectNoScope(token, "keyword.control.timeout.skiff");
      expectNoScope(token, "meta.interpolation.skiff");
    }
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
