import type {
  ActivationLookup,
  LoadedRouterArtifacts,
  ServiceVersionBuildBinding,
  RuntimeControlMetadata
} from '../artifacts/loadArtifactRoot.js';
import { buildActivationLookup } from '../artifacts/activationLookup.js';
import type { LoadedManifest } from '../manifest/types.js';

export interface RouterActiveSnapshot {
  activationByServiceOperation: ActivationLookup;
  control?: RuntimeControlMetadata;
  manifest: LoadedManifest;
  versionByService?: ReadonlyMap<string, ReadonlyMap<string, ServiceVersionBuildBinding>>;
}

export class RouterActiveSnapshotStore {
  private snapshot: RouterActiveSnapshot;

  constructor(snapshot: RouterActiveSnapshot) {
    this.snapshot = snapshot;
  }

  get(): RouterActiveSnapshot {
    return this.snapshot;
  }

  replace(snapshot: RouterActiveSnapshot): void {
    this.snapshot = snapshot;
  }
}

export function snapshotFromArtifacts(artifacts: LoadedRouterArtifacts): RouterActiveSnapshot {
  return {
    activationByServiceOperation: artifacts.activationByServiceOperation,
    control: artifacts.control,
    manifest: artifacts.manifest,
    ...(artifacts.versionByService !== undefined
      ? { versionByService: artifacts.versionByService }
      : {})
  };
}

export function snapshotFromManifest(manifest: LoadedManifest): RouterActiveSnapshot {
  return {
    activationByServiceOperation: buildActivationLookup([]),
    manifest
  };
}

export function summarizeRouterActiveSnapshot(
  snapshot: RouterActiveSnapshot
): Record<string, unknown> {
  const summary: Record<string, unknown> = {
    manifest: {
      serviceId: snapshot.manifest.service.id,
      revisionId: snapshot.manifest.service.revisionId,
      protocolIdentity: snapshot.manifest.service.protocolIdentity,
      httpRoutes: snapshot.manifest.httpRouteEntries.map((entry) => ({
        serviceId: entry.serviceId,
        method: entry.method,
        path: entry.path
      })),
      rawHttpServices: snapshot.manifest.rawHttpEntries.map((entry) => entry.serviceId),
      websocketPaths: snapshot.manifest.websocketEntries.map((entry) => entry.path)
    }
  };
  if (snapshot.control) {
    summary.artifact = {
      artifactRoots: snapshot.control.artifactRoots,
      ...(snapshot.control.devReload !== undefined
        ? { devReload: snapshot.control.devReload }
        : {}),
      ...(snapshot.control.mode !== undefined ? { mode: snapshot.control.mode } : {}),
      ...(snapshot.control.generation !== undefined
        ? { generation: snapshot.control.generation }
        : {}),
      ...(snapshot.control.fingerprint !== undefined
        ? { fingerprint: snapshot.control.fingerprint }
        : {}),
      ...(snapshot.control.serviceBuilds !== undefined
        ? { serviceBuilds: snapshot.control.serviceBuilds }
        : {}),
      serviceConfigCount: snapshot.control.serviceConfig?.length ?? 0,
      ...(snapshot.control.telemetry !== undefined
        ? { telemetry: summarizeTelemetry(snapshot.control.telemetry) }
        : {}),
      ...(snapshot.control.fileBackend !== undefined
        ? { fileBackend: summarizeFileBackend(snapshot.control.fileBackend) }
        : {})
    };
  }
  if (snapshot.versionByService !== undefined) {
    summary.versions = Array.from(snapshot.versionByService.entries()).map(
      ([serviceId, versions]) => ({
        serviceId,
        versions: Array.from(versions.values()).map((version) => ({
          version: version.version,
          buildId: version.buildId,
          ...(version.pointerBuildId !== undefined
            ? { pointerBuildId: version.pointerBuildId }
            : {})
        }))
      })
    );
  }
  return summary;
}

function summarizeFileBackend(
  fileBackend: NonNullable<RuntimeControlMetadata['fileBackend']>
): Record<string, unknown> {
  return {
    localConfigured: fileBackend.local !== undefined,
    ossConfigured: fileBackend.oss !== undefined,
    effectiveBackend: fileBackend.local !== undefined ? 'local' : 'oss'
  };
}

function summarizeTelemetry(
  telemetry: NonNullable<RuntimeControlMetadata['telemetry']>
): Record<string, unknown> {
  return {
    enabled: telemetry.enabled,
    endpointConfigured: true,
    protocol: telemetry.protocol,
    topics: telemetry.topics,
    queueMaxEvents: telemetry.queueMaxEvents,
    batchMaxEvents: telemetry.batchMaxEvents,
    batchMaxBytes: telemetry.batchMaxBytes,
    flushIntervalMs: telemetry.flushIntervalMs
  };
}
