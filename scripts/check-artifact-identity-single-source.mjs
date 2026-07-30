#!/usr/bin/env node

import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
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
const terminalLegacyArtifactSymbolNames = Object.freeze([
  'PackageUnit',
  'ServiceUnit',
  'PublicationAbiUnit',
  'PackageTestAssembly',
  'ServiceDependencyConstraint',
  'ServiceDependencyOperationRef',
  'PublicationApiBinding',
  'PublicationApiSymbolKind',
  'PublicationConformanceFact',
  'PublicationOperationAbi',
  'PublicationPublicInstanceExport',
  'PublicationSchemaType',
  'PublicationSchemaTypeNameability',
  'SourceCallMethodIndexEntry',
  'SourceCallOperationIndexEntry',
]);
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
    name: 'ServiceProtocolIdentityProjection',
    relPath: 'artifact-identity/src/contract.rs',
    regexp: /\bpub\s+struct\s+ServiceProtocolIdentityProjection\b/,
  },
  {
    name: 'DeploymentArtifactIdentityProjection',
    relPath: 'artifact-identity/src/deployment.rs',
    regexp: /\bpub\s+struct\s+DeploymentArtifactIdentityProjection\b/,
  },
  {
    name: 'service_deployment_identity',
    relPath: 'artifact-identity/src/deployment.rs',
    regexp: /\bpub\s+fn\s+service_deployment_identity\s*\(/,
  },
  {
    name: 'assign_service_deployment_identity',
    relPath: 'artifact-identity/src/deployment.rs',
    regexp: /\bpub\s+fn\s+assign_service_deployment_identity\s*\(/,
  },
  {
    name: 'validate_service_deployment_identity',
    relPath: 'artifact-identity/src/deployment.rs',
    regexp: /\bpub\s+fn\s+validate_service_deployment_identity\s*\(/,
  },
  {
    name: 'AssemblyIdentityProjection',
    relPath: 'artifact-identity/src/runtime_assembly.rs',
    regexp: /\bpub\s+struct\s+AssemblyIdentityProjection\b/,
  },
  {
    name: 'runtime_assembly_identity',
    relPath: 'artifact-identity/src/runtime_assembly.rs',
    regexp: /\bpub\s+fn\s+runtime_assembly_identity\s*\(/,
  },
  {
    name: 'assign_runtime_assembly_identity',
    relPath: 'artifact-identity/src/runtime_assembly.rs',
    regexp: /\bpub\s+fn\s+assign_runtime_assembly_identity\s*\(/,
  },
  {
    name: 'validate_runtime_assembly_identity',
    relPath: 'artifact-identity/src/runtime_assembly.rs',
    regexp: /\bpub\s+fn\s+validate_runtime_assembly_identity\s*\(/,
  },
  {
    name: 'DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX\b/,
  },
  {
    name: 'ASSEMBLY_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+use\s+skiff_artifact_model::RUNTIME_ASSEMBLY_IDENTITY_PREFIX\s+as\s+ASSEMBLY_IDENTITY_PREFIX\s*;/,
  },
  {
    name: 'package_schema_type_id',
    relPath: 'artifact-identity/src/contract.rs',
    regexp: /\bpub\s+fn\s+package_schema_type_id\s*\(/,
  },
  {
    name: 'package_schema_index_identity',
    relPath: 'artifact-identity/src/contract.rs',
    regexp: /\bpub\s+fn\s+package_schema_index_identity\s*\(/,
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
    name: 'SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER\b/,
  },
  {
    name: 'PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX\b/,
  },
  {
    name: 'PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER\b/,
  },
  {
    name: 'PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX\b/,
  },
  {
    name: 'PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER\b/,
  },
  {
    name: 'PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX\b/,
  },
  {
    name: 'PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER\b/,
  },
  {
    name: 'PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX\b/,
  },
  {
    name: 'PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER',
    relPath: 'artifact-identity/src/constants.rs',
    regexp: /\bpub\s+const\s+PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER\b/,
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
    name: 'ArtifactRelativePath',
    relPath: 'artifact-identity/src/artifact_path.rs',
    regexp: /\bpub\s+struct\s+ArtifactRelativePath\b/,
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
  'ServiceProtocolIdentityProjection',
  'DeploymentArtifactIdentityProjection',
  'service_deployment_identity',
  'assign_service_deployment_identity',
  'validate_service_deployment_identity',
  'AssemblyIdentityProjection',
  'runtime_assembly_identity',
  'assign_runtime_assembly_identity',
  'DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX',
  'ASSEMBLY_IDENTITY_PREFIX',
  'package_schema_index_identity',
  'contract_operation_id',
  'BoundaryOperationContract',
  'BoundaryOperationDescriptor',
  'PackageArtifactLocalAbiIdentityProjection',
  'PackageArtifactBuildIdentityProjection',
  'package_artifact_local_abi_identity',
  'package_artifact_build_identity',
  'assign_package_artifact_identities',
  'validate_package_artifact_identities',
  'SERVICE_PROTOCOL_IDENTITY_PREFIX',
  'SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER',
  'PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX',
  'PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER',
  'PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX',
  'PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER',
  'PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX',
  'PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER',
  'PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX',
  'PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER',
  'ArtifactRelativePath',
  'canonical_file_ir_identity_bytes',
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
  'constants',
  'contract',
  'deployment',
  'error',
  'file_ir',
  'framing',
  'package_artifact',
  'runtime_assembly',
  'semantic',
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
    rustUse: Object.freeze({
      modulePath: 'skiff_artifact_identity',
      symbol: 'validate_package_artifact_identities',
    }),
  },
  {
    relPath: 'compiler/driver/pipeline/mod.rs',
    helper: 'compiler-owned std PackageArtifact identity validation',
    regexp: /\buse\s+skiff_artifact_identity::validate_package_artifact_identities\b/,
  },
];
const canonicalCompileModelPaths = Object.freeze([
  ['artifact-model/src/package_artifact.rs', 'PackageArtifact'],
  ['artifact-model/src/service_contract.rs', 'ServiceContract'],
]);
const canonicalDeploymentAssemblyModels = Object.freeze([
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'PackageArtifactRef',
    requiredFields: Object.freeze([
      'package_id',
      'package_version',
      'package_build_id',
      'package_local_abi_identity',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'ServiceContractRef',
    requiredFields: Object.freeze([
      'service_id',
      'contract_version',
      'service_protocol_identity',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'ServiceDeploymentRef',
    requiredFields: Object.freeze([
      'service_id',
      'contract_version',
      'deployment_revision',
      'deployment_artifact_identity',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'PackageRequirementKey',
    requiredFields: Object.freeze([
      'caller_package_build_id',
      'package_requirement_alias',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'ServiceRequirementKey',
    requiredFields: Object.freeze([
      'caller_package_build_id',
      'service_requirement_slot',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'PackageBinding',
    requiredFields: Object.freeze(['key', 'package', 'collection_name_mapping']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'ServiceSelectorBinding',
    requiredFields: Object.freeze(['key', 'contract']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'ServiceDeploymentOperationInput',
    requiredFields: Object.freeze(['contract_operation_id', 'package_callable_id']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'DeploymentOperationBinding',
    requiredFields: Object.freeze(['contract_operation_id', 'package_callable_id']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'IngressSelector',
    requiredFields: Object.freeze(['protocol', 'method', 'path']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'DeploymentIngressBinding',
    requiredFields: Object.freeze(['selector', 'gateway_entry_key']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'DeploymentDiagnosticText',
    requiredFields: Object.freeze(['display_name', 'notes']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'ServiceDeploymentInput',
    requiredFields: Object.freeze([
      'schema_version',
      'contract',
      'deployment_revision',
      'implementation',
      'operation_bindings',
      'package_bindings',
      'service_selectors',
      'gateway_entries',
      'ingress',
      'diagnostic_text',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'ServiceDeployment',
    requiredFields: Object.freeze([
      'schema_version',
      'contract',
      'deployment_revision',
      'deployment_artifact_identity',
      'implementation',
      'operation_bindings',
      'package_bindings',
      'service_selectors',
      'gateway_entries',
      'ingress',
      'diagnostic_text',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/runtime_assembly.rs',
    typeName: 'PackageCodeSlot',
    requiredFields: Object.freeze(['package']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/runtime_assembly.rs',
    typeName: 'CanonicalPackageLinkPlan',
    requiredFields: Object.freeze(['code_slots', 'package_links']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/runtime_assembly.rs',
    typeName: 'ResolvedServiceBinding',
    requiredFields: Object.freeze(['key', 'contract', 'provider', 'used_operations']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/runtime_assembly.rs',
    typeName: 'ServiceBindingTemplate',
    requiredFields: Object.freeze(['activation', 'bindings']),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/runtime_assembly.rs',
    typeName: 'ActivationTemplate',
    requiredFields: Object.freeze([
      'deployment',
      'implementation_package_build_id',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/runtime_assembly.rs',
    typeName: 'GatewayIngressBinding',
    requiredFields: Object.freeze([
      'selector',
      'deployment',
      'gateway_entry_key',
      'gateway_entry_identity',
    ]),
  }),
  Object.freeze({
    relPath: 'artifact-model/src/runtime_assembly.rs',
    typeName: 'RuntimeAssembly',
    requiredFields: Object.freeze([
      'schema_version',
      'assembly_identity',
      'roots',
      'resolved_deployments',
      'resolved_contracts',
      'resolved_packages',
      'package_link_plan',
      'service_binding_templates',
      'activation_templates',
      'gateway_ingress',
    ]),
  }),
]);
const canonicalDeploymentAssemblyIdentityNewtypes = Object.freeze([
  'DeploymentRevision',
  'DeploymentArtifactIdentity',
  'AssemblyIdentity',
]);
const retiredDeploymentAssemblyBindingTypes = Object.freeze([
  'ResourceBinding',
  'RuntimeCapabilityBinding',
]);
const canonicalDeploymentAssemblyEnums = Object.freeze([
  Object.freeze({
    relPath: 'artifact-model/src/deployment.rs',
    typeName: 'IngressProtocol',
    variants: Object.freeze(['Http', 'WebSocket']),
  }),
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
  failures.push(...collectTerminalLegacyArtifactSymbolFailures(files));
  failures.push(...collectCanonicalCompileModelFailures(files));
  failures.push(...collectCanonicalDeploymentAssemblyModelFailures(files));
  failures.push(...collectCanonicalBoundaryContractFailures(files));
  failures.push(...collectCanonicalFileIrCallValidatorFailures(files));

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
    const delegates = requirement.rustUse === undefined
      ? requirement.regexp.test(productionText)
      : rustUseImportsSymbol(
        productionText,
        requirement.rustUse.modulePath,
        requirement.rustUse.symbol,
      );
    if (!delegates) {
      failures.push(
        `${requirement.relPath} must delegate ${requirement.helper} to skiff_artifact_identity`,
      );
    }
  }
  return failures;
}

function rustUseImportsSymbol(text, modulePath, symbol) {
  const canonicalModulePath = modulePath.replace(/\s/g, '');
  const canonicalSymbol = symbol.replace(/\s/g, '');
  const source = stripRustComments(text);
  for (const match of source.matchAll(/\buse\s+([^;]+);/gs)) {
    const useTree = match[1].replace(/\s/g, '');
    const direct = new RegExp(
      `^${escapeRegExp(canonicalModulePath)}::${escapeRegExp(canonicalSymbol)}(?:as\\w+)?$`,
    );
    if (direct.test(useTree)) {
      return true;
    }
    const groupedPrefix = `${canonicalModulePath}::{`;
    if (!useTree.startsWith(groupedPrefix) || !useTree.endsWith('}')) {
      continue;
    }
    const members = splitTopLevelUseTreeMembers(
      useTree.slice(groupedPrefix.length, -1),
    );
    if (
      members.some((member) => new RegExp(
        `^${escapeRegExp(canonicalSymbol)}(?:as\\w+)?$`,
      ).test(member))
    ) {
      return true;
    }
  }
  return false;
}

function splitTopLevelUseTreeMembers(text) {
  const members = [];
  let start = 0;
  let depth = 0;
  for (let index = 0; index < text.length; index += 1) {
    if (text[index] === '{') {
      depth += 1;
    } else if (text[index] === '}') {
      depth -= 1;
    } else if (text[index] === ',' && depth === 0) {
      members.push(text.slice(start, index));
      start = index + 1;
    }
  }
  members.push(text.slice(start));
  return members.filter((member) => member.length > 0);
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
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

function collectTerminalLegacyArtifactSymbolFailures(files) {
  const failures = [];
  const legacyTypeRegexp = new RegExp(
    `\\b(${terminalLegacyArtifactSymbolNames.join('|')})\\b`,
    'g',
  );
  for (const file of files) {
    const identifierView = rustIdentifierView(file.text);
    for (const match of identifierView.matchAll(legacyTypeRegexp)) {
      failures.push(
        `${file.relPath}:${lineNumberAt(identifierView, match.index ?? 0)} contains terminal legacy artifact symbol ${match[1]}`,
      );
    }
  }
  return failures;
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

function collectCanonicalDeploymentAssemblyModelFailures(files) {
  const failures = [];
  const byPath = new Map(files.map((file) => [file.relPath, file]));
  const canonicalTypeNames = new Set(
    canonicalDeploymentAssemblyModels.map(({ typeName }) => typeName),
  );

  for (const file of files) {
    const text = stripRustComments(stripInlineTestModules(file.text));
    for (const typeName of retiredDeploymentAssemblyBindingTypes) {
      if (new RegExp(`\\bpub\\s+struct\\s+${typeName}\\b`).test(text)) {
        failures.push(`${file.relPath} retains retired ${typeName} definition`);
      }
    }
  }

  for (const model of canonicalDeploymentAssemblyModels) {
    const file = byPath.get(model.relPath);
    if (file === undefined) {
      failures.push(`${model.relPath} is missing canonical ${model.typeName}`);
      continue;
    }
    const text = stripRustComments(stripInlineTestModules(file.text));
    const body = rustStructBody(text, model.typeName);
    if (body === undefined) {
      failures.push(`${model.relPath} is missing canonical ${model.typeName} definition`);
      continue;
    }
    if (!/serde\s*\([^)]*rename_all\s*=\s*"camelCase"[^)]*deny_unknown_fields[^)]*\)/s.test(text.slice(Math.max(0, body.start - 300), body.start))) {
      failures.push(`${model.relPath} ${model.typeName} must use strict camelCase wire`);
    }
    for (const field of model.requiredFields) {
      if (!new RegExp(`\\bpub\\s+${field}\\s*:`).test(body.text)) {
        failures.push(`${model.relPath} ${model.typeName} is missing canonical field ${field}`);
      }
    }
    const declaredFields = [...body.text.matchAll(/\bpub\s+(\w+)\s*:/g)]
      .map((match) => match[1]);
    const unexpectedFields = declaredFields.filter(
      (field) => !model.requiredFields.includes(field),
    );
    if (unexpectedFields.length > 0) {
      failures.push(
        `${model.relPath} ${model.typeName} has noncanonical field(s): ${unexpectedFields.join(', ')}`,
      );
    }
    if (/\b(?:PublicationAbiUnit|PackageUnit|ServiceUnit)\b/.test(body.text)) {
      failures.push(`${model.relPath} ${model.typeName} embeds a legacy aggregate`);
    }
    if (/\b(?:artifact_path|filesystem_path|service_assembly)\b/.test(body.text)) {
      failures.push(`${model.relPath} ${model.typeName} embeds a path or raw service assembly`);
    }
    if (/\b(?:BoundaryOperationDescriptor|BoundaryOperationContract|ContractSchemaType)\b/.test(body.text)) {
      failures.push(`${model.relPath} ${model.typeName} duplicates ServiceContract-owned descriptors`);
    }
  }

  for (const model of canonicalDeploymentAssemblyEnums) {
    const file = byPath.get(model.relPath);
    if (file === undefined) {
      failures.push(`${model.relPath} is missing canonical ${model.typeName}`);
      continue;
    }
    const text = stripRustComments(stripInlineTestModules(file.text));
    const body = rustEnumBody(text, model.typeName);
    if (body === undefined) {
      failures.push(`${model.relPath} is missing canonical ${model.typeName} definition`);
      continue;
    }
    if (!/serde\s*\([^)]*rename_all\s*=\s*"camelCase"[^)]*\)/s.test(text.slice(Math.max(0, body.start - 300), body.start))) {
      failures.push(`${model.relPath} ${model.typeName} must use canonical camelCase wire`);
    }
    const variants = [...body.text.matchAll(/\b([A-Z]\w*)\s*,/g)]
      .map((match) => match[1]);
    if (
      variants.length !== model.variants.length
      || variants.some((variant, index) => variant !== model.variants[index])
    ) {
      failures.push(
        `${model.relPath} ${model.typeName} variants must be ${model.variants.join(', ')}`,
      );
    }
  }

  for (const file of files) {
    if (!isProductionRustFile(file.relPath)) {
      continue;
    }
    const text = stripRustComments(stripInlineTestModules(file.text));
    for (const typeName of canonicalTypeNames) {
      const owner = canonicalDeploymentAssemblyModels.find(
        (model) => model.typeName === typeName,
      ).relPath;
      if (file.relPath !== owner && new RegExp(`\\bpub\\s+struct\\s+${typeName}\\b`).test(text)) {
        failures.push(`${file.relPath} repeats canonical ${typeName} owned by ${owner}`);
      }
    }
    for (const model of canonicalDeploymentAssemblyEnums) {
      if (
        file.relPath !== model.relPath
        && new RegExp(`\\bpub\\s+enum\\s+${model.typeName}\\b`).test(text)
      ) {
        failures.push(
          `${file.relPath} repeats canonical ${model.typeName} owned by ${model.relPath}`,
        );
      }
    }
  }

  for (const typeName of canonicalDeploymentAssemblyIdentityNewtypes) {
    const owner = 'artifact-model/src/compile_identity.rs';
    let ownerCount = 0;
    for (const file of files) {
      if (!isProductionRustFile(file.relPath)) {
        continue;
      }
      const text = stripRustComments(stripInlineTestModules(file.text));
      const macro = new RegExp(`\\bstring_identity!\\s*\\(\\s*${typeName}\\s*\\)`, 'g');
      const explicit = new RegExp(`\\bpub\\s+struct\\s+${typeName}\\b`, 'g');
      const count = [...text.matchAll(macro)].length + [...text.matchAll(explicit)].length;
      if (count === 0) {
        continue;
      }
      if (file.relPath === owner) {
        ownerCount += count;
      } else {
        failures.push(`${file.relPath} repeats canonical identity ${typeName} owned by ${owner}`);
      }
    }
    if (ownerCount !== 1) {
      failures.push(`${owner} must own exactly one ${typeName} definition, got ${ownerCount}`);
    }
  }
  return failures;
}

function rustStructBody(text, typeName) {
  const declaration = new RegExp(`\\bpub\\s+struct\\s+${typeName}\\s*\\{`).exec(text);
  if (declaration === null) {
    return undefined;
  }
  const open = text.indexOf('{', declaration.index);
  const close = matchingBraceIndex(text, open);
  if (close === -1) {
    return undefined;
  }
  return { start: declaration.index, text: text.slice(open + 1, close) };
}

function rustEnumBody(text, typeName) {
  const declaration = new RegExp(`\\bpub\\s+enum\\s+${typeName}\\s*\\{`).exec(text);
  if (declaration === null) {
    return undefined;
  }
  const open = text.indexOf('{', declaration.index);
  const close = matchingBraceIndex(text, open);
  if (close === -1) {
    return undefined;
  }
  return { start: declaration.index, text: text.slice(open + 1, close) };
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
  const terminalLegacyCases = [
    {
      name: 'accepts current artifact types',
      files: [{
        relPath: 'artifact-model/src/current.rs',
        text: 'pub struct PackageArtifact;\n',
      }],
      expectedFailures: 0,
    },
    {
      name: 'rejects every terminal legacy artifact symbol in production code',
      files: [{
        relPath: 'runtime/model/src/legacy.rs',
        text: [
          'pub struct PublicationApiBinding;',
          'pub enum PublicationApiSymbolKind {}',
          'pub struct PublicationConformanceFact;',
          'pub struct PublicationOperationAbi;',
          'pub struct PublicationPublicInstanceExport;',
          'pub struct PublicationSchemaType;',
          'pub enum PublicationSchemaTypeNameability {}',
          'pub struct SourceCallMethodIndexEntry;',
          'pub struct SourceCallOperationIndexEntry;',
          '',
        ].join('\n'),
      }],
      expectedFailures: 9,
    },
    {
      name: 'rejects terminal legacy type in test code',
      files: [{
        relPath: 'artifact-model/src/tests.rs',
        text: '#[test]\nfn legacy() { let _: Option<ServiceDependencyConstraint> = None; }\n',
      }],
      expectedFailures: 1,
    },
    {
      name: 'ignores terminal legacy names in comments and strings',
      files: [{
        relPath: 'artifact-model/src/current.rs',
        text: '// PackageUnit\nconst NOTE: &str = "ServiceUnit";\nconst RAW: &str = r#"PublicationAbiUnit"#;\nconst BYTES: &[u8] = b"PackageTestAssembly";\n',
      }],
      expectedFailures: 0,
    },
  ];
  const failures = [];
  for (const testCase of terminalLegacyCases) {
    const legacyFailures = collectTerminalLegacyArtifactSymbolFailures(testCase.files);
    if (legacyFailures.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} terminal legacy failure(s), got ${legacyFailures.length}`,
      );
    }
  }

  const cases = [
    {
      name: 'allows definitions in their declared owner modules',
      files: [
        {
          relPath: 'artifact-identity/src/framing.rs',
          text: 'pub fn framed_identity() {}\n',
        },
        {
          relPath: 'canonical-json/src/lib.rs',
          text: 'pub fn canonical_json_value() {}\n',
        },
      ],
      expectedViolations: 0,
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
  ];

  for (const testCase of cases) {
    const violations = collectOwnedDefinitionViolations(testCase.files);
    if (violations.length !== testCase.expectedViolations) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedViolations} violation(s), got ${violations.length}`,
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

  const canonicalDeploymentText = `
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageArtifactRef {
  pub package_id: String, pub package_version: String, pub package_build_id: Build,
  pub package_local_abi_identity: Abi,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractRef {
  pub service_id: String, pub contract_version: String, pub service_protocol_identity: Identity,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentRef {
  pub service_id: String, pub contract_version: String, pub deployment_revision: Revision,
  pub deployment_artifact_identity: Identity,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageRequirementKey {
  pub caller_package_build_id: Build, pub package_requirement_alias: String,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceRequirementKey {
  pub caller_package_build_id: Build, pub service_requirement_slot: u32,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageBinding {
  pub key: PackageRequirementKey, pub package: PackageArtifactRef,
  pub collection_name_mapping: Map,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceSelectorBinding { pub key: ServiceRequirementKey, pub contract: ServiceContractRef }
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentOperationInput {
  pub contract_operation_id: OperationId, pub package_callable_id: CallableId,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentOperationBinding {
  pub contract_operation_id: OperationId, pub package_callable_id: CallableId,
}
#[serde(rename_all = "camelCase")]
pub enum IngressProtocol { Http, WebSocket, }
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngressSelector {
  pub protocol: Protocol, pub method: Option<String>, pub path: String,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentIngressBinding {
  pub selector: IngressSelector, pub gateway_entry_key: GatewayEntryKey,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentDiagnosticText { pub display_name: String, pub notes: Map }
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentInput {
  pub schema_version: String, pub contract: Ref, pub deployment_revision: Revision,
  pub implementation: Ref, pub operation_bindings: Vec<Op>, pub package_bindings: Vec<Pkg>,
  pub service_selectors: Vec<Svc>, pub gateway_entries: Map, pub ingress: Vec<Ingress>,
  pub diagnostic_text: Text,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeployment {
  pub schema_version: String, pub contract: Ref, pub deployment_revision: Revision,
  pub deployment_artifact_identity: Identity, pub implementation: Ref,
  pub operation_bindings: Vec<Op>, pub package_bindings: Vec<Pkg>,
  pub service_selectors: Vec<Svc>, pub gateway_entries: Map, pub ingress: Vec<Ingress>,
  pub diagnostic_text: Text,
}
`;
  const canonicalAssemblyText = `
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCodeSlot { pub package: PackageArtifactRef }
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalPackageLinkPlan { pub code_slots: Vec<PackageCodeSlot>, pub package_links: Vec<PackageBinding> }
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedServiceBinding {
  pub key: ServiceRequirementKey, pub contract: ServiceContractRef,
  pub provider: ServiceDeploymentRef, pub used_operations: Vec<OperationId>,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceBindingTemplate {
  pub activation: ServiceDeploymentRef, pub bindings: Vec<ResolvedServiceBinding>,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationTemplate {
  pub deployment: ServiceDeploymentRef, pub implementation_package_build_id: Build,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayIngressBinding {
  pub selector: IngressSelector, pub deployment: ServiceDeploymentRef,
  pub gateway_entry_key: GatewayEntryKey, pub gateway_entry_identity: GatewayEntryIdentity,
}
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssembly {
  pub schema_version: String, pub assembly_identity: Identity, pub roots: Vec<Ref>,
  pub resolved_deployments: Vec<Ref>, pub resolved_contracts: Vec<Ref>,
  pub resolved_packages: Vec<Ref>, pub package_link_plan: Plan,
  pub service_binding_templates: Vec<ServiceTemplate>,
  pub activation_templates: Vec<ActivationTemplate>, pub gateway_ingress: Vec<Ingress>,
}
`;
  const canonicalLinkPlanText = `#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalPackageLinkPlan { pub code_slots: Vec<PackageCodeSlot>, pub package_links: Vec<PackageBinding> }
`;
  const canonicalServiceTemplateText = `#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceBindingTemplate {
  pub activation: ServiceDeploymentRef, pub bindings: Vec<ResolvedServiceBinding>,
}
`;
  const canonicalActivationTemplateText = `#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationTemplate {
  pub deployment: ServiceDeploymentRef, pub implementation_package_build_id: Build,
}
`;
  const canonicalIdentityText = canonicalDeploymentAssemblyIdentityNewtypes
    .map((name) => `string_identity!(${name});`)
    .join('\n');
  const canonicalDeploymentAssemblyFiles = ({
    deployment = canonicalDeploymentText,
    assembly = canonicalAssemblyText,
    extra = [],
  } = {}) => [
    { relPath: 'artifact-model/src/deployment.rs', text: deployment },
    { relPath: 'artifact-model/src/runtime_assembly.rs', text: assembly },
    { relPath: 'artifact-model/src/compile_identity.rs', text: canonicalIdentityText },
    ...extra,
  ];
  const deploymentAssemblyCases = [
    {
      name: 'accepts canonical deployment and assembly owners',
      files: canonicalDeploymentAssemblyFiles(),
      expectedFailures: 0,
    },
    {
      name: 'rejects retired deployment policy field',
      files: canonicalDeploymentAssemblyFiles({
        deployment: canonicalDeploymentText.replaceAll(
          'pub diagnostic_text: Text,',
          'pub policy: Policy, pub diagnostic_text: Text,',
        ),
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects retired deployment resource and runtime capability fields',
      files: canonicalDeploymentAssemblyFiles({
        deployment: canonicalDeploymentText.replaceAll(
          'pub diagnostic_text: Text,',
          'pub resource_bindings: Vec<Resource>, pub runtime_capability_bindings: Vec<Capability>, pub diagnostic_text: Text,',
        ),
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects retired deployment binding type definitions',
      files: canonicalDeploymentAssemblyFiles({
        deployment: `${canonicalDeploymentText}
pub struct ResourceBinding {}
pub struct RuntimeCapabilityBinding {}
`,
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects retired activation template policy field',
      files: canonicalDeploymentAssemblyFiles({
        assembly: canonicalAssemblyText.replace(
          'pub deployment: ServiceDeploymentRef, pub implementation_package_build_id: Build,',
          'pub deployment: ServiceDeploymentRef, pub implementation_package_build_id: Build, pub policy: Policy,',
        ),
      }),
      expectedFailures: 1,
    },
    {
      name: 'rejects retired activation template resource field',
      files: canonicalDeploymentAssemblyFiles({
        assembly: canonicalAssemblyText.replace(
          'pub deployment: ServiceDeploymentRef, pub implementation_package_build_id: Build,',
          'pub deployment: ServiceDeploymentRef, pub implementation_package_build_id: Build, pub resource_bindings: Vec<Resource>,',
        ),
      }),
      expectedFailures: 1,
    },
    {
      name: 'rejects renamed canonical assembly field',
      files: canonicalDeploymentAssemblyFiles({
        assembly: canonicalAssemblyText.replace('pub package_link_plan:', 'pub linked_plan:'),
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects host regression in deployment ingress selector',
      files: canonicalDeploymentAssemblyFiles({
        deployment: canonicalDeploymentText.replace(
          'pub protocol: Protocol, pub method:',
          'pub protocol: Protocol, pub host: String, pub method:',
        ),
      }),
      expectedFailures: 1,
    },
    ...['protocol', 'method', 'path'].map((field) => ({
      name: `rejects missing canonical deployment ingress selector ${field}`,
      files: canonicalDeploymentAssemblyFiles({
        deployment: canonicalDeploymentText.replace(
          new RegExp(`\\s*pub ${field}: [^,]+,`),
          '',
        ),
      }),
      expectedFailures: 1,
    })),
    {
      name: 'rejects renamed canonical package link-plan leaf',
      files: canonicalDeploymentAssemblyFiles({
        assembly: canonicalAssemblyText.replace('pub code_slots:', 'pub linked_code_slots:'),
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects duplicate canonical package link-plan owner',
      files: canonicalDeploymentAssemblyFiles({
        extra: [{
          relPath: 'runtime/model/src/package_link_plan.rs',
          text: canonicalLinkPlanText,
        }],
      }),
      expectedFailures: 1,
    },
    {
      name: 'rejects moved canonical package link-plan owner',
      files: canonicalDeploymentAssemblyFiles({
        assembly: canonicalAssemblyText.replace(canonicalLinkPlanText, ''),
        extra: [{
          relPath: 'runtime/model/src/package_link_plan.rs',
          text: canonicalLinkPlanText,
        }],
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects legacy aggregate embedded in package link plan',
      files: canonicalDeploymentAssemblyFiles({
        assembly: canonicalAssemblyText.replace(
          'pub package_links: Vec<PackageBinding>',
          'pub package_links: Vec<PackageBinding>, pub legacy: PackageUnit',
        ),
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects renamed service and activation template leaves',
      files: canonicalDeploymentAssemblyFiles({
        assembly: canonicalAssemblyText
          .replace('pub bindings:', 'pub resolved_bindings:')
          .replace('pub implementation_package_build_id:', 'pub implementation_build:'),
      }),
      expectedFailures: 4,
    },
    {
      name: 'rejects duplicate service and activation template owners',
      files: canonicalDeploymentAssemblyFiles({
        extra: [{
          relPath: 'runtime/model/src/templates.rs',
          text: canonicalServiceTemplateText + canonicalActivationTemplateText,
        }],
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects moved service and activation template owners',
      files: canonicalDeploymentAssemblyFiles({
        assembly: canonicalAssemblyText
          .replace(canonicalServiceTemplateText, '')
          .replace(canonicalActivationTemplateText, ''),
        extra: [{
          relPath: 'runtime/model/src/templates.rs',
          text: canonicalServiceTemplateText + canonicalActivationTemplateText,
        }],
      }),
      expectedFailures: 4,
    },
    {
      name: 'rejects legacy package public path deployment input',
      files: canonicalDeploymentAssemblyFiles({
        deployment: canonicalDeploymentText.replace(
          'pub contract_operation_id: OperationId, pub package_callable_id: CallableId,',
          'pub contract_operation_id: OperationId, pub package_public_path: String,',
        ),
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects second deployment operation input owner',
      files: canonicalDeploymentAssemblyFiles({
        extra: [{
          relPath: 'deployment/model/src/operation_input.rs',
          text: `#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentOperationInput {
  pub contract_operation_id: OperationId, pub package_callable_id: CallableId,
}
`,
        }],
      }),
      expectedFailures: 1,
    },
    {
      name: 'rejects legacy aggregate embedded in deployment',
      files: canonicalDeploymentAssemblyFiles({
        deployment: canonicalDeploymentText.replace(
          'pub diagnostic_text: Text,',
          'pub diagnostic_text: Text, pub legacy: ServiceUnit,',
        ),
      }),
      expectedFailures: 2,
    },
    {
      name: 'rejects moved or repeated deployment owner',
      files: canonicalDeploymentAssemblyFiles({
        extra: [{
          relPath: 'runtime/model/src/deployment.rs',
          text: 'pub struct ServiceDeployment {}\n',
        }],
      }),
      expectedFailures: 1,
    },
    {
      name: 'rejects second assembly identity owner',
      files: canonicalDeploymentAssemblyFiles({
        extra: [{
          relPath: 'runtime/model/src/identity.rs',
          text: 'pub struct AssemblyIdentity(String);\n',
        }],
      }),
      expectedFailures: 1,
    },
  ];
  for (const testCase of deploymentAssemblyCases) {
    const modelFailures = collectCanonicalDeploymentAssemblyModelFailures(testCase.files);
    if (modelFailures.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} deployment/assembly failure(s), got ${modelFailures.length}`,
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

  const canonicalDependencyAdapterRequirement = [{
    relPath: 'compiler/driver/source_compile/canonical_dependencies.rs',
    helper: 'canonical package dependency identity validation',
    rustUse: {
      modulePath: 'skiff_artifact_identity',
      symbol: 'validate_package_artifact_identities',
    },
  }];
  const canonicalDependencyImportCases = [
    {
      name: 'allows grouped canonical dependency identity import',
      text: `use skiff_artifact_identity::{
  package_artifact_ref,
  validate_package_artifact_identities,
};
`,
      expectedFailures: 0,
    },
    {
      name: 'allows split canonical dependency identity import',
      text: `use skiff_artifact_identity::package_artifact_ref;
use skiff_artifact_identity::validate_package_artifact_identities;
`,
      expectedFailures: 0,
    },
    {
      name: 'rejects grouped import without canonical dependency identity symbol',
      text: `use skiff_artifact_identity::{package_artifact_ref, validate_service_deployment_identity};
`,
      expectedFailures: 1,
    },
    {
      name: 'rejects split import from a noncanonical identity module',
      text: `use skiff_artifact_identity::package_artifact_ref;
use local_identity::validate_package_artifact_identities;
`,
      expectedFailures: 1,
    },
  ];
  for (const testCase of canonicalDependencyImportCases) {
    const importFailures = collectAdapterRequirementFailures(
      canonicalDependencyAdapterRequirement,
      new Map([[
        canonicalDependencyAdapterRequirement[0].relPath,
        testCase.text,
      ]]),
    );
    if (importFailures.length !== testCase.expectedFailures) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedFailures} adapter failure(s), got ${importFailures.length}`,
      );
    }
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

function rustIdentifierView(text) {
  const output = text.split('');
  const mask = (start, end) => {
    for (let index = start; index < end; index += 1) {
      if (output[index] !== '\n') {
        output[index] = ' ';
      }
    }
  };
  let index = 0;
  while (index < text.length) {
    if (text.startsWith('//', index)) {
      const end = text.indexOf('\n', index + 2);
      const stop = end === -1 ? text.length : end;
      mask(index, stop);
      index = stop;
      continue;
    }
    if (text.startsWith('/*', index)) {
      let depth = 1;
      let cursor = index + 2;
      while (cursor < text.length && depth > 0) {
        if (text.startsWith('/*', cursor)) {
          depth += 1;
          cursor += 2;
        } else if (text.startsWith('*/', cursor)) {
          depth -= 1;
          cursor += 2;
        } else {
          cursor += 1;
        }
      }
      mask(index, cursor);
      index = cursor;
      continue;
    }
    const rawString = /^(?:br|r)(#{0,16})"/.exec(text.slice(index));
    if (rawString !== null) {
      const terminator = `"${rawString[1]}`;
      const contentStart = index + rawString[0].length;
      const close = text.indexOf(terminator, contentStart);
      const end = close === -1 ? text.length : close + terminator.length;
      mask(index, end);
      index = end;
      continue;
    }
    const quoteOffset = text[index] === 'b' && text[index + 1] === '"' ? 1 : 0;
    if (text[index + quoteOffset] === '"') {
      let cursor = index + quoteOffset + 1;
      while (cursor < text.length) {
        if (text[cursor] === '\\') {
          cursor += 2;
        } else if (text[cursor] === '"') {
          cursor += 1;
          break;
        } else {
          cursor += 1;
        }
      }
      mask(index, cursor);
      index = cursor;
      continue;
    }
    index += 1;
  }
  return output.join('');
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
