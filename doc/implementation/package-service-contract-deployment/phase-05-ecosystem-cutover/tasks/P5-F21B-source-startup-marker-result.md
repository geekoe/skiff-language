# P5-F21B：Source-suite Startup Marker Result

`F21B PASS`

开发提交`367689528ab742d9fe4ccc85159885200eebdf84`，parent
`7bb6c2af9517f2091654fd1f127e87ca6ef02f68`，tree `f305b7d7edf5f9d9bb0d4775fbcce3c5c873bf0d`。合流提交为
`dbfb98ac0a10d3959d803a8a92de1c04bba66fce`，parent
`9863575ed6abfa1bafdae256d276303f2994317e`；F21A/B最终tree为
`68a824aa233ade4cd455c7be999f5fa1219b46cc`，lock保持
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

canonical source-suite在调用isolated runtime owner之前精确输出
`[skiff-tests] phase startup: isolated-runtime`。聚焦负例证明runtime owner在pre-readiness直接抛错时，startup marker已先
写出且没有source command执行；原有std、Host prepare与Host runner marker顺序不变。F21A使用该marker将pre-readiness
失败归入startup/isolated-runtime，而不依赖ready后才出现的workspace/control输出。

同一最终代码状态上的唯一batch combined为：

```bash
node --unhandled-rejections=strict --test \
  scripts/tests/platform-source-probe-diagnostic.test.mjs \
  scripts/tests/platform-source-shared-target-probe.test.mjs \
  scripts/tests/skiff-source-test-suite.test.mjs
```

结果44 pass/0 fail，`git diff --check` PASS。未运行I16动态probe、Host/full/stable；F21C仍是下一独立pending节点，
须先建立任务合同。
