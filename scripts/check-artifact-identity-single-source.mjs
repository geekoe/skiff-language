#!/usr/bin/env node

import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  collectDevSyncArtifactPathFailures,
} from './lib/artifact-identity-dev-sync-check.mjs';
import { devSyncArtifactPathSelfTestFailures } from './lib/artifact-identity-dev-sync-check-self-test.mjs';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const skippedRustScanDirectories = new Set([
  '.git',
  '.skiff-instance',
  'build',
  'node_modules',
  'target',
]);
const artifactIdentityFacadePath = 'artifact-identity/src/lib.rs';
const ownerRequirements = [
  {
    name: 'framed_identity',
    relPath: 'artifact-identity/src/framing.rs',
    regexp: /\bpub\s+fn\s+framed_identity\s*\(/,
  },
  {
    name: 'framed_identity facade re-export',
    relPath: artifactIdentityFacadePath,
    regexp: /\bpub\s+use\s+framing::framed_identity\s*;/,
  },
  {
    name: 'FileIrIdentityPayload',
    relPath: 'artifact-identity/src/file_ir.rs',
    regexp: /\bstruct\s+FileIrIdentityPayload\b/,
  },
  {
    name: 'file_ir_identity',
    relPath: 'artifact-identity/src/file_ir.rs',
    regexp: /\bpub\s+fn\s+file_ir_identity\s*\(/,
  },
  {
    name: 'canonical_file_ir_identity_bytes',
    relPath: 'artifact-identity/src/file_ir.rs',
    regexp: /\bpub\s+fn\s+canonical_file_ir_identity_bytes\s*\(/,
  },
  {
    name: 'ServiceUnitStorageIdentityPayload',
    relPath: 'artifact-identity/src/legacy_service.rs',
    regexp: /\bstruct\s+ServiceUnitStorageIdentityPayload\b/,
  },
  {
    name: 'service_unit_identity',
    relPath: 'artifact-identity/src/legacy_service.rs',
    regexp: /\bpub\s+fn\s+service_unit_identity\s*\(/,
  },
  {
    name: 'service_unit_identity_bytes',
    relPath: 'artifact-identity/src/legacy_service.rs',
    regexp: /\bpub\s+fn\s+service_unit_identity_bytes\s*\(/,
  },
  {
    name: 'PackageLocalAbiIdentityProjection',
    relPath: 'artifact-identity/src/package/projection.rs',
    regexp: /\bpub\s+struct\s+PackageLocalAbiIdentityProjection\b/,
  },
  {
    name: 'PackageBuildIdentityProjection',
    relPath: 'artifact-identity/src/package/projection.rs',
    regexp: /\bpub\s+struct\s+PackageBuildIdentityProjection\b/,
  },
  {
    name: 'package_build_identity',
    relPath: 'artifact-identity/src/package.rs',
    regexp: /\bpub\s+fn\s+package_build_identity\s*\(/,
  },
  {
    name: 'package_local_abi_identity',
    relPath: 'artifact-identity/src/package.rs',
    regexp: /\bpub\s+fn\s+package_local_abi_identity\s*\(/,
  },
  {
    name: 'package_implementation_links_identity',
    relPath: 'artifact-identity/src/package.rs',
    regexp: /\bpub\s+fn\s+package_implementation_links_identity\s*\(/,
  },
  {
    name: 'PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX\b/,
  },
  {
    name: 'PublicationAbiIdentityProjection',
    relPath: 'artifact-identity/src/publication.rs',
    regexp: /\bstruct\s+PublicationAbiIdentityProjection\b/,
  },
  {
    name: 'publication_abi_identity',
    relPath: 'artifact-identity/src/publication.rs',
    regexp: /\bpub\s+fn\s+publication_abi_identity\s*\(/,
  },
  {
    name: 'publication_abi_identity_bytes',
    relPath: 'artifact-identity/src/publication.rs',
    regexp: /\bpub\s+fn\s+publication_abi_identity_bytes\s*\(/,
  },
  {
    name: 'OperationAbiIdentityInput',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+struct\s+OperationAbiIdentityInput\b/,
  },
  {
    name: 'operation_abi_hash',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+fn\s+operation_abi_hash\s*\(/,
  },
  {
    name: 'operation_abi_identity',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+fn\s+operation_abi_identity\s*\(/,
  },
  {
    name: 'public_function_operation_abi_id',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+fn\s+public_function_operation_abi_id\s*\(/,
  },
  {
    name: 'public_instance_method_operation_abi_id',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+fn\s+public_instance_method_operation_abi_id\s*\(/,
  },
  {
    name: 'abi_type_id_from_source_anchor',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+abi_type_id_from_source_anchor\s*\(/,
  },
  {
    name: 'abi_alias_id_from_source_anchor',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+abi_alias_id_from_source_anchor\s*\(/,
  },
  {
    name: 'abi_interface_id_from_source_anchor',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+abi_interface_id_from_source_anchor\s*\(/,
  },
  {
    name: 'abi_callable_id_from_source_anchor',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+abi_callable_id_from_source_anchor\s*\(/,
  },
  {
    name: 'abi_const_id_from_source_anchor',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+abi_const_id_from_source_anchor\s*\(/,
  },
  {
    name: 'abi_instance_id_from_source_anchor',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+abi_instance_id_from_source_anchor\s*\(/,
  },
  {
    name: 'type_ref_abi_key',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+type_ref_abi_key\s*\(/,
  },
  {
    name: 'interface_instantiation_ref',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+interface_instantiation_ref\s*\(/,
  },
  {
    name: 'interface_instantiation_ref_for_type_ref',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+interface_instantiation_ref_for_type_ref\s*\(/,
  },
  {
    name: 'canonical_interface_method_abi_id',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+canonical_interface_method_abi_id\s*\(/,
  },
  {
    name: 'canonical_interface_method_abi_id_from_parts',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+canonical_interface_method_abi_id_from_parts\s*</,
  },
  {
    name: 'canonical_interface_instantiation_key',
    relPath: 'artifact-identity/src/semantic.rs',
    regexp: /\bpub\s+fn\s+canonical_interface_instantiation_key\s*\(/,
  },
  {
    name: 'validate_publication_abi_surface',
    relPath: 'artifact-identity/src/publication_validation.rs',
    regexp: /\bpub\s+fn\s+validate_publication_abi_surface\s*\(/,
  },
  {
    name: 'validate_publication_abi_identity',
    relPath: 'artifact-identity/src/publication_validation.rs',
    regexp: /\bpub\s+fn\s+validate_publication_abi_identity\s*\(/,
  },
  {
    name: 'PackageTestBuildIdentityPayload',
    relPath: 'artifact-identity/src/package_test.rs',
    regexp: /\bstruct\s+PackageTestBuildIdentityPayload\b/,
  },
  {
    name: 'RuntimeProgramServiceUnitIdentityPayload',
    relPath: 'artifact-identity/src/runtime_program.rs',
    regexp: /\bstruct\s+RuntimeProgramServiceUnitIdentityPayload\b/,
  },
  {
    name: 'ArtifactRelativePath',
    relPath: 'artifact-identity/src/artifact_path.rs',
    regexp: /\bpub\s+struct\s+ArtifactRelativePath\b/,
  },
  {
    name: 'ServiceAssemblyArtifactRef',
    relPath: 'artifact-identity/src/artifact_reference.rs',
    regexp: /\bpub\s+struct\s+ServiceAssemblyArtifactRef\b/,
  },
  {
    name: 'ServiceUnitArtifactRef',
    relPath: 'artifact-identity/src/artifact_reference.rs',
    regexp: /\bpub\s+struct\s+ServiceUnitArtifactRef\b/,
  },
  {
    name: 'PackageUnitArtifactRef',
    relPath: 'artifact-identity/src/artifact_reference.rs',
    regexp: /\bpub\s+struct\s+PackageUnitArtifactRef\b/,
  },
  {
    name: 'service_assembly_identity_projection',
    relPath: 'artifact-identity/src/service_assembly_identity.rs',
    regexp: /\bpub\s+fn\s+service_assembly_identity_projection\s*\(/,
  },
  {
    name: 'service_assembly_hash',
    relPath: 'artifact-identity/src/service_assembly_identity.rs',
    regexp: /\bpub\s+fn\s+service_assembly_hash\s*\(/,
  },
  {
    name: 'service_assembly_identity',
    relPath: 'artifact-identity/src/service_assembly_identity.rs',
    regexp: /\bpub\s+fn\s+service_assembly_identity\s*\(/,
  },
  {
    name: 'package_unit_content_hash',
    relPath: 'artifact-identity/src/artifact_coordinates.rs',
    regexp: /\bpub\s+fn\s+package_unit_content_hash\s*\(/,
  },
  {
    name: 'validate_package_unit_artifact_path',
    relPath: 'artifact-identity/src/artifact_coordinates.rs',
    regexp: /\bpub\s+fn\s+validate_package_unit_artifact_path\s*\(/,
  },
  {
    name: 'validate_service_artifact_closure',
    relPath: 'artifact-identity/src/service_artifact_closure.rs',
    regexp: /\bpub\s+fn\s+validate_service_artifact_closure\s*\(/,
  },
  {
    name: 'canonical_json_value',
    relPath: 'canonical-json/src/lib.rs',
    regexp: /\bpub\s+fn\s+canonical_json_value\s*\(/,
  },
  {
    name: 'canonical_json_number',
    relPath: 'canonical-json/src/lib.rs',
    regexp: /\bpub\s+fn\s+canonical_json_number\s*\(/,
  },
  {
    name: 'canonical_json_bytes',
    relPath: 'canonical-json/src/lib.rs',
    regexp: /\bpub\s+fn\s+canonical_json_bytes\s*</,
  },
];

const exclusiveDefinitionNames = new Set([
  'framed_identity',
  'FileIrIdentityPayload',
  'ServiceUnitStorageIdentityPayload',
  'PackageLocalAbiIdentityProjection',
  'PackageBuildIdentityProjection',
  'package_local_abi_identity',
  'package_implementation_links_identity',
  'PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX',
  'PublicationAbiIdentityProjection',
  'OperationAbiIdentityInput',
  'PackageTestBuildIdentityPayload',
  'RuntimeProgramServiceUnitIdentityPayload',
  'ArtifactRelativePath',
  'ServiceAssemblyArtifactRef',
  'ServiceUnitArtifactRef',
  'PackageUnitArtifactRef',
  'service_assembly_identity_projection',
  'service_assembly_hash',
  'service_assembly_identity',
  'package_unit_content_hash',
  'validate_package_unit_artifact_path',
  'validate_service_artifact_closure',
  'canonical_file_ir_identity_bytes',
  'service_unit_identity_bytes',
  'publication_abi_identity_bytes',
  'operation_abi_identity',
  'abi_type_id_from_source_anchor',
  'abi_alias_id_from_source_anchor',
  'abi_interface_id_from_source_anchor',
  'abi_callable_id_from_source_anchor',
  'abi_const_id_from_source_anchor',
  'abi_instance_id_from_source_anchor',
  'type_ref_abi_key',
  'interface_instantiation_ref',
  'interface_instantiation_ref_for_type_ref',
  'canonical_interface_method_abi_id',
  'canonical_interface_method_abi_id_from_parts',
  'canonical_interface_instantiation_key',
  'validate_publication_abi_surface',
  'validate_publication_abi_identity',
  'canonical_json_value',
  'canonical_json_number',
  'canonical_json_bytes',
]);
const definitionOwnerByName = new Map(
  ownerRequirements
    .filter(({ name }) => exclusiveDefinitionNames.has(name))
    .map(({ name, relPath }) => [name, relPath]),
);
const ownedDefinitionRegexp = new RegExp(
  `\\b(?:struct|fn|const)\\s+(${[...definitionOwnerByName.keys()].join('|')})\\b`,
  'g',
);

const facadeModules = [
  'artifact_coordinates',
  'artifact_path',
  'artifact_reference',
  'constants',
  'error',
  'file_ir',
  'framing',
  'legacy_service',
  'operation',
  'package',
  'package_test',
  'publication',
  'publication_validation',
  'runtime_program',
  'semantic',
  'service_artifact_closure',
  'service_assembly_identity',
];

const canonicalDelegationRequirements = [
  {
    relPath: 'artifact-identity/src/framing.rs',
    helper: 'artifact identity canonical bytes',
    regexp: /\bskiff_canonical_json::canonical_json_bytes\b/,
  },
  {
    relPath: 'compiler/core/src/json_utils.rs',
    helper: 'compiler canonical JSON API',
    regexp: /\bpub\s+use\s+skiff_canonical_json\s*::/,
  },
  {
    relPath: 'runtime/linker/src/json_utils.rs',
    helper: 'runtime linker canonical JSON API',
    regexp: /\buse\s+skiff_canonical_json::canonical_json_value\b/,
  },
  {
    relPath: 'runtime/linked-type-plan/src/type_plan.rs',
    helper: 'sort-only linked type key helper',
    regexp: /\bfn\s+sort_json_value\s*\(/,
  },
];

const adapterRequirements = [
  {
    relPath: 'compiler/lowering/src/file_ir/identity.rs',
    helper: 'File IR identity',
    regexp: /\bskiff_artifact_identity::file_ir_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'File IR identity',
    regexp: /\bskiff_artifact_identity::file_ir_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'service-unit identity',
    regexp: /\bskiff_artifact_identity::service_unit_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/package_unit_artifacts/mod.rs',
    helper: 'package identity assignment',
    regexp: /\bskiff_artifact_identity::assign_package_unit_identities\b/,
  },
  {
    relPath: 'compiler/emission/src/emission/package_unit_artifacts.rs',
    helper: 'projected package identity validation',
    regexp: /\bskiff_artifact_identity::validate_package_unit_identities\b/,
  },
  {
    relPath: 'compiler/emission/src/emission/package_test_artifacts/package_units.rs',
    helper: 'package implementation links identity',
    regexp: /\bskiff_artifact_identity::package_implementation_links_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'publication ABI identity',
    regexp: /\bskiff_artifact_identity::publication_abi_identity\b/,
  },
  {
    relPath: 'compiler/publication-abi/src/lib.rs',
    helper: 'public function operation ABI identity',
    regexp: /\bskiff_artifact_identity::public_function_operation_abi_id\b/,
  },
  {
    relPath: 'compiler/publication-abi/src/lib.rs',
    helper: 'public instance operation ABI identity',
    regexp: /\bskiff_artifact_identity::public_instance_method_operation_abi_id\b/,
  },
  {
    relPath: 'compiler/emission/src/emission/identity.rs',
    helper: 'artifact emission identity re-export',
    regexp: /\bpub\s+use\s+skiff_artifact_identity\s*::/,
  },
  {
    relPath: 'compiler/emission/src/emission/identity.rs',
    helper: 'service assembly identity',
    regexp: /\bservice_assembly_identity\b/,
  },
  {
    relPath: 'compiler/input/src/service_dependencies.rs',
    helper: 'compiler service dependency artifact closure validation',
    regexp: /\bvalidate_service_artifact_closure\s*\(/,
  },
  {
    relPath: 'runtime/package-test/src/lib.rs',
    helper: 'package implementation links identity',
    regexp: /\bskiff_artifact_identity::\{[^}]*package_implementation_links_identity|\buse\s+skiff_artifact_identity::package_implementation_links_identity\b/,
  },
  {
    relPath: 'compiler/emission/src/emission/identity.rs',
    helper: 'artifact emission framed_identity',
    regexp:
      /\bpub\s+use\s+skiff_artifact_identity\s*::\s*\{[\s\S]*?\bframed_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'public_function_operation_abi_id',
    regexp: /\bskiff_artifact_identity::public_function_operation_abi_id\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'public_instance_method_operation_abi_id',
    regexp: /\bskiff_artifact_identity::public_instance_method_operation_abi_id\b/,
  },
];

const artifactEmissionIdentityAdapterPath = 'compiler/emission/src/emission/identity.rs';
const artifactEmissionIdentityAdapterRegexp =
  /^\s*pub\s+use\s+skiff_artifact_identity\s*::\s*\{\s*[A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*\s*,?\s*\}\s*;\s*$/;
const artifactEmissionFramingRequirements = [
  {
    relPath: 'compiler/emission/src/emission/package_artifacts.rs',
    helper: 'package emission adapter import',
    regexp: /\bidentity\s*::\s*\{[^}]*\bframed_identity\b/,
  },
  {
    relPath: 'compiler/emission/src/emission/package_artifacts.rs',
    helper: 'package assembly identity',
    regexp: /\bframed_identity\s*\(\s*PACKAGE_ASSEMBLY_IDENTITY_PREFIX\s*,/,
  },
  {
    relPath: 'compiler/emission/src/emission/service_artifacts.rs',
    helper: 'service emission adapter import',
    regexp: /\buse\s+crate::emission::identity::framed_identity\s*;/,
  },
  {
    relPath: 'compiler/emission/src/emission/service_artifacts.rs',
    helper: 'service bundle identity',
    regexp: /\bframed_identity\s*\(\s*BUNDLE_IDENTITY_PREFIX\s*,/,
  },
];
const options = parseArgs(process.argv.slice(2));

if (options.help) {
  printUsage();
} else if (options.selfTest) {
  runSelfTest();
} else {
  await runCheck();
}

async function runCheck() {
  const failures = [];
  const files = await collectCandidateRustFiles(root);
  const scriptFiles = await collectIdentityScriptFiles();
  const ownerTextByPath = new Map();
  for (const requirement of ownerRequirements) {
    if (!ownerTextByPath.has(requirement.relPath)) {
      ownerTextByPath.set(
        requirement.relPath,
        stripInlineTestModules(await readFile(join(root, requirement.relPath), 'utf8')),
      );
    }
    if (!requirement.regexp.test(ownerTextByPath.get(requirement.relPath))) {
      failures.push(`${requirement.relPath} is missing owned ${requirement.name}`);
    }
  }

  const facadeText = stripInlineTestModules(
    await readFile(join(root, artifactIdentityFacadePath), 'utf8'),
  );
  for (const moduleName of facadeModules) {
    const moduleDeclaration = new RegExp(`\\bmod\\s+${moduleName}\\s*;`);
    if (!moduleDeclaration.test(facadeText)) {
      failures.push(`${artifactIdentityFacadePath} is missing ${moduleName} module declaration`);
    }
  }
  if (/\b(?:struct|enum|fn)\s+\w+/.test(facadeText)) {
    failures.push(`${artifactIdentityFacadePath} must contain declarations and re-exports only`);
  }

  const adapterTextByPath = new Map();
  for (const { relPath } of adapterRequirements) {
    if (!adapterTextByPath.has(relPath)) {
      adapterTextByPath.set(relPath, await readFile(join(root, relPath), 'utf8'));
    }
  }
  failures.push(...collectAdapterRequirementFailures(adapterRequirements, adapterTextByPath));
  failures.push(...collectArtifactEmissionIdentityAdapterFailures(files));

  const artifactEmissionFramingTextByPath = new Map();
  for (const { relPath } of artifactEmissionFramingRequirements) {
    if (!artifactEmissionFramingTextByPath.has(relPath)) {
      artifactEmissionFramingTextByPath.set(
        relPath,
        await readFile(join(root, relPath), 'utf8'),
      );
    }
  }
  failures.push(
    ...collectArtifactEmissionFramingRequirementFailures(
      artifactEmissionFramingRequirements,
      artifactEmissionFramingTextByPath,
    ),
  );

  const canonicalDelegationTextByPath = new Map();
  for (const { relPath } of canonicalDelegationRequirements) {
    if (!canonicalDelegationTextByPath.has(relPath)) {
      canonicalDelegationTextByPath.set(relPath, await readFile(join(root, relPath), 'utf8'));
    }
  }
  failures.push(
    ...collectDelegationRequirementFailures(
      canonicalDelegationRequirements,
      canonicalDelegationTextByPath,
    ),
  );

  for (const violation of collectOwnedDefinitionViolations(files)) {
    failures.push(
      `${violation.relPath}:${violation.line} ${violation.name} is owned by ${violation.owner}`,
    );
  }
  for (const violation of collectPackageImplementationLinksIdentityViolations([
    ...files,
    ...scriptFiles,
  ])) {
    failures.push(`${violation.relPath}:${violation.line} ${violation.message}`);
  }
  for (const violation of collectServiceAssemblyIdentityViolations([...files, ...scriptFiles])) {
    failures.push(`${violation.relPath}:${violation.line} ${violation.message}`);
  }
  failures.push(...collectDevSyncArtifactPathFailures(
    await readFile(join(root, 'scripts/skiff-dev-sync.mjs'), 'utf8'),
    await readFile(join(root, 'scripts/lib/artifact-identity-dev-sync-paths.mjs'), 'utf8'),
    scriptFiles,
  ));

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`FAIL ${failure}`);
    }
    process.exitCode = 1;
    return;
  }

  console.log('Artifact identity single-source check passed.');
}

function collectDelegationRequirementFailures(requirements, textByPath) {
  const failures = [];
  for (const requirement of requirements) {
    const text = textByPath.get(requirement.relPath);
    if (text === undefined) {
      failures.push(`${requirement.relPath} is missing required ${requirement.helper}`);
      continue;
    }
    if (!requirement.regexp.test(stripInlineTestModules(text))) {
      failures.push(`${requirement.relPath} is missing required ${requirement.helper} delegation`);
    }
  }
  return failures;
}

function collectAdapterRequirementFailures(requirements, textByPath) {
  const failures = [];
  for (const requirement of requirements) {
    const text = textByPath.get(requirement.relPath);
    if (text === undefined) {
      failures.push(`${requirement.relPath} is missing required ${requirement.helper} adapter`);
      continue;
    }
    const productionText = stripInlineTestModules(text);
    if (!requirement.regexp.test(productionText)) {
      failures.push(
        `${requirement.relPath} must delegate ${requirement.helper} to skiff_artifact_identity`,
      );
    }
  }
  return failures;
}

function collectArtifactEmissionIdentityAdapterFailures(files) {
  const adapter = files.find(({ relPath }) => relPath === artifactEmissionIdentityAdapterPath);
  if (adapter === undefined) {
    return [`${artifactEmissionIdentityAdapterPath} is missing`];
  }
  const productionText = stripRustComments(stripInlineTestModules(adapter.text));
  if (
    artifactEmissionIdentityAdapterRegexp.test(productionText)
    && /\bframed_identity\b/.test(productionText)
  ) {
    return [];
  }
  return [
    `${artifactEmissionIdentityAdapterPath} must be a single pub use skiff_artifact_identity::{...} adapter containing framed_identity`,
  ];
}

function collectArtifactEmissionFramingRequirementFailures(requirements, textByPath) {
  const failures = [];
  for (const requirement of requirements) {
    const text = textByPath.get(requirement.relPath);
    const productionText =
      text === undefined ? '' : stripRustComments(stripInlineTestModules(text));
    if (!requirement.regexp.test(productionText)) {
      failures.push(
        `${requirement.relPath} must frame ${requirement.helper} through the artifact emission framed_identity adapter`,
      );
    }
  }
  return failures;
}

function collectOwnedDefinitionViolations(files) {
  const violations = [];

  for (const file of files) {
    if (!isProductionRustFile(file.relPath)) {
      continue;
    }
    const text = stripInlineTestModules(file.text);
    for (const match of text.matchAll(ownedDefinitionRegexp)) {
      const name = match[1];
      const owner = definitionOwnerByName.get(name);
      if (owner === file.relPath) {
        continue;
      }
      violations.push({
        relPath: file.relPath,
        line: lineNumberAt(text, match.index ?? 0),
        name,
        owner,
      });
    }
  }

  return violations;
}

function collectPackageImplementationLinksIdentityViolations(files) {
  const restrictions = [
    {
      regexp: /skiff-package-implementation-links-v1:sha256/g,
      message: 'package implementation links identity prefix is owned by artifact-identity',
    },
    {
      regexp: /\b(?:fn|function)\s+(?:(?:package_)?implementation_links_identity|packageImplementationLinksIdentity)\s*\(/g,
      message: 'package implementation links identity helper is owned by artifact-identity',
    },
  ];
  const violations = [];
  for (const file of files) {
    if (
      file.relPath === 'artifact-identity/src/package.rs'
      || file.relPath === 'artifact-identity/src/constants.rs'
      || file.relPath === 'scripts/check-artifact-identity-single-source.mjs'
      || !isProductionIdentitySource(file.relPath)
    ) {
      continue;
    }
    const text = file.relPath.endsWith('.rs')
      ? stripInlineTestModules(file.text)
      : file.text;
    for (const restriction of restrictions) {
      restriction.regexp.lastIndex = 0;
      for (const match of text.matchAll(restriction.regexp)) {
        violations.push({
          relPath: file.relPath,
          line: lineNumberAt(text, match.index ?? 0),
          message: restriction.message,
        });
      }
    }
  }
  return violations;
}

function collectServiceAssemblyIdentityViolations(files) {
  const definitionPatterns = [
    /\b(?:fn|function)\s+service_assembly_identity_projection\s*\(/g,
    /\b(?:fn|function)\s+service_assembly_hash\s*\(/g,
    /\b(?:fn|function)\s+service_assembly_identity\s*\(/g,
    /\bfunction\s+serviceAssemblyHashInput\s*\(/g,
    /\bfunction\s+serviceAssemblyIdentityProjection\s*\(/g,
    /\bfunction\s+serviceAssemblyHash\s*\(/g,
    /\bfunction\s+serviceAssemblyIdentity\s*\(/g,
  ];
  const violations = [];
  for (const file of files) {
    if (
      file.relPath === 'artifact-identity/src/service_assembly_identity.rs'
      || file.relPath === 'scripts/check-artifact-identity-single-source.mjs'
      || (file.relPath.endsWith('.rs') && !isProductionRustFile(file.relPath))
    ) {
      continue;
    }
    const text = file.relPath.endsWith('.rs')
      ? stripInlineTestModules(file.text)
      : file.text;
    for (const regexp of definitionPatterns) {
      regexp.lastIndex = 0;
      for (const match of text.matchAll(regexp)) {
        violations.push({
          relPath: file.relPath,
          line: lineNumberAt(text, match.index ?? 0),
          message: 'service assembly identity projection/hash is owned by artifact-identity',
        });
      }
    }
    for (const prefixMatch of text.matchAll(/skiff-service-assembly-v1/g)) {
      const index = prefixMatch.index ?? 0;
      const nearbyImplementation = text.slice(
        Math.max(0, index - 400),
        Math.min(text.length, index + prefixMatch[0].length + 400),
      );
      if (/\b(?:createHash|Sha256|value_sha256|stableStringify|canonical_json_(?:value|bytes))\b/.test(nearbyImplementation)) {
        violations.push({
          relPath: file.relPath,
          line: lineNumberAt(text, index),
          message: 'service assembly identity prefix and hashing must not be combined outside artifact-identity',
        });
      }
    }
  }
  return violations;
}

async function collectIdentityScriptFiles() {
  const files = [];
  await collectIdentitySourceFiles(join(root, 'router', 'src'), files);
  await collectIdentitySourceFiles(join(root, 'scripts'), files);
  return files;
}

async function collectIdentitySourceFiles(directory, files) {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }
  for (const entry of entries) {
    const absPath = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name === 'tests') {
        continue;
      }
      await collectIdentitySourceFiles(absPath, files);
      continue;
    }
    if (
      !entry.isFile()
      || (!entry.name.endsWith('.ts') && !entry.name.endsWith('.mjs'))
      || entry.name.includes('.test.')
    ) {
      continue;
    }
    files.push({
      absPath,
      relPath: normalizePath(relative(root, absPath)),
      text: await readFile(absPath, 'utf8'),
    });
  }
}

async function collectCandidateRustFiles(repoRoot) {
  const files = [];
  await collectRustFiles(repoRoot, files);
  return files;
}

async function collectRustFiles(directory, files) {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      return;
    }
    throw error;
  }

  for (const entry of entries) {
    const absPath = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (shouldSkipRustScanDirectory(entry.name)) {
        continue;
      }
      await collectRustFiles(absPath, files);
      continue;
    }
    if (!entry.isFile() || !entry.name.endsWith('.rs')) {
      continue;
    }
    files.push({
      absPath,
      relPath: normalizePath(relative(root, absPath)),
      text: await readFile(absPath, 'utf8'),
    });
  }
}

function shouldSkipRustScanDirectory(name) {
  return skippedRustScanDirectories.has(name);
}

function isProductionRustFile(relPath) {
  if (relPath.endsWith('/tests.rs') || relPath.split('/').includes('tests')) {
    return false;
  }
  return relPath.endsWith('.rs');
}

function isProductionIdentitySource(relPath) {
  if (relPath.endsWith('.rs')) {
    return isProductionRustFile(relPath);
  }
  return (
    (relPath.startsWith('router/src/') || relPath.startsWith('scripts/'))
    && (relPath.endsWith('.ts') || relPath.endsWith('.mjs'))
    && !relPath.includes('.test.')
    && !relPath.split('/').includes('tests')
  );
}

function runSelfTest() {
  const cases = [
    {
      name: 'allows definitions in their declared owner modules',
      files: [
        {
          relPath: 'artifact-identity/src/framing.rs',
          text: 'pub fn framed_identity() {}\n',
        },
        {
          relPath: 'artifact-identity/src/operation.rs',
          text: 'pub struct OperationAbiIdentityInput;\npub fn operation_abi_identity() {}\n',
        },
        {
          relPath: 'canonical-json/src/lib.rs',
          text: 'pub fn canonical_json_value() {}\n',
        },
      ],
      expectedViolations: 0,
    },
    {
      name: 'rejects compiler operation identity duplicate struct',
      files: [
        {
          relPath: 'compiler/driver/shared/operation_abi_identity.rs',
          text: 'struct OperationAbiIdentityInput;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects lowering File IR payload duplicate',
      files: [
        {
          relPath: 'compiler/lowering/src/file_ir/identity.rs',
          text: 'struct FileIrIdentityPayload;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects package build identity projection duplicate',
      files: [
        {
          relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
          text: 'struct PackageBuildIdentityProjection;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects publication ABI byte projection duplicate',
      files: [
        {
          relPath: 'compiler/publication-abi/src/identity.rs',
          text: 'fn publication_abi_identity_bytes() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects an identity definition in the wrong artifact-identity module',
      files: [
        {
          relPath: 'artifact-identity/src/other.rs',
          text: 'fn operation_abi_identity() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects artifact emission framed_identity outside its owner',
      files: [
        {
          relPath: 'compiler/emission/src/emission/identity.rs',
          text: 'pub fn framed_identity() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects a canonical JSON definition outside the leaf owner',
      files: [
        {
          relPath: 'runtime/linker/src/json_utils.rs',
          text: 'fn canonical_json_value() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'ignores compiler test files',
      files: [
        {
          relPath: 'compiler/tests/operation_identity.rs',
          text: 'struct OperationAbiIdentityInput;\nfn operation_abi_identity() {}\n',
        },
      ],
      expectedViolations: 0,
    },
    {
      name: 'ignores cfg test modules',
      files: [
        {
          relPath: 'compiler/driver/shared/operation_abi_identity.rs',
          text: '#[cfg(test)]\nmod tests { struct OperationAbiIdentityInput; }\n',
        },
      ],
      expectedViolations: 0,
    },
    {
      name: 'rejects compiler package implementation links identity prefix',
      files: [
        {
          relPath: 'compiler/emission/src/package_test.rs',
          text: 'const PREFIX: &str = "skiff-package-implementation-links-v1:sha256";\n',
        },
      ],
      expectedViolations: 0,
      expectedPackageImplementationLinksViolations: 1,
    },
    {
      name: 'rejects compiler package implementation links identity helper',
      files: [
        {
          relPath: 'compiler/emission/src/package_test.rs',
          text: 'fn implementation_links_identity() {}\n',
        },
      ],
      expectedViolations: 0,
      expectedPackageImplementationLinksViolations: 1,
    },
    {
      name: 'rejects runtime package implementation links duplicate owner',
      files: [
        {
          relPath: 'runtime/package-test/src/lib.rs',
          text: 'const PREFIX: &str = "skiff-package-implementation-links-v1:sha256";\nfn package_implementation_links_identity() {}\n',
        },
      ],
      expectedViolations: 1,
      expectedPackageImplementationLinksViolations: 2,
    },
    {
      name: 'rejects script package implementation links duplicate owner',
      files: [
        {
          relPath: 'scripts/local-package-identity.mjs',
          text: 'const prefix = "skiff-package-implementation-links-v1:sha256";\nfunction packageImplementationLinksIdentity() {}\n',
        },
      ],
      expectedViolations: 0,
      expectedPackageImplementationLinksViolations: 2,
    },
  ];

  const failures = [];
  for (const testCase of cases) {
    const violations = collectOwnedDefinitionViolations(testCase.files);
    if (violations.length !== testCase.expectedViolations) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedViolations} violation(s), got ${violations.length}`,
      );
    }
    const packageImplementationLinksViolations =
      collectPackageImplementationLinksIdentityViolations(testCase.files);
    const expectedPackageImplementationLinksViolations =
      testCase.expectedPackageImplementationLinksViolations ?? 0;
    if (
      packageImplementationLinksViolations.length
      !== expectedPackageImplementationLinksViolations
    ) {
      failures.push(
        `${testCase.name}: expected ${expectedPackageImplementationLinksViolations} package implementation links violation(s), got ${packageImplementationLinksViolations.length}`,
      );
    }
  }

  const serviceAssemblyDuplicateCases = [
    {
      name: 'rejects router service assembly hash implementation',
      files: [{
        relPath: 'router/src/artifacts/identity.ts',
        text: 'function serviceAssemblyHashInput(value: unknown) { return value; }\n',
      }],
      expectedViolations: 1,
    },
    {
      name: 'rejects renamed prefix plus hash implementation',
      files: [{
        relPath: 'scripts/local-identity.mjs',
        text: 'const prefix = "skiff-service-assembly-v1"; createHash("sha256");\n',
      }],
      expectedViolations: 1,
    },
    {
      name: 'allows CLI-only script adapters',
      files: [{
        relPath: 'scripts/lib/artifact-identity-validation.mjs',
        text: 'spawn(path, ["runtime-program-build-id"]);\n',
      }],
      expectedViolations: 0,
    },
  ];
  for (const testCase of serviceAssemblyDuplicateCases) {
    const violations = collectServiceAssemblyIdentityViolations(testCase.files);
    if (violations.length !== testCase.expectedViolations) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedViolations} service assembly violation(s), got ${violations.length}`,
      );
    }
  }

  failures.push(...devSyncArtifactPathSelfTestFailures());

  const artifactEmissionAdapterCases = [
    {
      name: 'allows the pure artifact emission re-export adapter',
      files: [
        {
          relPath: 'compiler/emission/src/emission/identity.rs',
          text: 'pub use skiff_artifact_identity::{file_ir_identity, framed_identity};\n',
        },
      ],
      expectedFailures: 0,
    },
    {
      name: 'rejects a local function and format algorithm in the artifact emission adapter',
      files: [
        {
          relPath: 'compiler/emission/src/emission/identity.rs',
          text: `pub use skiff_artifact_identity::{file_ir_identity, framed_identity};
pub fn identity(prefix: &str, hash: &str) -> String {
  format!("{prefix}:{hash}")
}
`,
        },
      ],
      expectedFailures: 1,
    },
    {
      name: 'rejects a local const in the artifact emission adapter',
      files: [
        {
          relPath: 'compiler/emission/src/emission/identity.rs',
          text: `pub use skiff_artifact_identity::{file_ir_identity, framed_identity};
pub const SEPARATOR: &str = ":";
`,
        },
      ],
      expectedFailures: 1,
    },
    {
      name: 'rejects any other production item in the artifact emission adapter',
      files: [
        {
          relPath: 'compiler/emission/src/emission/identity.rs',
          text: `pub use skiff_artifact_identity::{file_ir_identity, framed_identity};
pub struct LocalIdentity;
`,
        },
      ],
      expectedFailures: 1,
    },
    {
      name: 'does not inspect unrelated compiler helpers',
      files: [
        {
          relPath: 'compiler/emission/src/emission/identity.rs',
          text: 'pub use skiff_artifact_identity::{file_ir_identity, framed_identity};\n',
        },
        {
          relPath: 'compiler/source/src/cache.rs',
          text: 'fn cache_path(prefix: &str, hash: &str) -> String { format!("{prefix}/{hash}") }\n',
        },
      ],
      expectedFailures: 0,
    },
  ];
  for (const testCase of artifactEmissionAdapterCases) {
    const adapterFailures = collectArtifactEmissionIdentityAdapterFailures(testCase.files);
    if (adapterFailures.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} failure(s), got ${adapterFailures.length}`,
      );
    }
  }

  const adapterFixtureRequirement = [
    {
      relPath: 'compiler/example/src/identity.rs',
      helper: 'fixture identity',
      regexp: /\bskiff_artifact_identity::file_ir_identity\b/,
    },
  ];
  const testOnlyAdapterFailures = collectAdapterRequirementFailures(
    adapterFixtureRequirement,
    new Map([
      [
        'compiler/example/src/identity.rs',
        `#[cfg(test)]
mod tests {
  fn parity() {
    skiff_artifact_identity::file_ir_identity();
  }
}
`,
      ],
    ]),
  );
  if (testOnlyAdapterFailures.length !== 1) {
    failures.push(
      `rejects test-only adapter delegation: expected 1 failure, got ${testOnlyAdapterFailures.length}`,
    );
  }

  const productionAdapterFailures = collectAdapterRequirementFailures(
    adapterFixtureRequirement,
    new Map([
      [
        'compiler/example/src/identity.rs',
        `fn identity() {
  skiff_artifact_identity::file_ir_identity();
}
`,
      ],
    ]),
  );
  if (productionAdapterFailures.length !== 0) {
    failures.push(
      `allows production adapter delegation: expected 0 failures, got ${productionAdapterFailures.length}`,
    );
  }

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`FAIL ${failure}`);
    }
    process.exitCode = 1;
    return;
  }

  console.log('Artifact identity single-source self-test passed.');
}

function stripInlineTestModules(text) {
  let output = text;
  let searchIndex = 0;
  while (searchIndex < output.length) {
    const attrIndex = output.indexOf('#[cfg(test)]', searchIndex);
    if (attrIndex === -1) {
      break;
    }
    const removal = cfgTestItemRange(output, attrIndex);
    if (removal === undefined) {
      searchIndex = attrIndex + 1;
      continue;
    }
    const replacement = output.slice(removal.start, removal.end).replace(/[^\n]/g, ' ');
    output = output.slice(0, removal.start) + replacement + output.slice(removal.end);
    searchIndex = removal.start + replacement.length;
  }
  return output;
}

function stripRustComments(text) {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/\/\/[^\n]*/g, '');
}

function cfgTestItemRange(text, attrIndex) {
  const attrMatch = /^#\[cfg\(test\)\]/.exec(text.slice(attrIndex));
  if (!attrMatch) {
    return undefined;
  }
  let index = attrIndex + attrMatch[0].length;
  while (index < text.length && /\s/.test(text[index])) {
    index += 1;
  }
  const nextSemicolon = text.indexOf(';', index);
  const nextBrace = text.indexOf('{', index);
  if (nextSemicolon !== -1 && (nextBrace === -1 || nextSemicolon < nextBrace)) {
    return { start: attrIndex, end: nextSemicolon + 1 };
  }
  if (nextBrace !== -1) {
    const closeBrace = matchingBraceIndex(text, nextBrace);
    if (closeBrace !== -1) {
      return { start: attrIndex, end: closeBrace + 1 };
    }
  }
  const nextLine = text.indexOf('\n', index);
  if (nextLine !== -1) {
    return { start: attrIndex, end: nextLine + 1 };
  }
  return { start: attrIndex, end: text.length };
}

function matchingBraceIndex(text, openBrace) {
  let depth = 0;
  for (let index = openBrace; index < text.length; index += 1) {
    const char = text[index];
    if (char === '{') {
      depth += 1;
    } else if (char === '}') {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}

function lineNumberAt(text, index) {
  let line = 1;
  for (let cursor = 0; cursor < index; cursor += 1) {
    if (text.charCodeAt(cursor) === 10) {
      line += 1;
    }
  }
  return line;
}

function parseArgs(argv) {
  const parsed = {
    help: false,
    selfTest: false,
  };

  for (const arg of argv) {
    if (arg === '-h' || arg === '--help') {
      parsed.help = true;
      continue;
    }
    if (arg === '--self-test') {
      parsed.selfTest = true;
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }

  return parsed;
}

function printUsage() {
  console.log(`Usage: node scripts/check-artifact-identity-single-source.mjs [--self-test]

Checks that canonical JSON and artifact identity definitions live in their declared
crate/module owners while compiler and runtime consumers use the public owner APIs.`);
}

function normalizePath(path) {
  return path.split('\\').join('/');
}
