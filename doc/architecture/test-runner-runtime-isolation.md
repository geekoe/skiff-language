# Test Runner Runtime Isolation

本文定义非 live runtime-backed test 与本地 runtime stack 的隔离边界。它描述 runner 和
CLI 的内部编排契约，不改变 `test` 语法、测试发现或 effect policy。

## Ownership Boundary

普通 `skiff test` 不借用开发者常驻的 router、runtime、artifact root 或 build root。CLI 为
每次命令拥有一套临时 instance：

- router HTTP/control 使用 `46000`–`46999` 内租约保护的动态端口；
- artifact、build、runtime home、pid、log 和 config 都位于同一个临时 workspace；
- Mongo 可以读取本机 `127.0.0.1:27017`，但 mongo、telemetry 和 watch 组件不由临时
  instance 启动；
- router/runtime binary 和 router source 来自执行 CLI 的当前 Skiff checkout。

Router 不接受空 artifact root。CLI 必须在 supervisor 启动前，用当前 checkout 的
`skiff-dev-sync` 向临时 artifact/build root 写入专用 bootstrap service，并显式
`--no-reload`。readiness 先通过带 service/version selector 的 isolated HTTP bootstrap 路由
触发 lazy runtime；路由响应成功后，还必须同时观察 isolated health、临时 artifact root 和
bootstrap service 的 runtime registration。health 200 本身不代表 runtime 已能 dispatch。

## Runner Contract

CLI 向 Rust runner 显式注入 `SKIFF_DEV_RELOAD_URL`、`SKIFF_TEST_ARTIFACT_ROOT` 和
`SKIFF_DEV_HOME`。非 live runtime path 在任何 health/reload 网络请求前验证前两个值同时
存在；缺任一都 fail closed，不能 fallback 到 `127.0.0.1:4001` 或 health 返回的常驻 root。
live mode 保留显式 harness 和原有 fallback。

`--service-artifact-root` 始终是只读输入。非 live runner 把输入 root 的全部 service
pointers 和所需 artifact 复制进临时 root，使 direct dependency 的 transitive runtime
closure 可用；它仍校验 publication 声明的 direct service IDs 确实存在。live mode 不扩大
复制范围。runner 调用 `skiff-dev-sync` 时必须显式传入临时 build root。

## Lifecycle And Recovery

父 CLI 持有 supervisor，并对正常返回、测试失败、startup 失败、`SIGINT` 和 `SIGTERM`
执行同一 cleanup：

1. 请求 supervisor 停止；
2. 无论 supervisor 状态如何，使用本次临时 config 执行 owner-verified `instance down`；
3. 用 `instance status` 确认 router/runtime stopped，再确认所有租约端口关闭；
4. 释放端口租约；
5. 只有以上步骤全部成功才删除临时 workspace。

测试错误和 cleanup 错误必须同时保留。cleanup 失败时保留 workspace 和日志路径作为诊断
证据。startup 在 config 可读且 supervisor 已尝试启动后才执行 instance ownership cleanup；
config write 或 bootstrap 阶段失败没有 owned process，不应因为读取半成品 config 制造二次
错误。

Mongo delayed database finalizer 不属于临时 stack：它只能携带合成 database 名并连接
Mongo，不能持有 artifact、runtime home、router/runtime 或 supervisor 资源。临时 stack
cleanup 不等待每个 test case 的 6.5 秒 finalizer。
