import { describe, expect, it } from 'vitest';

import {
  buildResolvedConfig,
  defaultConfigSourceSpecs,
  parseConfigYamlSource,
  validateConfigShapeEntries,
  type ConfigShapeEntry,
  type ConfigSource
} from '../src/config/index.js';

const extLlmShape = [
  {
    path: 'dashscopeApiKey',
    type: 'string',
    required: true
  },
  {
    path: 'dashscopeModel',
    type: 'string',
    required: false
  },
  {
    path: 'dashscopeExtra',
    type: 'Json',
    required: false
  }
] satisfies ConfigShapeEntry[];

describe('config YAML source parsing', () => {
  it('parses the JSON-compatible YAML subset into a normalized object', () => {
    expect(
      parseConfigYamlSource(
        [
          'dashscopeModel: qwen-plus',
          'dashscopeRetries: 2',
          'dashscopeEnabled: true',
          'dashscopeExtra: [fast, safe]'
        ].join('\n'),
        { label: 'config.yml', sourceClass: 'bundle' }
      )
    ).toEqual({
      sourceClass: 'bundle',
      label: 'config.yml',
      value: {
        dashscopeModel: 'qwen-plus',
        dashscopeRetries: 2,
        dashscopeEnabled: true,
        dashscopeExtra: ['fast', 'safe']
      }
    });
  });

  it('rejects duplicate keys before parser overwrite semantics can apply', () => {
    expect(() =>
      parseConfigYamlSource('dashscope:\n  model: one\n  model: two\n', {
        label: 'config.yml',
        sourceClass: 'bundle'
      })
    ).toThrow(/config\.yml.*duplicate key/i);
  });

  it('rejects dotted keys, invalid path segments, anchors, aliases, and tags', () => {
    expect(() =>
      parseConfigYamlSource('"openai.model": gpt-5\n', {
        label: 'config.yml',
        sourceClass: 'bundle'
      })
    ).toThrow(/config\.yml.*dotted YAML keys are not supported/);

    expect(() =>
      parseConfigYamlSource('"openai.model": gpt-5\nopenai:\n  model: gpt-4\n', {
        label: 'config.yml',
        sourceClass: 'bundle'
      })
    ).toThrow(/config\.yml.*dotted YAML keys are not supported/);

    expect(() =>
      parseConfigYamlSource('dashscope:\n  1bad: true\n', {
        label: 'config.yml',
        sourceClass: 'bundle'
      })
    ).toThrow(/config\.yml.*invalid config key dashscope\.1bad/);

    expect(() =>
      parseConfigYamlSource('dashscope: &dashscope\n  model: qwen\ncopy: *dashscope\n', {
        label: 'config.yml',
        sourceClass: 'bundle'
      })
    ).toThrow(/config\.yml.*anchor|config\.yml.*alias/);

    expect(() =>
      parseConfigYamlSource('dashscopeModel: !custom qwen\n', {
        label: 'config.yml',
        sourceClass: 'bundle'
      })
    ).toThrow(/config\.yml.*tag/);
  });
});

describe('config source set defaults', () => {
  it('uses base, profile, and profile secret overlay order', () => {
    expect(defaultConfigSourceSpecs('prod')).toEqual([
      { path: 'config.yml', label: 'config.yml', sourceClass: 'bundle' },
      { path: 'config.prod.yml', label: 'config.prod.yml', sourceClass: 'bundle' },
      {
        path: 'config.prod.secret.yml',
        label: 'config.prod.secret.yml',
        sourceClass: 'secret'
      }
    ]);
  });

  it('uses only base config without a profile', () => {
    expect(defaultConfigSourceSpecs()).toEqual([
      { path: 'config.yml', label: 'config.yml', sourceClass: 'bundle' }
    ]);
  });
});

describe('config shape validation and overlay merge', () => {
  it('merges overlays, tracks provenance, tombstones removed config, and redacts secret leaves', () => {
    const sources = [
      parseConfigYamlSource(
        [
          'dashscopeModel: qwen-turbo',
          'dashscopeExtra:',
          '  modes: [chat]'
        ].join('\n'),
        { label: 'config.yml', sourceClass: 'bundle' }
      ),
      parseConfigYamlSource(
        ['dashscopeModel: qwen-plus', 'dashscopeExtra: null'].join('\n'),
        { label: 'config.prod.yml', sourceClass: 'bundle' }
      ),
      parseConfigYamlSource(
        ['dashscopeApiKey: sk-secret'].join('\n'),
        { label: 'config.prod.secret.yml', sourceClass: 'secret' }
      )
    ] satisfies ConfigSource[];

    const resolved = buildResolvedConfig({
      configShape: validateConfigShapeEntries(extLlmShape, 'assembly.configShape.entries'),
      sources
    });

    expect(resolved.resolvedConfig).toEqual({
      dashscopeModel: 'qwen-plus',
      dashscopeApiKey: 'sk-secret'
    });
    expect(resolved.redactedResolvedConfig).toEqual({
      dashscopeModel: 'qwen-plus',
      dashscopeApiKey: '[REDACTED]'
    });
    expect(resolved.provenance.leaves['dashscopeModel']).toMatchObject({
      sourceClass: 'bundle',
      label: 'config.prod.yml'
    });
    expect(resolved.provenance.tombstones['dashscopeExtra']).toMatchObject({
      sourceClass: 'bundle',
      label: 'config.prod.yml'
    });
    expect(resolved.provenance.leaves['dashscopeExtra.modes']).toBeUndefined();
    expect(resolved.redactionProjectionIdentity).toMatch(
      /^skiff-config-redaction-v1:sha256:[0-9a-f]{64}$/
    );
  });

  it('keeps non-local leaves visible even when required by configShape', () => {
    const resolved = buildResolvedConfig({
      configShape: [
        { path: 'openaiApiKey', type: 'string', required: true },
        { path: 'model', type: 'string', required: true }
      ],
      sources: [
        parseConfigYamlSource('openaiApiKey: sk-bundle\nmodel: gpt-5\n', {
          label: 'config.yml',
          sourceClass: 'bundle'
        })
      ]
    });

    expect(resolved.redactedResolvedConfig).toEqual({
      openaiApiKey: 'sk-bundle',
      model: 'gpt-5'
    });
  });

  it('allows unknown leaves while enforcing declared final types', () => {
    expect(
      buildResolvedConfig({
        configShape: extLlmShape,
        sources: [
          parseConfigYamlSource('dashscopeUnknown: true\n', {
            label: 'config.yml',
            sourceClass: 'bundle'
          }),
          parseConfigYamlSource(
            'dashscopeApiKey: sk-local\ndashscopeLocalDynamic: secret\n',
            {
              label: 'config.prod.secret.yml',
              sourceClass: 'secret'
            }
          )
        ]
      }).redactedResolvedConfig
    ).toMatchObject({
      dashscopeUnknown: true,
      dashscopeLocalDynamic: '[REDACTED]'
    });

    expect(() =>
      buildResolvedConfig({
        configShape: extLlmShape,
        sources: [
          parseConfigYamlSource('dashscopeModel: 7\n', {
            label: 'config.yml',
            sourceClass: 'bundle'
          }),
          parseConfigYamlSource('dashscopeApiKey: sk-local\n', {
            label: 'config.prod.secret.yml',
            sourceClass: 'secret'
          })
        ]
      })
    ).toThrow(/final resolvedConfig.*dashscopeModel.*must be string/);
  });

  it('supports Json and JsonObject type spelling and rejects legacy policy fields', () => {
    expect(
      validateConfigShapeEntries(
        [
          { path: 'providerConfig', type: 'JsonObject', required: true },
          { path: 'providerAny', type: 'Json', required: false }
        ],
        'assembly.configShape.entries'
      )
    ).toEqual([
      { path: 'providerAny', type: 'Json', required: false },
      { path: 'providerConfig', type: 'JsonObject', required: true }
    ]);

    expect(() =>
      validateConfigShapeEntries(
        [
          {
            path: 'providerConfig',
            type: 'object',
            required: true
          }
        ],
        'assembly.configShape.entries'
      )
    ).toThrow(/type must be string, number, bool, Json, or JsonObject/);

    expect(() =>
      validateConfigShapeEntries(
        [
          {
            path: 'openaiApiKey',
            type: 'string',
            required: true,
            distribution: 'local'
          }
        ],
        'assembly.configShape.entries'
      )
    ).toThrow(/must not declare distribution/);

    expect(() =>
      validateConfigShapeEntries(
        [
          {
            path: 'openaiApiKey',
            type: 'string',
            required: true,
            redact: true
          }
        ],
        'assembly.configShape.entries'
      )
    ).toThrow(/must not declare redact/);
  });

  it('rejects missing required config and JsonObject mismatches', () => {
    expect(() =>
      buildResolvedConfig({
        configShape: [{ path: 'openaiApiKey', type: 'string', required: true }],
        sources: []
      })
    ).toThrow(/final resolvedConfig openaiApiKey is required/);

    expect(() =>
      buildResolvedConfig({
        configShape: [{ path: 'providerConfig', type: 'JsonObject', required: true }],
        sources: [
          parseConfigYamlSource('providerConfig: [one]\n', {
            label: 'config.yml',
            sourceClass: 'bundle'
          })
        ]
      })
    ).toThrow(/final resolvedConfig providerConfig must be JsonObject/);
  });
});
