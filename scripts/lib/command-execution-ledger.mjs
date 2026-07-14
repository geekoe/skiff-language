export const COMMAND_OWNER_CLASSES = Object.freeze({
  ATTACHED_PRIMITIVE: 'attached-primitive',
  OWNED_PROCESS_GROUP: 'owned-process-group',
  BROWSER: 'browser',
  MANAGED_COMPONENT: 'managed-component',
  SUPERVISOR: 'supervisor',
  BINARY_ADAPTER: 'binary-adapter',
  TIMEOUT_OWNER: 'timeout-owner',
  DOMAIN_ADAPTER: 'domain-adapter',
  MIGRATION_PENDING: 'migration-pending',
});

export const COMMAND_EXECUTION_LEDGER = deepFreeze([
  owner('scripts/lib/command-execution.mjs', 'spawn', 'spawnCommandChild',
    'attached-capture-spawn', 'spawnAttachedChild', 'attached-primitive',
    'canonical attached and capture spawn boundary'),
  owner('scripts/lib/owned-command.mjs', 'spawn', 'spawnOwnedChild',
    'owned-process-group', 'runOwnedCommand', 'owned-process-group',
    'detached process-group Abort/TERM/KILL owner'),
  owner('scripts/skiff.mjs', 'spawn', 'spawnBrowserChild',
    'browser-unref', 'openBrowser', 'browser',
    'detached browser launch is intentionally unrefed'),
  owner('scripts/skiff-instance.mjs', 'spawn', 'spawnManagedChild',
    'managed-component', 'startManagedProcess', 'managed-component',
    'managed component retains PID, PGID, and log descriptors'),
  owner('scripts/lib/isolated-test-runtime-instance.mjs', 'spawn', 'spawnSupervisorChild',
    'isolated-supervisor', 'isolatedInstanceOperations', 'supervisor',
    'isolated runtime lifecycle retains the supervisor child handle'),
  owner('scripts/lib/source-key.mjs', 'spawn', 'spawnGitBufferChild',
    'git-buffer', 'gitBuffer', 'binary-adapter',
    'source-key hashing requires binary git stdout'),
  owner('scripts/lib/source-key.mjs', 'spawn', 'spawnGitExitChild',
    'git-exit-code', 'gitExitCode', 'binary-adapter',
    'source-key comparison preserves git exit-code semantics'),
  owner('scripts/check-crate-public-api.mjs', 'spawn', 'spawnRustdocChild',
    'rustdoc-timeout', 'runCommand', 'timeout-owner',
    'rustdoc nightly probe owns timeout and kill behavior'),
  owner('scripts/lib/loop-risk-stress-node.mjs', 'execFile', 'execLoopRiskCpuSample',
    'loop-risk-cpu-sample', 'readProcessCpu', 'domain-adapter',
    'loop-risk stress samples runtime CPU with ps'),
  owner('scripts/lib/loop-risk-stress-node.mjs', 'execFile', 'execLoopRiskPgrep',
    'loop-risk-pgrep', 'findRuntimePids', 'domain-adapter',
    'explicit diagnostic pgrep preserves domain outcome semantics'),

  pending('scripts/skiff.mjs', 'spawn', 'spawnCredentialCapture',
    'credential-capture-pending', 'spawnCapture',
    'keychain and tar capture moves in Phase 5 commit two'),
  pending('scripts/skiff-instance.mjs', 'spawn', 'spawnInstanceCapture',
    'instance-status-capture-pending', 'capture',
    'lsof and ps outcome capture moves in Phase 5 commit two'),
  pending('scripts/check-runtime-crate-dag.mjs', 'spawn', 'spawnRuntimeDagCapture',
    'runtime-dag-capture-pending', 'run',
    'runtime Cargo metadata outcome moves in Phase 5 commit two'),
  pending('scripts/check-package-store-discovery.mjs', 'spawn', 'spawnPackageStoreSkiff',
    'package-store-skiff-pending', 'runSkiff',
    'package-store success adapter moves in Phase 5 commit two'),
  pending('scripts/check-package-store-discovery.mjs', 'spawn', 'spawnPackageStoreCommand',
    'package-store-command-pending', 'runCommand',
    'package-store general outcome adapter moves in Phase 5 commit two'),
  pending('scripts/check-package-store-discovery.mjs', 'spawn', 'spawnPackageStoreExpectedFailure',
    'package-store-expected-failure-pending', 'runSkiffExpectFailure',
    'package-store expected-failure adapter moves in Phase 5 commit two'),
  pending('scripts/lib/isolated-test-runtime-instance.mjs', 'spawn', 'spawnIsolatedStatusCapture',
    'isolated-status-capture-pending', 'runCommandCapture',
    'isolated status checked capture moves in Phase 5 commit two'),
  pending('scripts/lib/encrypted-storage-live-harness.mjs', 'spawn', 'spawnMongoshCapture',
    'mongosh-capture-pending', 'runCommandCapture',
    'mongosh checked capture moves in Phase 5 commit two'),
  pending('scripts/build-runtime-stack.mjs', 'spawn', 'spawnBuildStackCapture',
    'build-stack-capture-pending', 'capture',
    'build stack git checked capture moves in Phase 5 commit two'),
  pending('scripts/check-local-instance.mjs', 'spawn', 'spawnLocalCheckCapture',
    'local-check-capture-pending', 'runCapture',
    'local instance checked capture moves in Phase 5 commit two'),
  pending('scripts/check-compiler-crate-dag.mjs', 'spawn', 'spawnCompilerDagCapture',
    'compiler-dag-capture-pending', 'readCargoMetadata',
    'compiler Cargo metadata checked capture moves in Phase 5 commit two'),
  pending('scripts/package-live-test.mjs', 'spawn', 'spawnPackageLiveCapture',
    'package-live-capture-pending', 'runCli',
    'package live checked capture moves in Phase 5 commit two'),
]);

function pending(path, importedSymbol, localAlias, ownerId, ownerFunction, reason) {
  return owner(
    path,
    importedSymbol,
    localAlias,
    ownerId,
    ownerFunction,
    COMMAND_OWNER_CLASSES.MIGRATION_PENDING,
    reason,
  );
}

function owner(
  path,
  importedSymbol,
  localAlias,
  ownerId,
  ownerFunction,
  ownerClass,
  reason,
) {
  return {
    path,
    importedSymbol,
    localAlias,
    ownerId,
    ownerFunction,
    callCount: 1,
    ownerClass,
    reason,
  };
}

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) {
      deepFreeze(child);
    }
    Object.freeze(value);
  }
  return value;
}
