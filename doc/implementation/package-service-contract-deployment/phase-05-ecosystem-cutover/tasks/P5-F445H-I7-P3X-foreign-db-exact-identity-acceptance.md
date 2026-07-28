# P5-F445H-I7-P3X Foreign DB exact identity acceptance

状态：`RED_FROZEN_AWAITING_REPAIR`。

## 1. Purpose

独立验收P3C、P3R0、P3R0B和P3R1合并后的真实纵向链路：

```text
test service source
  -> compiler/lowering
  -> linked exact DbObjectTargetId
  -> Host activation metadata
  -> Eval DB operation
  -> service-db capability seam
```

验收不得用手写linked fixture替代真实compiler产物。两个provider package都声明
`model.Session`和物理名为`sessions`的DB object，但拥有不同的exact
`PackageArtifactRef`，测试服务把它们分别映射到`first_sessions`和`second_sessions`。

## 2. Baseline and ownership

| 项 | 值 |
| --- | --- |
| baseline commit | `e1530c6a0bdbc7ee4bf6ef9094de7e9a965a3b9e` |
| baseline tree | `50fba533c5698f06d556274db94eb11f0e3d7be4` |
| branch | `codex/p5-f445h-i7-p3x-foreign-db-acceptance` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p3x-foreign-db-acceptance` |
| integration owner | `/root/phase05_integration_steward` |

零worktree预检确认integration基线干净且精确匹配后，才创建上述worktree。

## 3. Write scope

本节点只允许：

- Host测试模块中的真实纵向验收；
- 本task，以及最终转绿后的result；
- fake DB provider/store和临时authoring fixture等test-only代码。

本节点禁止：

- production实现或production API修改；
- Cargo dependency、feature或schema修改；
- network、Mongo、stable instance、OAuth、browser或外部状态；
- push。

若RED需要修改production seam或扩张跨crate测试架构，必须停止并上报。

## 4. Required assertions

- compiler生成的两个DB target分别携带exact package artifact、artifact-owned File IR和
  local type index；
- Host activation给provider的两份metadata必须保持上述exact identity；
- 两个同名`Session`不得因module/type display name或相同File IR内容发生碰撞；
- 至少一次read和一次write必须到达fake store，并分别命中`first_sessions`和
  `second_sessions`；
- substituted package、file或type必须fail closed；
- D7 identical diamond仍只投影一份canonical provider build；
- compiler、linker、capability-context、service-db、Eval、Host locked suites通过；
- locked checks、rustfmt和`git diff --check`通过。

## 5. Frozen RED

真实链路已经到达Host activation，且两份`DbProviderTargetMetadata`具有不同的exact
package/file identity、相同的local `typeIndex = 0`，collection分别为
`first_sessions`和`second_sessions`。请求随后在任何`DbCapabilityStoreApi`调用之前
失败：

```text
InvalidArtifact("DB target type index declaration is ambiguous")
```

真实File IR的`declarations.types`只有一项：

```text
map key: Session
declaration.symbol: model.Session
typeIndex: 0
```

Eval `resolve_db_declaration`把未限定的map key和限定的declaration symbol直接比较，
将这个合法形状误判为歧义。该production repair不属于本验收节点；P3X保留RED测试，
等待repair commit后临时join并完成独立GREEN验收，届时再写最终result。

