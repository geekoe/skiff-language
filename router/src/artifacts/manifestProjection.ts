import type {
  HttpRouteHandlerManifest,
  JsonSchema,
  ServiceAccessManifest,
  SkiffRuntimeManifest,
  WebSocketEntryManifest,
} from "../manifest/types.js";
import {
  assertRecord,
  isRecord,
  readOptionalRecord,
  readRequiredArray,
  readRequiredJsonSchema,
  readRequiredString,
} from "./readUtils.js";

interface AssemblyOperationProjection {
  operation: string;
  operationAbiId: string;
  entrypoint: string;
  mode: "unary" | "serverStream";
  parameters: SkiffRuntimeManifest["operations"][number]["parameters"];
  response: JsonSchema;
  serviceProtocolIdentity: string;
}

interface ServiceUnitOperationRoute {
  operation: string;
  operationAbiId: string;
  parameterNames: string[];
}

interface ServiceUnitOperationSignature {
  operation: string;
  operationAbiId: string;
  parameterNames: string[];
}

export function operationsFromServiceUnitRoutes(
  assembly: Record<string, unknown>,
  serviceUnit: Record<string, unknown>,
  indexPath: string,
  serviceUnitPath: string,
  protocolIdentity: string,
): SkiffRuntimeManifest["operations"] {
  const assemblyOperations = assemblyOperationsFromServiceAssembly(
    assembly.operations,
    indexPath,
    protocolIdentity,
  );
  const serviceRoutes = serviceUnitOperationRoutes(
    serviceUnit,
    `${serviceUnitPath} service unit`,
  );
  const assemblyByName = new Map<string, AssemblyOperationProjection>();
  for (const operation of assemblyOperations) {
    if (assemblyByName.has(operation.operation)) {
      throw new Error(
        `duplicate serviceAssembly operation: ${operation.operation}`,
      );
    }
    assemblyByName.set(operation.operation, operation);
  }

  const operations: SkiffRuntimeManifest["operations"] = [];
  for (const route of serviceRoutes) {
    const assemblyOperation = assemblyByName.get(route.operation);
    if (!assemblyOperation) {
      throw new Error(
        `${indexPath} serviceAssembly.operations is missing typed serviceUnit operation ${route.operation}`,
      );
    }
    assemblyByName.delete(route.operation);
    validateServiceRouteProjection(assemblyOperation, route, indexPath);
    operations.push({
      operation: route.operation,
      operationAbiId: route.operationAbiId,
      target: assemblyOperation.entrypoint,
      mode: assemblyOperation.mode,
      parameters: assemblyOperation.parameters,
      response: assemblyOperation.response,
      serviceProtocolIdentity: assemblyOperation.serviceProtocolIdentity,
    });
  }

  const [extraOperation] = assemblyByName.keys();
  if (extraOperation !== undefined) {
    throw new Error(
      `${indexPath} serviceAssembly.operations declares ${extraOperation} but typed serviceUnit routes do not`,
    );
  }
  return operations;
}

function assemblyOperationsFromServiceAssembly(
  value: unknown,
  indexPath: string,
  protocolIdentity: string,
): AssemblyOperationProjection[] {
  return readRequiredArray(
    value,
    `${indexPath} serviceAssembly.operations`,
  ).map((operationValue) => {
    assertRecord(operationValue, `${indexPath} serviceAssembly.operations[]`);
    const operation = readRequiredString(
      operationValue.operation,
      `${indexPath} serviceAssembly.operation.operation`,
    );
    const operationAbiId = readRequiredString(
      operationValue.operationAbiId,
      `${indexPath} serviceAssembly.operation.operationAbiId`,
    );
    rejectLegacyRouteFields(
      operationValue,
      `${indexPath} serviceAssembly.operation`,
    );
    const entrypoint = readRequiredString(
      operationValue.entrypoint,
      `${indexPath} serviceAssembly.operation.entrypoint`,
    );
    const mode = readDispatchMode(
      operationValue.mode,
      `${indexPath} serviceAssembly.operation.mode`,
    );
    const parameters = readRequiredArray(
      operationValue.parameters,
      `${indexPath} serviceAssembly.operation.parameters`,
    ).map((parameterValue) => {
      assertRecord(
        parameterValue,
        `${indexPath} serviceAssembly.operation.parameter`,
      );
      return {
        name: readRequiredString(
          parameterValue.name,
          `${indexPath} serviceAssembly.operation.parameter.name`,
        ),
        schema: readRequiredJsonSchema(
          parameterValue.schema,
          `${indexPath} serviceAssembly.operation.parameter.schema`,
        ),
      };
    });
    return {
      operation,
      operationAbiId,
      entrypoint,
      mode,
      parameters,
      response: readRequiredJsonSchema(
        operationValue.response,
        `${indexPath} serviceAssembly.operation.response`,
      ),
      serviceProtocolIdentity: protocolIdentity,
    };
  });
}

function serviceUnitOperationRoutes(
  serviceUnit: Record<string, unknown>,
  label: string,
): ServiceUnitOperationRoute[] {
  const operations =
    serviceUnit.operations === undefined
      ? []
      : readRequiredArray(serviceUnit.operations, `${label}.operations`);
  if (operations.length === 0) {
    return [];
  }
  const signatures = serviceUnitOperationSignatures(
    serviceUnit.publicationAbi,
    `${label}.publicationAbi`,
  );
  return operations.map((operationValue, index) => {
    const operationLabel = `${label}.operations[${index}]`;
    assertRecord(operationValue, operationLabel);
    rejectLegacyRouteFields(operationValue, operationLabel);
    const operationRef = serviceUnitOperationRef(operationValue, operationLabel);
    const signature = signatures.get(operationRef.operationAbiId);
    if (!signature) {
      throw new Error(
        `${operationLabel}.operation must have a matching ${label}.publicationAbi.operationAbi entry`,
      );
    }
    if (signature.operation !== operationRef.operation) {
      throw new Error(
        `${operationLabel}.operation.publicPath must match ${label}.publicationAbi.operationAbi publicPath`,
      );
    }
    return signature;
  });
}

function serviceUnitOperationSignatures(
  value: unknown,
  label: string,
): Map<string, ServiceUnitOperationSignature> {
  const publicationAbi = readOptionalRecord(value);
  if (!publicationAbi) {
    throw new Error(`${label} must be an object`);
  }
  const signatures = new Map<string, ServiceUnitOperationSignature>();
  const operationAbi =
    publicationAbi.operationAbi === undefined
      ? []
      : readRequiredArray(publicationAbi.operationAbi, `${label}.operationAbi`);
  for (const [index, abiValue] of operationAbi.entries()) {
    const abiLabel = `${label}.operationAbi[${index}]`;
    assertRecord(abiValue, abiLabel);
    const operationRef = operationAbiRef(abiValue.operation, `${abiLabel}.operation`);
    const publicSignature = readOptionalRecord(abiValue.publicSignature);
    if (!publicSignature) {
      throw new Error(`${abiLabel}.publicSignature must be an object`);
    }
    readRequiredBoolean(
      publicSignature.maySuspend,
      `${abiLabel}.publicSignature.maySuspend`,
    );
    if (signatures.has(operationRef.operationAbiId)) {
      throw new Error(
        `${label}.operationAbi contains duplicate operationAbiId ${operationRef.operationAbiId}`,
      );
    }
    signatures.set(operationRef.operationAbiId, {
      operation: operationRef.operation,
      operationAbiId: operationRef.operationAbiId,
      parameterNames: serviceUnitParameterNames(
        publicSignature.params,
        `${abiLabel}.publicSignature.params`,
      ),
    });
  }
  return signatures;
}

function serviceUnitOperationRef(
  operationValue: Record<string, unknown>,
  label: string,
): { operation: string; operationAbiId: string } {
  const kind = readRequiredString(operationValue.kind, `${label}.kind`);
  if (kind === "localExecutable") {
    rejectUnsupportedServiceOperationKeys(
      operationValue,
      label,
      new Set(["kind", "operation", "executable"]),
    );
    return operationAbiRef(operationValue.operation, `${label}.operation`);
  }
  if (kind === "localReceiverExecutable") {
    rejectUnsupportedServiceOperationKeys(
      operationValue,
      label,
      new Set(["kind", "operation", "receiverExecutable"]),
    );
    return operationAbiRef(operationValue.operation, `${label}.operation`);
  }
  throw new Error(`${label}.kind must be localExecutable or localReceiverExecutable`);
}

function operationAbiRef(
  value: unknown,
  label: string,
): { operation: string; operationAbiId: string } {
  assertRecord(value, label);
  return {
    operation: readRequiredString(value.publicPath, `${label}.publicPath`),
    operationAbiId: readRequiredString(
      value.operationAbiId,
      `${label}.operationAbiId`,
    ),
  };
}

function rejectUnsupportedServiceOperationKeys(
  value: Record<string, unknown>,
  label: string,
  supported: ReadonlySet<string>,
): void {
  const keys = Object.keys(value).filter((key) => !supported.has(key));
  if (keys.length > 0) {
    throw new Error(
      `${label} does not support ${keys.map((key) => JSON.stringify(key)).join(", ")}`,
    );
  }
}

function readRequiredBoolean(value: unknown, label: string): boolean {
  if (typeof value !== "boolean") {
    throw new Error(`${label} must be a boolean`);
  }
  return value;
}

function validateServiceRouteProjection(
  assemblyOperation: AssemblyOperationProjection,
  route: ServiceUnitOperationRoute,
  indexPath: string,
): void {
  if (assemblyOperation.operationAbiId !== route.operationAbiId) {
    throw new Error(
      `${indexPath} serviceAssembly.operation.operationAbiId for ${route.operation} must match typed serviceUnit operationAbiId ${route.operationAbiId}`,
    );
  }
  const assemblyParameterNames = assemblyOperation.parameters.map(
    (parameter) => parameter.name,
  );
  if (!sameStrings(assemblyParameterNames, route.parameterNames)) {
    throw new Error(
      `${indexPath} serviceAssembly.operation.parameters for ${route.operation} must match typed serviceUnit route parameters`,
    );
  }
}

export function gatewayFromServiceAssembly(
  assembly: Record<string, unknown>,
  indexPath: string,
  operations: SkiffRuntimeManifest["operations"] = [],
): NonNullable<SkiffRuntimeManifest["gateway"]> {
  const gateway: NonNullable<SkiffRuntimeManifest["gateway"]> = {};
  const operationByName = new Map(
    operations.map((operation) => [operation.operation, operation]),
  );
  const gatewayRecord = readOptionalRecord(assembly.gateway);
  if (!gatewayRecord) {
    return gateway;
  }

  const http = readOptionalRecord(gatewayRecord.http);
  if (http) {
    let raw:
      | {
          operation: string;
          target: string;
        }
      | undefined;
    let routes: NonNullable<
      NonNullable<SkiffRuntimeManifest["gateway"]>["http"]
    >["routes"];
    if ("raw" in http) {
      assertRecord(http.raw, `${indexPath} serviceAssembly.gateway.http.raw`);
      const operation = readRequiredString(
        http.raw.operation,
        `${indexPath} serviceAssembly.gateway.http.raw.operation`,
      );
      const target = readRequiredString(
        http.raw.target,
        `${indexPath} serviceAssembly.gateway.http.raw.target`,
      );
      raw = { operation, target };
    }
    if ("routes" in http) {
      routes = httpRoutesFromServiceAssembly(
        http.routes,
        `${indexPath} serviceAssembly.gateway.http.routes`,
        operationByName,
      );
    }
    const keys = Object.keys(http).filter(
      (key) => key !== "raw" && key !== "routes",
    );
    if (keys.length > 0) {
      throw new Error(
        `${indexPath} serviceAssembly.gateway.http does not support ${keys
          .map((key) => JSON.stringify(key))
          .join(", ")}`,
      );
    }
    gateway.http = {
      ...(raw ? { raw } : {}),
      ...(routes !== undefined ? { routes } : {}),
    };
  }
  const websocket = readOptionalRecord(gatewayRecord.websocket);
  if (websocket) {
    gateway.websocket = websocketFromServiceAssembly(
      websocket,
      `${indexPath} serviceAssembly.gateway.websocket`,
      operationByName,
    );
  }
  return gateway;
}

function httpRoutesFromServiceAssembly(
  value: unknown,
  label: string,
  operationByName: ReadonlyMap<
    string,
    SkiffRuntimeManifest["operations"][number]
  >,
): NonNullable<NonNullable<SkiffRuntimeManifest["gateway"]>["http"]>["routes"] {
  return readRequiredArray(value, label).map((routeValue, index) => {
    const routeLabel = `${label}[${index}]`;
    assertRecord(routeValue, routeLabel);
    const supported = new Set([
      "id",
      "path",
      "method",
      "handler",
      "operation",
      "operationAbiId",
      "target",
      "serviceOperationTarget",
      "serviceProtocolIdentity",
      "gatewayEntryIdentity",
      "adapter",
      "typed",
    ]);
    const keys = Object.keys(routeValue).filter((key) => !supported.has(key));
    if (keys.length > 0) {
      throw new Error(
        `${routeLabel} does not support ${keys.map((key) => JSON.stringify(key)).join(", ")}`,
      );
    }
    const handler =
      routeValue.handler !== undefined
        ? httpRouteHandlerFromServiceAssembly(
            routeValue.handler,
            `${routeLabel}.handler`,
          )
        : undefined;
    const projectedOperation =
      routeValue.operation !== undefined
        ? readRequiredString(routeValue.operation, `${routeLabel}.operation`)
        : undefined;
    let operationAbiId =
      routeValue.operationAbiId !== undefined
        ? readRequiredString(
            routeValue.operationAbiId,
            `${routeLabel}.operationAbiId`,
          )
        : undefined;
    const typedOperation =
      projectedOperation !== undefined
        ? operationByName.get(projectedOperation)
        : undefined;
    const operation =
      projectedOperation !== undefined &&
      (typedOperation !== undefined || handler?.kind !== "packageFunction")
        ? projectedOperation
        : undefined;
    if (typedOperation !== undefined) {
      if (
        operationAbiId !== undefined &&
        operationAbiId !== typedOperation.operationAbiId
      ) {
        throw new Error(
          `${routeLabel}.operationAbiId must match operation ${operation}.operationAbiId`,
        );
      }
      operationAbiId = typedOperation.operationAbiId;
    }
    return {
      ...(routeValue.id !== undefined
        ? { id: readRequiredString(routeValue.id, `${routeLabel}.id`) }
        : {}),
      path: readRequiredString(routeValue.path, `${routeLabel}.path`),
      ...(routeValue.method !== undefined
        ? {
            method: readRequiredString(
              routeValue.method,
              `${routeLabel}.method`,
            ),
          }
        : {}),
      ...(handler !== undefined ? { handler } : {}),
      ...(operation !== undefined ? { operation } : {}),
      ...(operationAbiId !== undefined ? { operationAbiId } : {}),
      ...(routeValue.target !== undefined
        ? {
            target: readRequiredString(
              routeValue.target,
              `${routeLabel}.target`,
            ),
          }
        : {}),
      ...(routeValue.serviceOperationTarget !== undefined
        ? {
            serviceOperationTarget: readRequiredString(
              routeValue.serviceOperationTarget,
              `${routeLabel}.serviceOperationTarget`,
            ),
          }
        : {}),
      ...(routeValue.serviceProtocolIdentity !== undefined
        ? {
            serviceProtocolIdentity: readRequiredString(
              routeValue.serviceProtocolIdentity,
              `${routeLabel}.serviceProtocolIdentity`,
            ),
          }
        : {}),
      ...(routeValue.gatewayEntryIdentity !== undefined
        ? {
            gatewayEntryIdentity: readRequiredString(
              routeValue.gatewayEntryIdentity,
              `${routeLabel}.gatewayEntryIdentity`,
            ),
          }
        : {}),
      ...(routeValue.adapter !== undefined
        ? { adapter: routeValue.adapter as never }
        : {}),
      ...(routeValue.typed !== undefined
        ? { typed: routeValue.typed as never }
        : {}),
    };
  });
}

function httpRouteHandlerFromServiceAssembly(
  value: unknown,
  label: string,
): HttpRouteHandlerManifest {
  assertRecord(value, label);
  const kind = readRequiredString(value.kind, `${label}.kind`);
  if (kind === "serviceFunction") {
    rejectUnsupportedRouteHandlerKeys(
      value,
      label,
      new Set(["kind", "source", "modulePath", "symbol"]),
    );
    return {
      kind,
      ...(value.source !== undefined
        ? { source: readRequiredString(value.source, `${label}.source`) }
        : {}),
      ...(value.modulePath !== undefined
        ? {
            modulePath: readRequiredString(
              value.modulePath,
              `${label}.modulePath`,
            ),
          }
        : {}),
      ...(value.symbol !== undefined
        ? { symbol: readRequiredString(value.symbol, `${label}.symbol`) }
        : {}),
    };
  }
  if (kind === "packageFunction") {
    rejectUnsupportedRouteHandlerKeys(
      value,
      label,
      new Set(["kind", "source", "packageId", "alias", "symbolPath"]),
    );
    return {
      kind,
      ...(value.source !== undefined
        ? { source: readRequiredString(value.source, `${label}.source`) }
        : {}),
      packageId: readRequiredString(value.packageId, `${label}.packageId`),
      ...(value.alias !== undefined
        ? { alias: readRequiredString(value.alias, `${label}.alias`) }
        : {}),
      symbolPath: readRequiredString(value.symbolPath, `${label}.symbolPath`),
    };
  }
  throw new Error(`${label}.kind must be serviceFunction or packageFunction`);
}

function rejectUnsupportedRouteHandlerKeys(
  handler: Record<string, unknown>,
  label: string,
  supported: ReadonlySet<string>,
): void {
  const keys = Object.keys(handler).filter((key) => !supported.has(key));
  if (keys.length > 0) {
    throw new Error(
      `${label} does not support ${keys.map((key) => JSON.stringify(key)).join(", ")}`,
    );
  }
}

export function timeoutFromServiceAssembly(
  assembly: Record<string, unknown>,
): SkiffRuntimeManifest["timeout"] | undefined {
  return isRecord(assembly.timeout)
    ? (assembly.timeout as NonNullable<SkiffRuntimeManifest["timeout"]>)
    : undefined;
}

export function accessFromServiceAssembly(
  service: Record<string, unknown>,
): SkiffRuntimeManifest["service"]["access"] | undefined {
  if (service.access === undefined || service.access === null) {
    return undefined;
  }
  assertRecord(service.access, "serviceAssembly.service.access");
  const keys = Object.keys(service.access).filter(
    (key) => key !== "visibility" && key !== "organizationRole",
  );
  if (keys.length > 0) {
    throw new Error(
      `serviceAssembly.service.access does not support ${keys
        .map((key) => JSON.stringify(key))
        .join(", ")}`,
    );
  }
  const visibility = readServiceAccessVisibility(
    service.access.visibility,
    "serviceAssembly.service.access.visibility",
  );
  const organizationRole =
    service.access.organizationRole === undefined ||
    service.access.organizationRole === null
      ? undefined
      : readServiceAccessOrganizationRole(
          service.access.organizationRole,
          "serviceAssembly.service.access.organizationRole",
        );
  if (visibility === "public" && organizationRole !== undefined) {
    throw new Error(
      "serviceAssembly.service.access.organizationRole is only allowed when visibility is internal",
    );
  }
  return {
    visibility,
    ...(visibility === "internal"
      ? { organizationRole: organizationRole ?? "viewer" }
      : {}),
  };
}

function readServiceAccessVisibility(
  value: unknown,
  label: string,
): ServiceAccessManifest["visibility"] {
  if (value === "public" || value === "internal") {
    return value;
  }
  throw new Error(`${label} must be public or internal`);
}

function readServiceAccessOrganizationRole(
  value: unknown,
  label: string,
): NonNullable<ServiceAccessManifest["organizationRole"]> {
  if (value === "viewer" || value === "maintainer" || value === "owner") {
    return value;
  }
  throw new Error(`${label} must be viewer, maintainer, or owner`);
}

function readDispatchMode(
  value: unknown,
  label: string,
): "unary" | "serverStream" {
  if (value === "unary" || value === "serverStream") {
    return value;
  }
  throw new Error(`${label} must be unary or serverStream`);
}

function rejectLegacyRouteFields(
  value: Record<string, unknown>,
  label: string,
): void {
  if ("routeTarget" in value || "route_target" in value) {
    throw new Error(
      `${label}.routeTarget is no longer supported; use entrypoint`,
    );
  }
  if ("target" in value) {
    throw new Error(
      `${label}.target is no longer a dispatch field; use entrypoint`,
    );
  }
}

function serviceUnitParameterNames(value: unknown, label: string): string[] {
  if (value === undefined || value === null) {
    return [];
  }
  return readRequiredArray(value, label).map((parameterValue, index) => {
    assertRecord(parameterValue, `${label}[${index}]`);
    return readRequiredString(parameterValue.name, `${label}[${index}].name`);
  });
}

function sameStrings(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function websocketFromServiceAssembly(
  websocket: Record<string, unknown>,
  label: string,
  operationByName: ReadonlyMap<
    string,
    SkiffRuntimeManifest["operations"][number]
  >,
): WebSocketEntryManifest {
  const projected = { ...websocket };
  if (projected.context !== undefined) {
    projected.context = websocketContextSchema(
      projected.context,
      `${label}.context`,
    );
  }
  projected.connect = websocketOperationFromServiceAssembly(
    projected.connect,
    `${label}.connect`,
    operationByName,
  );
  projected.receive = websocketOperationFromServiceAssembly(
    projected.receive,
    `${label}.receive`,
    operationByName,
  );
  if (Array.isArray(projected.routes)) {
    projected.routes = projected.routes.map((route, index) =>
      websocketOperationFromServiceAssembly(
        route,
        `${label}.routes[${index}]`,
        operationByName,
      ),
    );
  }
  return projected as unknown as WebSocketEntryManifest;
}

function websocketOperationFromServiceAssembly(
  value: unknown,
  label: string,
  operationByName: ReadonlyMap<
    string,
    SkiffRuntimeManifest["operations"][number]
  >,
): unknown {
  if (value === undefined || value === null) {
    return value;
  }
  const operation = readOptionalRecord(value);
  if (!operation) {
    return value;
  }
  const operationName = readRequiredString(
    operation.operation,
    `${label}.operation`,
  );
  const typedOperation = operationByName.get(operationName);
  if (!typedOperation) {
    throw new Error(
      `${label} references unknown typed serviceUnit operation ${operationName}`,
    );
  }
  return {
    ...operation,
    operationAbiId: typedOperation.operationAbiId,
    serviceOperationTarget: typedOperation.target,
    serviceProtocolIdentity: typedOperation.serviceProtocolIdentity,
  };
}

function websocketContextSchema(value: unknown, label: string): JsonSchema {
  const record = readOptionalRecord(value);
  if (record && "schema" in record) {
    return readRequiredJsonSchema(record.schema, `${label}.schema`);
  }
  return readRequiredJsonSchema(value, label);
}
