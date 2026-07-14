# Skiff Language

Skiff 是面向后端服务的语言和 runtime stack。这个仓库包含语言实现、runtime、router、telemetry、CLI 脚本、标准库源码和 canonical 文档。

本语言尚未发布，不需要兼容历史格式。修改实现时优先让语义和文档收敛到当前正确模型，不要为旧 artifact、旧配置或旧 CLI 形态新增兼容层，除非已有测试明确要求当前行为。

## 仓库入口

- 文档入口：`doc/README.md` 和 `doc/overview.md`。
- 语言规范：`doc/reference/`。
- 长期架构契约：`doc/architecture/`。
- CLI 入口：`scripts/skiff.mjs`。
- Rust workspace：仓库根 `Cargo.toml`。
- TypeScript packages：`router/`、`telemetry/`、`scripts/`、`vscode/`。
- Skiff 标准库源码：`std/` 和 `prelude/`。

## 开发约定

- 保持改动聚焦，不要顺手重排无关代码或文档。
- 文件已经很长或模式重复时，先考虑职责边界和抽象是否需要调整。
- 新增公共语义时，同时更新对应 `doc/reference/` 或 `doc/architecture/` 文档。
- 不要提交本地状态、构建产物、secret 配置、package store、runtime home、截图或浏览器 profile。
- 被忽略的本地覆盖文件包括 `.skiff-instance/`、`.skiff-package-store/`、`skiff.local.yml`、`router/router.yml`、`runtime/runtime.yml`、`target/`、`node_modules/` 和 `build/`。

## 本地语言实例

开发 compiler、runtime、router 或 telemetry 时，可以为当前 worktree 创建独立本地 instance：

```bash
node scripts/skiff.mjs instance init .skiff-instance/config.yml
node scripts/skiff.mjs instance up .skiff-instance/config.yml
node scripts/skiff.mjs instance status .skiff-instance/config.yml
```

默认生成端口：

- `ports.base + 0`（默认 `4100`）：service HTTP。
- `ports.base + 1`（默认 `4101`）：router control/runtime WebSocket。
- `ports.base + 2`（默认 `4102`）：telemetry。

MongoDB 是本机共享开发基础设施，不随 `ports.base` 偏移；默认仍是独立的
`ports.mongo: 27017`。如果要让两个目录 instance 同时运行，创建第二个
instance 后，在它的配置文件中使用不同起始端口：

```yaml
ports:
  base: 4200
  mongo: 27017
```

结束后关闭：

```bash
node scripts/skiff.mjs instance down .skiff-instance/config.yml
```

如果改动 runtime、artifact identity、artifact schema、native signature、runtime protocol 或 artifact 加载语义，使用 `instance up` 构建并启动。它会校验受管 runtime binary 的 SHA-256 identity；磁盘 binary 已变化时会自动精确重启该 instance 的 runtime，不需要再人工串联 build 与 restart：

```bash
node scripts/skiff.mjs instance up .skiff-instance/config.yml
```

`node scripts/build-dev-runtime.mjs` 默认读取同一 instance config，安装 binary 后自动执行
`instance refresh-binaries`。自定义 instance 应传 `--config <path>` 和匹配的 `--dev-home <dir>`。
只有明确需要纯 build/install 时才传 `--no-refresh`；输出会标记 active runtime 可能 stale，并给出
`instance refresh-binaries <config>` 恢复命令。`instance build` 同样保留 build-only 语义，但会报告
仍在运行旧 binary 的 PID 和恢复命令。

纯编译和单元验证不需要启动 instance：

```bash
cargo check --manifest-path runtime/Cargo.toml
cargo test --manifest-path runtime/Cargo.toml --no-fail-fast
```

## 测试入口

仓库根有三个语义不同的权威入口：

```bash
cargo test --workspace --no-fail-fast  # Rust 测试所有权入口
pnpm test                              # Node/TypeScript 测试和 type-check 所有权入口
pnpm verify                            # 完整非 live 组合验证：Rust + Node/TS + checker
```

这里的入口按测试所有权划分，不代表进程树只使用一种语言工具链。`pnpm test` 不调度 Rust
workspace tests，但 Node-owned tests 和 fixture build 可能调用 Cargo；`cargo test --workspace
--no-fail-fast` 拥有完整 Rust tests，其中 test-runner runtime integration 会通过 Node host 启动
隔离 runtime。`pnpm verify` 组合前两类测试和 checker，不另行维护底层测试清单。

跨语言计划只在 `scripts/verify.mjs` 中维护。可以先审计展开后的命令而不执行：

```bash
node scripts/verify.mjs --list
node scripts/verify.mjs --only rust --list
node scripts/verify.mjs --only node --list
```

默认入口不运行 live 检查；需要时显式使用 `--only runtime-live` 或
`--only db-encrypted-storage-live`。compiler boundary 和受管 compiler crates 的实际
rustdoc public API 检查都属于默认 `checks` gate；`--only compiler-boundaries` 只用于
聚焦运行 source-boundary checker。

常用聚焦测试：

```bash
node scripts/verify.mjs --only router
node scripts/verify.mjs --only telemetry
node scripts/verify.mjs --only scripts
node scripts/verify.mjs --only vscode
node scripts/verify.mjs --only checks
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test
pnpm --filter @skiff/telemetry type-check
pnpm --filter @skiff/telemetry test
pnpm --dir scripts type-check
```

## Runtime Stack

Skiff release-mode 拓扑分成 router、runtime 和 artifacts：

- router 负责 service HTTP、control HTTP 和 runtime WebSocket。
- runtime 主动连接 router，注册当前 loaded service。
- artifacts 是文件系统里的不可变 build record 和 release pointer。
- release pointer 指向 immutable build id；router 根据 service + release/version 找到 build id，再把请求分发给注册了同一 build id 的 runtime。

release-mode HTTP 调用必须使用 selector headers：

- `X-Skiff-Service: <service-id>`
- `X-Skiff-Version: <release-id>`

没有 service/version selector、release 不存在、runtime 未注册都应该 fail closed。

部署 runtime stack 时使用显式远端：

```bash
node scripts/deploy-runtime-stack.mjs \
  --remote <user@host> \
  --only all \
  --runtime-binary build/cargo-target/x86_64-unknown-linux-gnu/release/runtime
```

示例 router/runtime 配置：

```yaml
artifacts:
  root: /opt/skiff/artifacts
releaseMode: true
http:
  port: 4000
runtime:
  port: 4001
  path: /runtime
```

```yaml
router: ws://127.0.0.1:4001/runtime
runtime-home: /opt/skiff/runtime-home
```

更新 artifacts 或 release pointer 后，可以调用 control reload：

```bash
curl -X POST http://127.0.0.1:4001/__skiff/reload-artifacts
```

## 文档维护

`doc/overview.md`、`doc/reference/` 和 `doc/architecture/` 是公开文档集合。已过期的临时计划、执行记录和历史草案不要放回公开仓库；必要的稳定规则应并入 canonical 文档。
