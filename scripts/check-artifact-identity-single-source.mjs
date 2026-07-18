#!/usr/bin/env node

import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  collectDevSyncArtifactPathFailures,
} from './lib/artifact-identity-dev-sync-check.mjs';
import { devSyncArtifactPathSelfTestFailures } from './lib/artifact-identity-dev-sync-check-self-test.mjs';
import {
  collectDeprecatedPackageAbiRustSymbolFailures,
  deprecatedPackageAbiRustSymbolSelfTestFailures,
} from './lib/artifact-identity-deprecated-package-abi.mjs';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const skippedRustScanDirectories = new Set([
  '.git',
  '.skiff-instance',
  'build',
  'node_modules',
  'target',
]);
const artifactIdentityFacadePath = 'artifact-identity/src/lib.rs';
const canonicalFileIrCallValidatorRegistry = Object.freeze([
  Object.freeze({
    name: 'service-call',
    owner: Object.freeze({
      name: 'validate_file_ir_service_calls',
      relPath: 'artifact-model/src/file_ir/service_calls.rs',
      regexp: /\bpub\s+fn\s+validate_file_ir_service_calls\s*\(/,
      fixtureText: 'pub fn validate_file_ir_service_calls() {}\n',
    }),
    shapeRequirements: Object.freeze([
      Object.freeze({
        relPath: 'artifact-model/src/file_ir.rs',
        description: 'ExternalRefTable typed service_call_refs',
        regexp: /\bpub\s+service_call_refs\s*:\s*Vec\s*<\s*ServiceCallRef\s*>/,
        forbiddenRegexp: /serde\s*\([^)]*default[^)]*\)\s*]\s*pub\s+service_call_refs\b/s,
        fixtureText: 'pub struct ExternalRefTable { pub service_call_refs: Vec<ServiceCallRef> }\n',
      }),
      Object.freeze({
        relPath: 'artifact-model/src/executable.rs',
        description: 'indexed ServiceCall target',
        regexp: /\bServiceCall\s*\{\s*service_call_ref_index\s*:\s*ServiceCallRefIndex\s*,?\s*\}/s,
        fixtureText: 'enum CallTargetIr { ServiceCall { service_call_ref_index: ServiceCallRefIndex } }\n',
      }),
    ]),
    consumers: Object.freeze([
      Object.freeze({
        relPath: 'artifact-identity/src/file_ir.rs',
        helper: 'File IR service-call identity validation',
        regexp: /\bvalidate_file_ir_service_calls\s*\(\s*unit\s*\)\s*\?/,
        fixtureText: 'fn identity(unit: &FileIrUnit) { validate_file_ir_service_calls(unit)?; }\n',
      }),
    ]),
  }),
  Object.freeze({
    name: 'package-call',
    owner: Object.freeze({
      name: 'validate_file_ir_package_calls',
      relPath: 'artifact-model/src/file_ir/package_calls.rs',
      regexp: /\bpub\s+fn\s+validate_file_ir_package_calls\s*\(/,
      fixtureText: 'pub fn validate_file_ir_package_calls() {}\n',
    }),
    shapeRequirements: Object.freeze([
      Object.freeze({
        relPath: 'artifact-model/src/file_ir.rs',
        description: 'ExternalRefTable typed package_callables',
        regexp: /\bpub\s+package_callables\s*:\s*Vec\s*<\s*PackageCallableRef\s*>/,
        fixtureText: 'pub struct ExternalRefTable { pub package_callables: Vec<PackageCallableRef> }\n',
      }),
      Object.freeze({
        relPath: 'artifact-model/src/executable.rs',
        description: 'exact PackageCallable target fields',
        regexp: /\bPackageCallable\s*\{\s*package_ref\s*:\s*PackageRefIr\s*,\s*package_callable_id\s*:\s*PackageCallableId\s*,?\s*\}/s,
        fixtureText: 'enum CallTargetIr { PackageCallable { package_ref: PackageRefIr, package_callable_id: PackageCallableId } }\n',
      }),
      Object.freeze({
        relPath: 'artifact-model/src/file_ir/package_calls.rs',
        description: 'package-call exact-set validation error matrix',
        regexp: /\benum\s+FileIrPackageCallValidationError\s*\{[\s\S]*\bMissingRef\s*\{[\s\S]*\bOrphanRef\s*\{[\s\S]*\bFieldMismatch\s*\{[\s\S]*\bDuplicateRef\s*\{/,
        fixtureText: 'enum FileIrPackageCallValidationError { MissingRef {}, OrphanRef {}, FieldMismatch {}, DuplicateRef {} }\n',
      }),
      Object.freeze({
        relPath: 'artifact-model/src/file_ir/package_calls.rs',
        description: 'package-call exact packageRef and callable-id key',
        regexp: /\bstruct\s+PackageCallKey\s*<[^>]+>\s*\{\s*package_ref\s*:\s*PackageRefKey\s*<[^>]+>\s*,\s*package_callable_id\s*:\s*&[^,]+str\s*,?\s*\}/s,
        fixtureText: "struct PackageCallKey<'a> { package_ref: PackageRefKey<'a>, package_callable_id: &'a str }\n",
      }),
    ]),
    consumers: Object.freeze([
      Object.freeze({
        relPath: 'artifact-identity/src/file_ir.rs',
        helper: 'File IR package-call identity validation',
        regexp: /\bvalidate_file_ir_package_calls\s*\(\s*unit\s*\)\s*\?/,
        fixtureText: 'fn identity(unit: &FileIrUnit) { validate_file_ir_package_calls(unit)?; }\n',
      }),
      Object.freeze({
        relPath: 'compiler/emission/src/emission/package_requirement_coverage.rs',
        helper: 'File IR package-call emission validation',
        regexp: /\bvalidate_file_ir_package_calls\s*\(\s*&file\.unit\s*\)/,
        fixtureText: 'fn emit(file: &File) { validate_file_ir_package_calls(&file.unit); }\n',
      }),
    ]),
  }),
]);
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
    name: 'ServiceCallRefIndex',
    relPath: 'artifact-model/src/file_ir.rs',
    regexp: /\bpub\s+struct\s+ServiceCallRefIndex\s*\(\s*u32\s*\)\s*;/,
  },
  ...canonicalFileIrCallValidatorRegistry.map(({ owner }) => owner),
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
    name: 'ServiceProtocolIdentityProjection',
    relPath: 'artifact-identity/src/contract.rs',
    regexp: /\bpub\s+struct\s+ServiceProtocolIdentityProjection\b/,
  },
  {
    name: 'contract_type_id',
    relPath: 'artifact-identity/src/contract.rs',
    regexp: /\bpub\s+fn\s+contract_type_id\s*\(/,
  },
  {
    name: 'contract_operation_id',
    relPath: 'artifact-identity/src/contract.rs',
    regexp: /\bpub\s+fn\s+contract_operation_id\s*\(/,
  },
  {
    name: 'service_protocol_identity',
    relPath: 'artifact-identity/src/contract.rs',
    regexp: /\bpub\s+fn\s+service_protocol_identity\s*\(/,
  },
  {
    name: 'BoundaryOperationContract',
    relPath: 'artifact-model/src/boundary/operation.rs',
    regexp: /\bpub\s+struct\s+BoundaryOperationContract\b/,
  },
  {
    name: 'BoundaryOperationDescriptor',
    relPath: 'artifact-model/src/boundary/operation.rs',
    regexp: /\bpub\s+struct\s+BoundaryOperationDescriptor\b/,
  },
  {
    name: 'PackageArtifactLocalAbiIdentityProjection',
    relPath: 'artifact-identity/src/package_artifact.rs',
    regexp: /\bpub\s+struct\s+PackageArtifactLocalAbiIdentityProjection\b/,
  },
  {
    name: 'PackageArtifactBuildIdentityProjection',
    relPath: 'artifact-identity/src/package_artifact.rs',
    regexp: /\bpub\s+struct\s+PackageArtifactBuildIdentityProjection\b/,
  },
  {
    name: 'package_artifact_local_abi_identity',
    relPath: 'artifact-identity/src/package_artifact.rs',
    regexp: /\bpub\s+fn\s+package_artifact_local_abi_identity\s*\(/,
  },
  {
    name: 'package_artifact_build_identity',
    relPath: 'artifact-identity/src/package_artifact.rs',
    regexp: /\bpub\s+fn\s+package_artifact_build_identity\s*\(/,
  },
  {
    name: 'assign_package_artifact_identities',
    relPath: 'artifact-identity/src/package_artifact.rs',
    regexp: /\bpub\s+fn\s+assign_package_artifact_identities\s*\(/,
  },
  {
    name: 'validate_package_artifact_identities',
    relPath: 'artifact-identity/src/package_artifact.rs',
    regexp: /\bpub\s+fn\s+validate_package_artifact_identities\s*\(/,
  },
  {
    name: 'SERVICE_PROTOCOL_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+SERVICE_PROTOCOL_IDENTITY_PREFIX\b/,
  },
  {
    name: 'PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX\b/,
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
  'ServiceCallRefIndex',
  ...canonicalFileIrCallValidatorRegistry.map(({ owner }) => owner.name),
  'ServiceUnitStorageIdentityPayload',
  'PackageLocalAbiIdentityProjection',
  'PackageBuildIdentityProjection',
  'package_local_abi_identity',
  'package_implementation_links_identity',
  'PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX',
  'ServiceProtocolIdentityProjection',
  'contract_type_id',
  'contract_operation_id',
  'service_protocol_identity',
  'BoundaryOperationContract',
  'BoundaryOperationDescriptor',
  'PackageArtifactLocalAbiIdentityProjection',
  'PackageArtifactBuildIdentityProjection',
  'package_artifact_local_abi_identity',
  'package_artifact_build_identity',
  'assign_package_artifact_identities',
  'validate_package_artifact_identities',
  'SERVICE_PROTOCOL_IDENTITY_PREFIX',
  'PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX',
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
  'contract',
  'error',
  'file_ir',
  'framing',
  'legacy_service',
  'operation',
  'package',
  'package_artifact',
  'package_test',
  'publication',
  'publication_validation',
  'runtime_program',
  'semantic',
  'service_artifact_closure',
  'service_assembly_identity',
];

const canonicalDelegationRequirements = [
  ...canonicalFileIrCallValidatorRegistry.flatMap(({ consumers }) => consumers),
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
    relPath: 'compiler/projection/src/package_artifact/projection.rs',
    helper: 'PackageArtifact identity assignment',
    regexp: /\buse\s+skiff_artifact_identity::assign_package_artifact_identities\b/,
  },
  {
    relPath: 'compiler/emission/src/emission/package_artifact.rs',
    helper: 'PackageArtifact identity validation',
    regexp: /\buse\s+skiff_artifact_identity::validate_package_artifact_identities\b/,
  },
  {
    relPath: 'compiler/driver/source_compile/canonical_dependencies.rs',
    helper: 'canonical package dependency identity validation',
    regexp: /\buse\s+skiff_artifact_identity::validate_package_artifact_identities\b/,
  },
  {
    relPath: 'compiler/driver/pipeline/mod.rs',
    helper: 'compiler-owned std PackageArtifact identity validation',
    regexp: /\buse\s+skiff_artifact_identity::validate_package_artifact_identities\b/,
  },
  {
    relPath: 'runtime/package-test/src/lib.rs',
    helper: 'package implementation links identity',
    regexp: /\bskiff_artifact_identity::\{[^}]*package_implementation_links_identity|\buse\s+skiff_artifact_identity::package_implementation_links_identity\b/,
  },
];
const canonicalCompileModelPaths = Object.freeze([
  ['artifact-model/src/package_artifact.rs', 'PackageArtifact'],
  ['artifact-model/src/service_contract.rs', 'ServiceContract'],
]);
const canonicalBoundaryContractPaths = Object.freeze({
  projection: 'artifact-model/src/boundary/projection.rs',
  operation: 'artifact-model/src/boundary/operation.rs',
  serviceContract: 'artifact-model/src/service_contract.rs',
});
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
  const rustTextByPath = new Map(files.map(({ relPath, text }) => [relPath, text]));
  failures.push(...collectOwnerRequirementFailures(ownerRequirements, rustTextByPath));

  const facadeSource = rustTextByPath.get(artifactIdentityFacadePath);
  if (facadeSource === undefined) {
    failures.push(`${artifactIdentityFacadePath} is missing canonical facade owner`);
  } else {
    const facadeText = stripInlineTestModules(facadeSource);
    for (const moduleName of facadeModules) {
      const moduleDeclaration = new RegExp(`\\bmod\\s+${moduleName}\\s*;`);
      if (!moduleDeclaration.test(facadeText)) {
        failures.push(`${artifactIdentityFacadePath} is missing ${moduleName} module declaration`);
      }
    }
    if (/\b(?:struct|enum|fn)\s+\w+/.test(facadeText)) {
      failures.push(`${artifactIdentityFacadePath} must contain declarations and re-exports only`);
    }
  }

  failures.push(...collectAdapterRequirementFailures(adapterRequirements, rustTextByPath));
  failures.push(
    ...collectDelegationRequirementFailures(
      canonicalDelegationRequirements,
      rustTextByPath,
    ),
  );

  for (const violation of collectOwnedDefinitionViolations(files)) {
    failures.push(
      `${violation.relPath}:${violation.line} ${violation.name} is owned by ${violation.owner}`,
    );
  }
  failures.push(...collectDeprecatedPackageAbiRustSymbolFailures(files));
  failures.push(...collectCanonicalCompileModelFailures(files));
  failures.push(...collectCanonicalBoundaryContractFailures(files));
  failures.push(...collectCanonicalFileIrCallValidatorFailures(files));
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

function collectOwnerRequirementFailures(requirements, textByPath) {
  const failures = [];
  for (const requirement of requirements) {
    const text = textByPath.get(requirement.relPath);
    if (text === undefined) {
      failures.push(`${requirement.relPath} is missing canonical owner ${requirement.name}`);
      continue;
    }
    if (!requirement.regexp.test(stripInlineTestModules(text))) {
      failures.push(`${requirement.relPath} is missing owned ${requirement.name}`);
    }
  }
  return failures;
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

function collectCanonicalCompileModelFailures(files) {
  const failures = [];
  const byPath = new Map(files.map((file) => [file.relPath, file]));
  for (const [relPath, typeName] of canonicalCompileModelPaths) {
    const file = byPath.get(relPath);
    if (file === undefined) {
      failures.push(`${relPath} is missing canonical ${typeName}`);
      continue;
    }
    const text = stripRustComments(stripInlineTestModules(file.text));
    if (!new RegExp(`\\bpub\\s+struct\\s+${typeName}\\b`).test(text)) {
      failures.push(`${relPath} is missing canonical ${typeName} definition`);
    }
    if (/\bservice_unit\s*::/.test(text)) {
      failures.push(`${relPath} must not depend on the ServiceUnit module`);
    }
    const legacyField = /\bpub\s+\w+\s*:\s*(?:(?:Box|Option|Vec)\s*<\s*)*(PublicationAbiUnit|PackageUnit|ServiceUnit)\b/.exec(text);
    if (legacyField !== null) {
      failures.push(`${relPath} canonical ${typeName} embeds legacy ${legacyField[1]}`);
    }
  }
  return failures;
}

function collectCanonicalBoundaryContractFailures(files) {
  const failures = [];
  const byPath = new Map(files.map((file) => [file.relPath, file]));
  const projection = byPath.get(canonicalBoundaryContractPaths.projection);
  const operation = byPath.get(canonicalBoundaryContractPaths.operation);
  const serviceContract = byPath.get(canonicalBoundaryContractPaths.serviceContract);
  if (projection === undefined || operation === undefined || serviceContract === undefined) {
    return ['canonical boundary operation owner files are incomplete'];
  }

  const projectionText = stripRustComments(stripInlineTestModules(projection.text));
  const operationText = stripRustComments(stripInlineTestModules(operation.text));
  const serviceContractText = stripRustComments(stripInlineTestModules(serviceContract.text));
  if (!/\bAvailable\s*\{\s*operation_contract\s*:\s*BoundaryOperationContract\s*,\s*implementation_requirements\s*:\s*BoundaryImplementationRequirements\s*,?\s*\}/s.test(projectionText)) {
    failures.push(
      'BoundaryCallableProjection::Available must contain only operation_contract and implementation_requirements',
    );
  }
  if (!/\bpub\s+struct\s+BoundaryOperationDescriptor\s*\{\s*pub\s+operation_id\s*:\s*ContractOperationId\s*,\s*pub\s+stable_key\s*:\s*String\s*,\s*pub\s+contract\s*:\s*BoundaryOperationContract\s*,?\s*\}/s.test(operationText)) {
    failures.push(
      'BoundaryOperationDescriptor must own the real operation id, stable key, and shared contract body',
    );
  }
  if (!/\bpub\s+operations\s*:\s*BTreeMap\s*<\s*ContractOperationId\s*,\s*BoundaryOperationDescriptor\s*>/.test(serviceContractText)) {
    failures.push('ServiceContract.operations must require BoundaryOperationDescriptor values');
  }
  return failures;
}

function collectCanonicalFileIrCallValidatorFailures(
  files,
  registry = canonicalFileIrCallValidatorRegistry,
) {
  const failures = [];
  const byPath = new Map(files.map((file) => [file.relPath, file]));
  for (const entry of registry) {
    for (const requirement of entry.shapeRequirements) {
      const file = byPath.get(requirement.relPath);
      if (file === undefined) {
        failures.push(
          `${requirement.relPath} is missing canonical ${entry.name} shape owner for ${requirement.description}`,
        );
        continue;
      }
      const text = stripRustComments(stripInlineTestModules(file.text));
      if (!requirement.regexp.test(text)) {
        failures.push(
          `${requirement.relPath} is missing canonical ${entry.name} shape ${requirement.description}`,
        );
      }
      if (requirement.forbiddenRegexp?.test(text)) {
        failures.push(
          `${requirement.relPath} has forbidden ${entry.name} shape ${requirement.description}`,
        );
      }
    }
  }
  return failures;
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

function canonicalFileIrCallValidatorFixture(entry) {
  const textByPath = new Map();
  for (const requirement of [entry.owner, ...entry.shapeRequirements, ...entry.consumers]) {
    const current = textByPath.get(requirement.relPath) ?? '';
    textByPath.set(requirement.relPath, current + requirement.fixtureText);
  }
  return [...textByPath].map(([relPath, text]) => ({ relPath, text }));
}

function mutateCanonicalFixture(files, requirement, replacement = '') {
  return files.map((file) => file.relPath === requirement.relPath
    ? { ...file, text: file.text.replace(requirement.fixtureText, replacement) }
    : file);
}

function collectFileIrCallValidatorGraphDiagnostics(entry, files) {
  const textByPath = new Map(files.map(({ relPath, text }) => [relPath, text]));
  return {
    ownerFailures: collectOwnerRequirementFailures([entry.owner], textByPath),
    shapeFailures: collectCanonicalFileIrCallValidatorFailures(files, [entry]),
    delegationFailures: collectDelegationRequirementFailures(entry.consumers, textByPath),
    duplicateViolations: collectOwnedDefinitionViolations(files)
      .filter(({ name }) => name === entry.owner.name),
  };
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
      name: 'rejects duplicate File IR service-call validator owner',
      files: [
        {
          relPath: 'compiler/lowering/src/service_call_validation.rs',
          text: 'fn validate_file_ir_service_calls() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects package build identity duplicate in terminal projection',
      files: [
        {
          relPath: 'compiler/projection/src/package_artifact/projection.rs',
          text: 'struct PackageBuildIdentityProjection;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects duplicate PackageArtifact identity validation in terminal emission',
      files: [
        {
          relPath: 'compiler/emission/src/emission/package_artifact.rs',
          text: 'fn validate_package_artifact_identities() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects service protocol identity projection duplicate',
      files: [
        {
          relPath: 'compiler/contract/src/identity.rs',
          text: 'struct ServiceProtocolIdentityProjection;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects duplicate boundary operation contract owner',
      files: [
        {
          relPath: 'compiler/projection/src/boundary/operation.rs',
          text: 'pub struct BoundaryOperationContract;\n',
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
      name: 'rejects framed_identity implementation in terminal package emission',
      files: [
        {
          relPath: 'compiler/emission/src/emission/package_artifact.rs',
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

  const canonicalOwnerFixture = [
    {
      name: 'terminal_identity',
      relPath: 'artifact-identity/src/terminal.rs',
      regexp: /\bpub\s+fn\s+terminal_identity\s*\(/,
    },
  ];
  const ownerRequirementCases = [
    {
      name: 'accepts a declared canonical owner',
      textByPath: new Map([
        ['artifact-identity/src/terminal.rs', 'pub fn terminal_identity() {}\n'],
      ]),
      expectedFailures: 0,
    },
    {
      name: 'rejects a missing canonical owner file',
      textByPath: new Map(),
      expectedFailures: 1,
    },
    {
      name: 'rejects a canonical owner without its required definition',
      textByPath: new Map([
        ['artifact-identity/src/terminal.rs', 'pub fn other_identity() {}\n'],
      ]),
      expectedFailures: 1,
    },
  ];
  for (const testCase of ownerRequirementCases) {
    const ownerFailures = collectOwnerRequirementFailures(
      canonicalOwnerFixture,
      testCase.textByPath,
    );
    if (ownerFailures.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} owner failure(s), got ${ownerFailures.length}`,
      );
    }
  }

  const canonicalModelCases = [
    {
      name: 'allows independent canonical compile models',
      files: [
        {
          relPath: 'artifact-model/src/package_artifact.rs',
          text: 'pub struct PackageArtifact { pub package_id: String }\n',
        },
        {
          relPath: 'artifact-model/src/service_contract.rs',
          text: 'pub struct ServiceContract { pub service_id: String }\n',
        },
      ],
      expectedFailures: 0,
    },
    {
      name: 'rejects legacy aggregate embedding in canonical models',
      files: [
        {
          relPath: 'artifact-model/src/package_artifact.rs',
          text: 'pub struct PackageArtifact { pub legacy: PublicationAbiUnit }\n',
        },
        {
          relPath: 'artifact-model/src/service_contract.rs',
          text: 'pub struct ServiceContract { pub legacy: Option<ServiceUnit> }\n',
        },
      ],
      expectedFailures: 2,
    },
    {
      name: 'rejects ServiceUnit module dependency from canonical model',
      files: [
        {
          relPath: 'artifact-model/src/package_artifact.rs',
          text: 'use crate::service_unit::OperationTargetRef; pub struct PackageArtifact {}\n',
        },
        {
          relPath: 'artifact-model/src/service_contract.rs',
          text: 'pub struct ServiceContract {}\n',
        },
      ],
      expectedFailures: 1,
    },
  ];
  for (const testCase of canonicalModelCases) {
    const modelFailures = collectCanonicalCompileModelFailures(testCase.files);
    if (modelFailures.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} canonical model failure(s), got ${modelFailures.length}`,
      );
    }
  }

  const canonicalBoundaryContractFiles = ({
    projection = 'enum BoundaryCallableProjection { Available { operation_contract: BoundaryOperationContract, implementation_requirements: BoundaryImplementationRequirements } }\n',
    serviceContract = 'pub struct ServiceContract { pub operations: BTreeMap<ContractOperationId, BoundaryOperationDescriptor> }\n',
  } = {}) => [
    {
      relPath: 'artifact-model/src/boundary/projection.rs',
      text: projection,
    },
    {
      relPath: 'artifact-model/src/boundary/operation.rs',
      text: 'pub struct BoundaryOperationContract;\npub struct BoundaryOperationDescriptor { pub operation_id: ContractOperationId, pub stable_key: String, pub contract: BoundaryOperationContract }\n',
    },
    {
      relPath: 'artifact-model/src/service_contract.rs',
      text: serviceContract,
    },
  ];
  const canonicalBoundaryContractCases = [
    {
      name: 'accepts contract-agnostic package boundary projections and service descriptors',
      files: canonicalBoundaryContractFiles(),
      expectedFailures: 0,
    },
    {
      name: 'rejects operation descriptors in package boundary projections',
      files: canonicalBoundaryContractFiles({
        projection: 'enum BoundaryCallableProjection { Available { descriptor: BoundaryOperationDescriptor, implementation_requirements: BoundaryImplementationRequirements } }\n',
      }),
      expectedFailures: 1,
    },
    {
      name: 'rejects contract identities in package boundary projections',
      files: canonicalBoundaryContractFiles({
        projection: 'enum BoundaryCallableProjection { Available { operation_contract: BoundaryOperationContract, operation_id: ContractOperationId, stable_key: String, implementation_requirements: BoundaryImplementationRequirements } }\n',
      }),
      expectedFailures: 1,
    },
    {
      name: 'rejects body-only ServiceContract operations',
      files: canonicalBoundaryContractFiles({
        serviceContract: 'pub struct ServiceContract { pub operations: BTreeMap<ContractOperationId, BoundaryOperationContract> }\n',
      }),
      expectedFailures: 1,
    },
  ];
  for (const testCase of canonicalBoundaryContractCases) {
    const boundaryFailures = collectCanonicalBoundaryContractFailures(testCase.files);
    if (boundaryFailures.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} canonical boundary failure(s), got ${boundaryFailures.length}`,
      );
    }
  }

  const assertCallValidatorGraph = (name, entry, files, expected = {}) => {
    const diagnostics = collectFileIrCallValidatorGraphDiagnostics(entry, files);
    for (const key of [
      'ownerFailures',
      'shapeFailures',
      'delegationFailures',
      'duplicateViolations',
    ]) {
      const expectedCount = expected[key] ?? 0;
      if (diagnostics[key].length !== expectedCount) {
        failures.push(
          `${name}: expected ${expectedCount} ${key}, got ${diagnostics[key].length}`,
        );
      }
    }
  };

  const serviceCallRegistry = canonicalFileIrCallValidatorRegistry
    .find(({ name }) => name === 'service-call');
  const canonicalServiceCallFiles = canonicalFileIrCallValidatorFixture(serviceCallRegistry);
  assertCallValidatorGraph(
    'accepts required table-owned indexed service calls',
    serviceCallRegistry,
    canonicalServiceCallFiles,
  );
  assertCallValidatorGraph(
    'rejects optional service-call table ownership',
    serviceCallRegistry,
    mutateCanonicalFixture(
      canonicalServiceCallFiles,
      serviceCallRegistry.shapeRequirements[0],
      'pub struct ExternalRefTable { #[serde(default)] pub service_call_refs: Vec<ServiceCallRef> }\n',
    ),
    { shapeFailures: 1 },
  );
  assertCallValidatorGraph(
    'rejects inline service-call refs in instructions',
    serviceCallRegistry,
    mutateCanonicalFixture(
      canonicalServiceCallFiles,
      serviceCallRegistry.shapeRequirements[1],
      'enum CallTargetIr { ServiceCall { service_call_ref: ServiceCallRef } }\n',
    ),
    { shapeFailures: 1 },
  );

  const packageCallRegistry = canonicalFileIrCallValidatorRegistry
    .find(({ name }) => name === 'package-call');
  const canonicalPackageCallFiles = canonicalFileIrCallValidatorFixture(packageCallRegistry);
  assertCallValidatorGraph(
    'accepts canonical package-call owner graph',
    packageCallRegistry,
    canonicalPackageCallFiles,
  );
  assertCallValidatorGraph(
    'rejects missing package-call owner file',
    packageCallRegistry,
    canonicalPackageCallFiles.filter(({ relPath }) => relPath !== packageCallRegistry.owner.relPath),
    { ownerFailures: 1, shapeFailures: 2 },
  );
  assertCallValidatorGraph(
    'rejects missing package-call validator definition',
    packageCallRegistry,
    mutateCanonicalFixture(canonicalPackageCallFiles, packageCallRegistry.owner),
    { ownerFailures: 1 },
  );
  assertCallValidatorGraph(
    'rejects duplicate package-call validator definition',
    packageCallRegistry,
    [
      ...canonicalPackageCallFiles,
      {
        relPath: 'compiler/lowering/src/package_call_validation.rs',
        text: 'fn validate_file_ir_package_calls() {}\n',
      },
    ],
    { duplicateViolations: 1 },
  );
  for (const consumer of packageCallRegistry.consumers) {
    assertCallValidatorGraph(
      `rejects missing ${consumer.helper} delegation`,
      packageCallRegistry,
      mutateCanonicalFixture(canonicalPackageCallFiles, consumer),
      { delegationFailures: 1 },
    );
  }
  assertCallValidatorGraph(
    'rejects noncanonical package-call table shape',
    packageCallRegistry,
    mutateCanonicalFixture(
      canonicalPackageCallFiles,
      packageCallRegistry.shapeRequirements[0],
      'pub struct ExternalRefTable { pub package_callables: Vec<PackageSymbolRef> }\n',
    ),
    { shapeFailures: 1 },
  );
  assertCallValidatorGraph(
    'rejects incomplete PackageCallable target shape',
    packageCallRegistry,
    mutateCanonicalFixture(
      canonicalPackageCallFiles,
      packageCallRegistry.shapeRequirements[1],
      'enum CallTargetIr { PackageCallable { package_ref: PackageRefIr } }\n',
    ),
    { shapeFailures: 1 },
  );
  const validationMatrixRequirement = packageCallRegistry.shapeRequirements[2];
  assertCallValidatorGraph(
    'rejects incomplete package-call exact-set validation matrix',
    packageCallRegistry,
    mutateCanonicalFixture(
      canonicalPackageCallFiles,
      validationMatrixRequirement,
      validationMatrixRequirement.fixtureText.replace('FieldMismatch {}, ', ''),
    ),
    { shapeFailures: 1 },
  );

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
  failures.push(...deprecatedPackageAbiRustSymbolSelfTestFailures());

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
