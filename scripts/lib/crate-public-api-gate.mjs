import { checkPublicApi } from './crate-public-api-graph.mjs';
import {
  MANAGED_CRATE_NAMES,
  publicApiConfigForCrate,
} from './crate-public-api-policy.mjs';
import {
  buildRustdocJson,
  cargoMetadata,
  probeCargoNightly,
  readRustdocJson,
} from './crate-public-api-rustdoc.mjs';

const defaultDependencies = Object.freeze({
  buildRustdocJson,
  cargoMetadata,
  checkPublicApi,
  probeCargoNightly,
  readRustdocJson,
});

export async function runCratePublicApiGate({
  dependencies = defaultDependencies,
  env,
  options,
  report,
  root,
}) {
  const metadata = await dependencies.cargoMetadata({ env, root });
  const packages = options.allConfigured
    ? resolveConfiguredPackages(metadata, MANAGED_CRATE_NAMES)
    : resolveExplicitPackage(metadata, options.crateName);
  if (packages.length === 0) {
    report({ kind: 'skip', crateName: options.crateName });
    return { exitCode: 0, violationCount: 0 };
  }

  const nightlyProbe = await dependencies.probeCargoNightly({ env, root });
  let violationCount = 0;
  for (const packageInfo of packages) {
    const crateName = packageInfo.name;
    const extraAllowedCrates = options.allConfigured ? [] : options.extraAllowedCrates;
    const config = publicApiConfigForCrate(crateName, extraAllowedCrates);
    if (!nightlyProbe.available) {
      report({ kind: 'warning', code: 'nightly-unavailable', crateName });
    }
    const build = await dependencies.buildRustdocJson({
      crateName,
      env,
      nightlyProbe,
      root,
    });
    if (build.fallbackLabel !== undefined) {
      report({
        kind: 'warning',
        code: 'rustdoc-fallback-succeeded',
        crateName,
        label: build.fallbackLabel,
      });
    }
    const rustdoc = await dependencies.readRustdocJson({ metadata, packageInfo });
    const result = dependencies.checkPublicApi(rustdoc, {
      crateName,
      allowedCrates: config.allowedCrates,
    });
    report({ kind: 'crate-result', crateName, config, result });
    violationCount += result.violations.length;
  }
  return {
    exitCode: violationCount > 0 ? 1 : 0,
    violationCount,
  };
}

export function resolveConfiguredPackages(metadata, configuredNames) {
  const packageByName = new Map(metadata.packages.map((pkg) => [pkg.name, pkg]));
  const missing = configuredNames.filter((crateName) => !packageByName.has(crateName));
  if (missing.length > 0) {
    throw new Error(
      `configured public API crate(s) missing from workspace: ${missing.join(', ')}`,
    );
  }
  return configuredNames.map((crateName) => packageByName.get(crateName));
}

function resolveExplicitPackage(metadata, crateName) {
  const packageInfo = metadata.packages.find((pkg) => pkg.name === crateName);
  return packageInfo === undefined ? [] : [packageInfo];
}
