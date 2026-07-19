import {
  appendFile,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
  collectRuntimeArtifactBoundaryViolations,
  formatRuntimeArtifactBoundaryViolation,
} from './runtime-artifact-boundary-checker.mjs';
import {
  RUNTIME_ARTIFACT_BOUNDARY_SUBJECTS,
  RUNTIME_REQUEST_ENTRY_BOUNDARY_ROOT,
} from './runtime-artifact-boundary-subjects.mjs';

export async function runRuntimeArtifactBoundarySelfTest() {
  const matrix = [
    mutation(
      'renamed old DTO owner',
      'legacy-runtime-dto',
      async (root) => append(root, 'runtime/loader/src/runtime_assembly.rs', '\nstruct RenamedServiceUnitOwner;\n'),
    ),
    mutation(
      'moved canonical owner',
      'unregistered-canonical-owner',
      async (root) => {
        await write(root, 'runtime/model/src/keep.rs', 'pub struct Keep;\n');
        await rename(
          join(root, 'runtime/loader/src/runtime_assembly.rs'),
          join(root, 'runtime/model/src/moved_runtime_loader.rs'),
        );
      },
    ),
    mutation(
      'copied canonical owner',
      'duplicate-canonical-owner',
      async (root) => {
        await mkdir(join(root, 'runtime/model/src'), { recursive: true });
        await copyFile(
          join(root, 'runtime/loader/src/runtime_assembly.rs'),
          join(root, 'runtime/model/src/copied_runtime_loader.rs'),
        );
      },
    ),
    mutation(
      'helper facade wrapper',
      'compatibility-or-fallback-path',
      async (root) => {
        await append(root, 'runtime/linker/src/assembly.rs', '\nmod facade;\n');
        await write(
          root,
          'runtime/linker/src/assembly/facade.rs',
          'pub fn compatibility_fallback_facade() {}\n',
        );
      },
    ),
    mutation(
      'test-named production camouflage',
      'legacy-runtime-dto',
      async (root) => {
        const path = join(root, 'runtime/loader/src/runtime_assembly.rs');
        const source = await readFile(path, 'utf8');
        await writeFile(path, source.replace('#[cfg(test)]\nmod tests;', 'mod tests;'));
      },
    ),
    mutation(
      'test-support feature camouflage',
      'compatibility-or-fallback-path',
      async (root) => append(
        root,
        'runtime/linker/src/assembly.rs',
        '\n#[cfg(any(test, feature = "test-support"))]\nfn compatibility_fallback_for_tests() {}\n',
      ),
    ),
    mutation(
      'required subject registry omission',
      'subject-registry-omission',
      undefined,
      RUNTIME_ARTIFACT_BOUNDARY_SUBJECTS.filter(
        ({ id }) => id !== 'runtime-assembly-linker',
      ),
    ),
    mutation(
      'request-entry subject registry omission',
      'subject-registry-omission',
      undefined,
      RUNTIME_ARTIFACT_BOUNDARY_SUBJECTS.map((subject) =>
        subject.id === 'whole-assembly-host'
          ? {
              ...subject,
              ownedRoots: subject.ownedRoots.filter(
                (root) => root !== RUNTIME_REQUEST_ENTRY_BOUNDARY_ROOT,
              ),
            }
          : subject),
    ),
    mutation(
      'request-entry production file omission',
      'subject-root-missing',
      async (root) => rm(join(root, RUNTIME_REQUEST_ENTRY_BOUNDARY_ROOT)),
    ),
    mutation(
      'broad allowlist registry escape',
      'forbidden-subject-exception-field',
      undefined,
      RUNTIME_ARTIFACT_BOUNDARY_SUBJECTS.map((subject, index) =>
        index === 0 ? { ...subject, allowlist: ['runtime/**'] } : subject),
    ),
    mutation(
      'raw serviceAssembly wire',
      'raw-service-assembly-wire',
      async (root) => append(
        root,
        'runtime/loader/src/runtime_assembly.rs',
        '\nconst RAW_KEY: &str = "serviceAssembly";\n',
      ),
    ),
    mutation(
      'raw JSON semantic linker',
      'raw-json-semantic-linking',
      async (root) => append(
        root,
        'runtime/linker/src/assembly.rs',
        '\nfn link_raw(value: serde_json::Value) { drop(value); }\n',
      ),
    ),
    mutation(
      'display and source target inference',
      'display-or-source-linking',
      async (root) => append(
        root,
        'runtime/linked-program/src/shared_image.rs',
        '\nfn resolve_target_from_source_path(source_path: &str) { let _ = source_path; }\n',
      ),
    ),
    mutation(
      'request-time lazy artifact load',
      'request-time-lazy-load',
      async (root) => append(
        root,
        RUNTIME_REQUEST_ENTRY_BOUNDARY_ROOT,
        '\nfn lazy_load_request_service() {}\n',
      ),
    ),
    mutation(
      'dual-read compatibility path',
      'compatibility-or-fallback-path',
      async (root) => append(
        root,
        'runtime/host/src/loader/assembly_admission.rs',
        '\nfn dual_read_compat_adapter() {}\n',
      ),
    ),
    mutation(
      'provider executable patch',
      'provider-executable-patch',
      async (root) => append(
        root,
        'runtime/linked-program/src/shared_image.rs',
        '\nfn patch_provider_executable() {}\n',
      ),
    ),
  ];

  await withFixture(async (root) => {
    const baseline = await collectRuntimeArtifactBoundaryViolations(root);
    assertNoViolations('baseline with a genuine #[cfg(test)] external module', baseline);
  });

  for (const entry of matrix) {
    await withFixture(async (root) => {
      if (entry.mutate) {
        await entry.mutate(root);
      }
      const violations = await collectRuntimeArtifactBoundaryViolations(root, entry.subjects);
      if (!violations.some(({ id }) => id === entry.expectedId)) {
        throw new Error(
          `${entry.name}: expected ${entry.expectedId}; got\n${formatViolations(violations)}`,
        );
      }
    });
  }

  return matrix.map(({ name, expectedId }) => ({ name, expectedId }));
}

async function withFixture(run) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-runtime-artifact-boundary-'));
  try {
    await writeSafeFixture(root);
    await run(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

async function writeSafeFixture(root) {
  await Promise.all([
    write(
      root,
      'runtime/loader/src/runtime_assembly.rs',
      [
        'pub struct RuntimeAssemblyLoader;',
        'pub struct HydratedRuntimeAssembly;',
        'mod content_validation;',
        '#[cfg(test)]',
        'mod tests;',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/loader/src/runtime_assembly/content_validation.rs',
      'pub fn validate_typed_content() {}\n',
    ),
    write(
      root,
      'runtime/loader/src/runtime_assembly/tests.rs',
      'use crate::ServiceUnit;\nfn test_only_legacy_fixture() {}\n',
    ),
    write(
      root,
      'runtime/linked-program/src/shared_image.rs',
      'pub struct SharedPackageLinkedImage;\n#[cfg(test)]\nmod tests;\n',
    ),
    write(
      root,
      'runtime/linked-program/src/shared_image/tests.rs',
      'struct PackageUnit;\n',
    ),
    write(
      root,
      'runtime/linker/src/assembly.rs',
      'pub struct AssemblyLinkedCandidate;\nmod candidate;\n',
    ),
    write(
      root,
      'runtime/linker/src/assembly/candidate.rs',
      'pub fn typed_candidate() {}\n',
    ),
    write(
      root,
      'runtime/host/src/loader/assembly_admission.rs',
      [
        'pub struct AssemblyAdmissionController;',
        'pub struct RuntimeHost;',
        'impl RuntimeHost {',
        '    pub fn admit_runtime_assembly(&self) {}',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      RUNTIME_REQUEST_ENTRY_BOUNDARY_ROOT,
      'impl RuntimeHost { async fn spawn_request(&self) {} }\n',
    ),
    write(
      root,
      'runtime/eval/src/assembly_consumer.rs',
      'fn consume(candidate: &AssemblyLinkedCandidate) { let _ = candidate; }\n',
    ),
  ]);
}

async function write(root, relPath, contents) {
  const path = join(root, relPath);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, contents);
}

async function append(root, relPath, contents) {
  await appendFile(join(root, relPath), contents);
}

function mutation(name, expectedId, mutate, subjects = RUNTIME_ARTIFACT_BOUNDARY_SUBJECTS) {
  return { name, expectedId, mutate, subjects };
}

function assertNoViolations(label, violations) {
  if (violations.length > 0) {
    throw new Error(`${label}: expected no violations; got\n${formatViolations(violations)}`);
  }
}

function formatViolations(violations) {
  return violations.length === 0
    ? '<none>'
    : violations.map(formatRuntimeArtifactBoundaryViolation).join('\n');
}
