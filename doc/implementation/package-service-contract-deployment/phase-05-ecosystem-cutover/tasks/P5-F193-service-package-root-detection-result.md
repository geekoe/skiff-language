# P5-F193：Service Package 根目录识别结果

状态：Completed

## 直接父任务

- `P5-F193-service-package-root-detection.md`

## 结果

CLI 根目录识别已与统一 Package authoring 模型对齐：

- `package.yml` 是 source root 身份的唯一 manifest；
- 同目录存在 `service.yml` 时，仍识别为 Package；`service.yml` 只附加 service role；
- 只有 `service.yml`、没有 `package.yml` 的 legacy service-only 目录明确失败，并说明迁移方式；
- 没有 `package.yml` 的目录明确失败；
- 单文件测试入口保持不变，由 test-runner 向上寻找所属 `package.yml`；
- 不要求 Registry 删除 `package.yml` 或 `service.yml`，也没有恢复独立 service source root。

公开 Publication 文档同步明确了这条根身份规则。compiler authoring 和 test-runner 原本已经以
`package.yml` 为唯一 Package owner；本任务删除的是 `skiff test` 在进入真实 authoring 前错误添加的
“双 manifest 冲突”预判。

## 验证

- `node --test scripts/tests/skiff-test-cli.test.mjs`：7/7 PASS。
  - package-only 目录进入 canonical test-runner；
  - `package.yml + service.yml` 目录进入同一个 canonical test-runner；
  - service-only 和无 manifest 目录都在启动 Cargo 前失败；
  - 文件入口、live 参数、废弃参数及重复参数矩阵继续通过。
- `git diff --check`：PASS。
- 真实命令：

  ```bash
  node scripts/skiff.mjs test \
    /Users/geek/workspace/skiff-packages-phase-05-integration/registry \
    --artifact-root <temporary-artifact-root> \
    --require-tests
  ```

  已越过原先的 `package.yml + service.yml` ambiguous 门禁，启动隔离 MongoDB、Router、Runtime，
  并进入真实 compiler authoring。随后被直接父链已单列的 F192 test-link blocker 阻断：
  Registry 引用 `skiff.run/std`，而 source artifact root 尚未预置 canonical std
  `PackageArtifact`。这不是根识别失败；F192 合入后应在 integration HEAD 重跑同一命令完成终态验收。

测试只使用临时 artifact root 和隔离 instance；没有连接或修改 stable instance，没有 push。
