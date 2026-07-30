# P5-D04：Test Infrastructure Repair Design

## 角色与输入

独立只读Agent审计T05 candidate `f8ad689`、当前T02–T04/F03A integration、T02/T05/F03C/I02合同与
权威设计。不得修改、提交或运行live/full gate；输出可一次实现的repair数据流、CLI表、owner与聚焦gate。

## 冻结结论

1. Runner的唯一外部依赖输入是canonical `--artifact-root`与按需`--base-assembly <AssemblyIdentity>`。
   `package.yml`中的package/contract dependency只从typed pointers/records解析，不递归编译dependency source。
   base assembly提供exact provider deployments、contracts、packages及config/state/resource/capability bindings；有
   runtime requirement但无base assembly时activation前fail closed。
2. non-live把外部artifact root当只读source，isolated harness复制exact closure到隐藏temporary runtime root，再
   写test overlay/deployment/assembly；live显式提供activation/ingress/environment/generation。
3. 真实consumer fixture由contract-first provider + helper package + consumer package/deployments组成；最终Host
   结果必须同时证明实际package callee共享heap mutation和service `InProcessBoundary` detached调用。不得手调
   mutation primitive或只检查binding。
4. 保留CLI：`--artifact-root`、`--base-assembly`、`--live`、`--activation-url`、`--ingress-url`、
   `--environment`、`--expected-generation`、`--deny-skips`、`--require-tests`。删除且不留parser：`--profile`、
   `--service-artifact-root`、`--config`、`--package-test-concurrency`、`--router-reload-url`、`--packages-dir`、
   `--allow-network`。config/state/resource只能来自test-owned deployment；旧ambient config/test-doubles整体退役，
   service mock必须是四对象provider。
5. 删除`UnsupportedStream` fixture bridge与artifact重签；I02使用已有WebSocket generation pin。若production
   authoring不能生成WS adapter，报告compiler owner blocker，不得恢复手工projection。本repair不扩compiler stream。
6. fixture拆为discovery、canonical package/store、package-test assembly、execution职责；test-only大corpus本身
   不因行数阻塞。F03C直接迁移runtime-host旧API，不在F04保留compat。
