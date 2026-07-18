import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const checker = join(repoRoot, 'scripts', 'check-compiler-boundaries.mjs');

test('terminal package and code-free contract producers remain permitted', async () => {
  await withFixture(async (root) => {
    await write(
      root,
      'compiler/driver/lib.rs',
      `pub fn compile_package() {}
pub fn compile_contract() {}
`,
    );
    await write(
      root,
      'compiler/contract/src/lib.rs',
      `pub fn compile_service_contract_definition() {}
`,
    );

    const result = await runChecker(['--root', root]);
    assert.equal(result.code, 0, result.stderr);
    assert.match(result.stdout, /passed with no known violations/);
  });
});

test('terminal compiler shape rejects legacy publication, unit, facade, adapter, and provider paths', async () => {
  await withFixture(async (root) => {
    await write(
      root,
      'compiler/input/src/lib.rs',
      `pub struct PublicationInput;
pub enum PublicationInputKind { Package }
`,
    );
    await write(
      root,
      'compiler/input-model/src/lib.rs',
      `pub enum PublicationKind { Package }
`,
    );
    await write(
      root,
      'compiler/future-terminal/src/lib.rs',
      'pub struct CompatibilityCompilerFacade;\n',
    );
    await write(
      root,
      'compiler/driver/lib.rs',
      `pub struct CompiledPublication;
pub struct LoweredPublication;
pub struct PublicationAbiUnit;
pub struct PackageUnit;
pub struct ServiceUnit;
pub fn service_assembly() {}
pub fn compile_service_publication() {}
pub struct RawServicePublicationJob;
pub struct LegacyPublicationAdapter;
pub fn compatibility_output_adapter() {}
pub fn infer_provider() {}
pub struct ProviderInference;
`,
    );

    const result = await runChecker(['--root', root]);
    assert.notEqual(result.code, 0, result.stdout);
    assert.equal((result.stderr.match(/^DENY /gm) ?? []).length, 16, result.stderr);
    assert.match(result.stderr, /terminal_compiler_shape_no_legacy_publication_or_provider_paths/);
    assert.match(result.stderr, /PublicationInput/);
    assert.match(result.stderr, /compiler\/input-model\/src\/lib\.rs/);
    assert.match(result.stderr, /PublicationAbiUnit/);
    assert.match(result.stderr, /PackageUnit/);
    assert.match(result.stderr, /service_assembly/);
    assert.match(result.stderr, /ServicePublication/);
    assert.match(result.stderr, /LegacyPublicationAdapter/);
    assert.match(result.stderr, /compiler\/future-terminal\/src\/lib\.rs/);
    assert.match(result.stderr, /compatibility_output_adapter/);
    assert.match(result.stderr, /infer_provider/);
  });
});

test('terminal compiler shape ignores test-only support files', async () => {
  await withFixture(async (root) => {
    await write(root, 'compiler/driver/lib.rs', 'pub fn compile_package() {}\n');
    await write(root, 'compiler/driver/test_support.rs', 'pub struct PublicationInput;\n');
    await write(
      root,
      'compiler/input/src/test_support/legacy_fixture.rs',
      'pub struct ServiceUnit;\n',
    );
    await write(root, 'compiler/core/src/tests.rs', 'pub fn infer_provider() {}\n');

    const result = await runChecker(['--root', root]);
    assert.equal(result.code, 0, result.stderr);
    assert.match(result.stdout, /passed with no known violations/);
  });
});

test('projection-input permits arbitrary DTO constructors, getters, and builders', async () => {
  await withFixture(async (root) => {
    await write(
      root,
      'compiler/projection-input/src/lib.rs',
      `pub struct ResourceDto;

impl ResourceDto {
    pub fn construct_resource() -> Self { Self }
    pub fn checksum_bytes(&self) -> u64 { 0 }
    pub fn replacing_metadata(self) -> Self { self }
    #[must_use]
    pub const fn qualified_getter(&self) -> u64 { 0 }
}
`,
    );

    const result = await runChecker(['--root', root]);
    assert.equal(result.code, 0, result.stderr);
    assert.match(result.stdout, /passed with no known violations/);
    assert.doesNotMatch(`${result.stdout}\n${result.stderr}`, /DENY/);
  });
});

test('projection-input rejects only an exact known behavior receiver and method pair', async () => {
  await withFixture(async (root) => {
    await write(
      root,
      'compiler/projection-input/src/lib.rs',
      `pub struct ProjectionSourceFacts;
pub struct HarmlessDto;

impl ProjectionSourceFacts {
    pub fn derive_projection_abi_ids(&self) {}
}

impl HarmlessDto {
    pub fn derive_projection_abi_ids(&self) {}
}
`,
    );

    const result = await runChecker(['--root', root]);
    assert.notEqual(result.code, 0, result.stdout);
    assert.match(result.stderr, /compiler\/projection-input\/src\/lib\.rs:5/);
    assert.match(result.stderr, /projection_input_pure_dto_api_phase_7_5/);
    assert.match(result.stderr, /known non-DTO public behavior/);
    assert.equal((result.stderr.match(/^DENY /gm) ?? []).length, 1, result.stderr);
  });
});

test('projection-input rejects indented and qualified public free functions', async () => {
  await withFixture(async (root) => {
    await write(
      root,
      'compiler/projection-input/src/lib.rs',
      `mod nested {
  pub fn plain() {}
  #[must_use]
  pub async fn asynchronous() {}
  pub const fn constant() {}
  pub unsafe fn unsafe_behavior() {}
  #[allow(improper_ctypes_definitions)] pub extern "C" fn external() {}
}
`,
    );

    const result = await runChecker(['--root', root]);
    assert.notEqual(result.code, 0, result.stdout);
    assert.equal((result.stderr.match(/^DENY /gm) ?? []).length, 5, result.stderr);
    assert.equal(
      (result.stderr.match(/pattern="public free functions"/g) ?? []).length,
      5,
      result.stderr,
    );
    assert.match(result.stderr, /pub async fn asynchronous/);
    assert.match(result.stderr, /pub const fn constant/);
    assert.match(result.stderr, /pub unsafe fn unsafe_behavior/);
    assert.match(result.stderr, /pub extern "C" fn external/);
  });
});

test('compiler-core artifact identity import still fails closed', async () => {
  await withFixture(async (root) => {
    await write(
      root,
      'compiler/core/src/lib.rs',
      'use skiff_artifact_identity::file_ir_identity;\n',
    );

    const result = await runChecker(['--root', root]);
    assert.notEqual(result.code, 0, result.stdout);
    assert.match(result.stderr, /compiler_core_no_forbidden_imports/);
    assert.match(result.stderr, /skiff_artifact_identity/);
  });
});

test('compiler boundary CLI rejects invalid arguments and missing roots', async () => {
  const invalid = await runChecker(['--unknown']);
  assert.notEqual(invalid.code, 0);
  assert.match(invalid.stderr, /unknown argument --unknown/);

  const missing = join(tmpdir(), `missing-skiff-boundary-${process.pid}-${Date.now()}`);
  const missingResult = await runChecker(['--root', missing]);
  assert.notEqual(missingResult.code, 0);
  assert.match(missingResult.stderr, /compiler boundary root does not exist/);
});

async function withFixture(run) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-compiler-boundary-'));
  try {
    await run(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

async function write(root, relativePath, contents) {
  const path = join(root, relativePath);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, contents);
}

function runChecker(args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, [checker, ...args], { cwd: repoRoot });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.once('error', reject);
    child.once('close', (code, signal) => {
      resolvePromise({ code, signal, stdout, stderr });
    });
  });
}
