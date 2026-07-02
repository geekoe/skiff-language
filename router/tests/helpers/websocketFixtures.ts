export function connectionMessageSchema(nullable = false) {
  return {
    ...(nullable ? { nullable: true } : {}),
    oneOf: [
      {
        type: 'object',
        required: ['tag', 'text'],
        properties: {
          tag: { type: 'string', enum: ['text'] },
          text: { type: 'string' }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: ['tag', 'base64'],
        properties: {
          tag: { type: 'string', enum: ['binary'] },
          base64: { type: 'string' }
        },
        additionalProperties: false
      }
    ]
  };
}

export function gatewayConnectResultSchema() {
  return {
    oneOf: [
      {
        type: 'object',
        required: ['tag', 'context'],
        properties: {
          tag: { type: 'string', enum: ['accept'] },
          context: { type: 'any' },
          businessIdentity: { type: 'string', nullable: true },
          connectionPolicy: websocketConnectionPolicySchema()
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: ['tag', 'code', 'reason'],
        properties: {
          tag: { type: 'string', enum: ['reject'] },
          code: { type: 'integer' },
          reason: { type: 'string' }
        },
        additionalProperties: false
      }
    ]
  };
}

export function websocketConnectionPolicySchema() {
  return {
    type: 'object',
    required: ['maxConnections', 'overflow'],
    properties: {
      maxConnections: { type: 'integer' },
      overflow: { type: 'string' },
      closeCode: { type: 'integer' },
      closeReason: { type: 'string' }
    },
    additionalProperties: false
  };
}

export function baseWebSocketManifest(): any {
  return websocketManifestValue({
    serviceId: 'skiff.run/hello',
    revisionId: '1111111111111111111111111111111111111111111111111111111111111111',
    protocolIdentity:
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000002',
    connectOperation: 'HelloSocket.connect',
    connectTarget: 'service.skiff~run~~hello.HelloSocket.connect',
    connectParameters: [
      {
        name: 'session',
        schema: { type: 'any' }
      }
    ],
    receiveOperation: 'HelloConnection.receive',
    receiveTarget: 'service.skiff~run~~hello.HelloConnection.receive',
    receiveParameters: [
      {
        name: 'context',
        schema: { type: 'any' }
      },
      {
        name: 'message',
        schema: connectionMessageSchema()
      },
      {
        name: 'connectionId',
        schema: { type: 'string' }
      }
    ],
    context: { type: 'any' },
    connectAdapterArgs: [
      { param: 'session', source: { kind: 'websocket.connectRequest' } }
    ],
    receiveAdapterArgs: [
      { param: 'context', source: { kind: 'websocket.connectionContext' } },
      { param: 'message', source: { kind: 'websocket.message' } },
      { param: 'connectionId', source: { kind: 'websocket.connectionId' } }
    ]
  });
}

export function webSocketManifestValue(): any {
  return websocketManifestValue({
    serviceId: 'example.com/websocket_fixture',
    revisionId: '2222222222222222222222222222222222222222222222222222222222222222',
    protocolIdentity:
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000003',
    connectOperation: 'WebSocketFixtureConnection.connect',
    connectTarget: 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect',
    connectParameters: [
      {
        name: 'input',
        schema: {
          type: 'object',
          required: ['deviceId', 'platform', 'clientVersion', 'language'],
          properties: {
            service: { type: 'string' },
            deviceId: { type: 'string' },
            platform: { type: 'string' },
            clientVersion: { type: 'string' },
            language: { type: 'string' },
            userId: { type: 'string', nullable: true }
          },
          additionalProperties: true
        }
      }
    ],
    receiveOperation: 'WebSocketFixtureConnection.receive',
    receiveTarget: 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.receive',
    receiveParameters: [
      {
        name: 'context',
        schema: { type: 'any' }
      },
      {
        name: 'message',
        schema: connectionMessageSchema()
      },
      {
        name: 'userId',
        schema: { type: 'string' }
      },
      {
        name: 'connectionId',
        schema: { type: 'string' }
      }
    ],
    context: {
      type: 'object',
      required: ['userId', 'deviceId', 'platform', 'clientVersion', 'language'],
      properties: {
        userId: { type: 'string' },
        deviceId: { type: 'string' },
        platform: { type: 'string' },
        clientVersion: { type: 'string' },
        language: { type: 'string' }
      },
      additionalProperties: false
    },
    connectAdapterArgs: [
      { param: 'input', source: { kind: 'websocket.connectRequest' } }
    ],
    receiveAdapterArgs: [
      { param: 'context', source: { kind: 'websocket.connectionContext' } },
      { param: 'message', source: { kind: 'websocket.message' } },
      { param: 'userId', source: { kind: 'websocket.businessIdentity' } },
      { param: 'connectionId', source: { kind: 'websocket.connectionId' } }
    ],
    timeoutMs: 2000
  });
}

function websocketManifestValue(options: {
  serviceId: string;
  revisionId: string;
  protocolIdentity: string;
  connectOperation: string;
  connectTarget: string;
  connectParameters: Array<{ name: string; schema: object }>;
  receiveOperation: string;
  receiveTarget: string;
  receiveParameters: Array<{ name: string; schema: object }>;
  context: object;
  connectAdapterArgs: Array<{ param: string; source: { kind: string } }>;
  receiveAdapterArgs: Array<{ param: string; source: { kind: string } }>;
  timeoutMs?: number;
}): any {
  const manifest = {
    schemaVersion: 'skiff-runtime-manifest-v1',
    service: {
      id: options.serviceId,
      revisionId: options.revisionId,
      protocolIdentity: options.protocolIdentity
    },
    operations: [
      {
        operation: options.connectOperation,
        operationAbiId: testOperationAbiId(options.connectTarget),
        target: options.connectTarget,
        mode: 'unary',
        parameters: options.connectParameters,
        response: gatewayConnectResultSchema()
      },
      {
        operation: options.receiveOperation,
        operationAbiId: testOperationAbiId(options.receiveTarget),
        target: options.receiveTarget,
        mode: 'unary',
        parameters: options.receiveParameters,
        response: { type: 'null' }
      }
    ],
    gateway: {
      websocket: {
        id: 'client',
        path: '/ws',
        serviceParam: 'service',
        context: options.context,
        contextExpectation: {
          kind: 'typed',
          connectOperationAbiId: testOperationAbiId(options.connectTarget),
          contextTypeIdentity: `type:test:${options.connectTarget}:context`
        },
        connect: {
          operation: options.connectOperation,
          operationAbiId: testOperationAbiId(options.connectTarget),
          adapterArgs: options.connectAdapterArgs
        },
        receive: {
          operation: options.receiveOperation,
          operationAbiId: testOperationAbiId(options.receiveTarget),
          adapterArgs: options.receiveAdapterArgs
        }
      }
    },
    ...(options.timeoutMs !== undefined ? { timeout: { defaultMs: options.timeoutMs } } : {})
  };

  return manifest;
}

function testOperationAbiId(target: string): string {
  return `operation:test:${target}`;
}
