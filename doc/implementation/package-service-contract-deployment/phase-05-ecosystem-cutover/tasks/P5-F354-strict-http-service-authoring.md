# P5-F354 Strict named HTTP service authoring

状态：Ready（C1 HTTP authoring leaf）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F347-external-ingress-compiler-artifact-audit-result.md`
- `P5-F351-gateway-artifact-model-identity-result.md`

H36覆盖F347中旧`routes`/WebSocket建议。本任务只实现已冻结HTTP authoring；WebSocket保持待设计。

## DAG位置与目标

把`ServiceManifestAuthoring.http: Option<Value>`替换为严格named mapping，复用F351 HTTP enum/arg owner，
并在真正compile input边界fail closed。该leaf不解析handler到PackageCallableId，不生成gateway entry。

唯一目标shape：

```yaml
http:
  createUser:
    method: POST
    path: /users
    kind: typedJson
    handler: users.create
    adapterArgs:
      - param: body
        source: { kind: http.body }
```

必须完成：

1. `http`是`GatewayEntryKey -> HttpGatewayEntryAuthoring` mapping，没有`routes`/`entries`/`id`；
2. entry严格拥有optional host（缺省`*`）、required method/path/kind/handler、entry-local optional
   guard/pre与adapterArgs；全部递归deny unknown；
3. key走`GatewayEntryKey` validation；handler/guard/pre走当前Package source selector词法规则，不允许
   public path fallback；
4. method/path/host与F351 adapter args在input边界验证；`http.context`要求pre，raw HTTP拒绝body source；
5. 任何旧`operation`、`handlerArgs`、global guard/pre、`routes`、`entries`或unknown field立即失败；
6. `websocket`字段及其现有迁移实现不扩展、不重命名、不据此新增shared type；
7. 尚未接线的旧generated-deployment HTTP operation路径必须明确fail closed，不能静默忽略、转回
   ContractOperationId或保留dual reader。

## 写入范围

允许修改：

- `artifact-model/src/ecosystem_authoring.rs`及直接exports/tests；
- `compiler/input/src/service_config.rs`及直接tests；
- source selector parser的最小复用exports；
- `compiler/driver/generated_deployment.rs`中关闭旧HTTP operation consumer所需的最小fail-closed改动；
- 专用fixtures。

禁止修改：

- F351 gateway model/identity；
- PackageArtifact/serviceCall/generic projection；
- deployment DTO/identity/projection；
- runtime/router/test-runner、WebSocket authoring、三仓库service、live fixtures、lockfile。

## 验证

```bash
cargo test -p skiff-artifact-model service_manifest -- --list
cargo test -p skiff-compiler-input service_config -- --list
cargo test -p skiff-compiler generated_service_deployment -- --list
cargo test -p skiff-artifact-model service_manifest
cargo test -p skiff-compiler-input service_config
cargo test -p skiff-compiler generated_service_deployment
cargo fmt -p skiff-artifact-model -p skiff-compiler-input -p skiff-compiler -- --check
git diff --check
```

selector必须非零。正例至少覆盖typed/raw、default/explicit host、entry-local pre/context。负例逐项覆盖旧
fields、unknown/duplicate key、非法selector/key/path/method、typed/raw source错配、missing pre、
HTTP key顺序canonical，并反搜production不再按HTTP `operation`解析。

不运行workspace/root、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f354-http-authoring`
- branch：`codex/p5-f354-http-authoring`
- 从包含本task的integration checkpoint创建；result记录exact base/commit/tree。
- 提交production/tests，再提交result；worktree保持clean，不merge/rebase integration。
