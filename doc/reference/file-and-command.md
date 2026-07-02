# Skiff File And Command Reference

> 当前落地能力优先使用不可变文件对象。`AppendLog`、`DirectoryTree` / workspace 等能力仍作为方向描述保留。

本文负责：Skiff 中持久文件对象、追加日志、目录工作区和命令执行能力的语义边界，以及这些能力在分布式 runtime 下的实现约束。

本文不负责：具体 OSS / S3 / NAS / GridFS SDK 适配、对象存储签名格式、文件 GC 扫描器实现、worker 调度存储 schema、构建系统协议、业务应用自己的文件权限模型。

## 定位

Skiff 不应把一个通用 POSIX 文件系统暴露为核心能力。runtime 是分布式的，请求可以落到任意 runtime 实例；任何持久文件能力都必须以共享后端为 source of truth，不能依赖某台 runtime 机器的本地磁盘。

文件能力应拆成三类：

- `ROFile`：不可变文件对象，适合大多数业务文件和 artifact。
- `AppendLog`：追加型分段日志，适合流式输出和事件记录。
- `DirectoryTree` / workspace：显式挂载式工作区，适合需要真实路径的工具链。

这三类能力不互相伪装。附着到 `db object` 的 `type` stored field 仍应遵守数据库记录大小限制；超过普通记录边界的内容应显式建模为文件或日志，而不是由 DB 字段透明变成文件。

## ROFile

`ROFile` 是 Skiff 的默认文件对象语义：

- 写入完成后不可修改。
- 通过 opaque `FileRef` 引用。
- 应用代码看不到 bucket、object key、NAS path、GridFS id 或后端类型。
- 更新内容时创建新文件，再让业务 DB 记录指向新的 `FileRef`。

`FileRef` 至少包含可见元信息：

- file id
- size
- sha256
- content type
- created at

后端选择：

- 生产优先使用 OSS / S3 / MinIO 这类对象存储。
- 无对象存储的自包含部署可以使用 GridFS 作为后端。
- 本地 filesystem 只能作为单机 dev adapter，不能成为分布式生产语义。

适用场景：

- 请求体归档。
- 已完成响应体归档。
- package source archive。
- build artifact。
- 用户上传附件。
- 图片、报告、导出文件。

## AppendLog

Skiff 不应把云对象存储的 append API 作为可移植核心语义。不同云厂商的 append 支持并不一致，而且对象存储 append 不能和业务 DB transaction 原子提交。

追加型内容应建模为分段日志：

```text
AppendLog(fileId, state, createdAt, closedAt, ...)
AppendChunk(fileId, seq, bytes, sha256, createdAt)
```

实现可以使用 MongoDB 或其他强一致 metadata store：

- `(fileId, seq)` 唯一约束保证单个 chunk 顺序位置只写一次。
- expected seq / offset 可用于 CAS。
- 单 writer lease 可用于避免并发写入同一个 log。
- close / finalize 状态表示不再接受新 chunk。

读取时按 `seq` 顺序拼接 chunk。关闭后的日志可以选择 compact 成 `ROFile`，再由 GC 清理分段 chunk。

适用场景：

- 响应流捕获。
- trace stream。
- agent transcript。
- audit / event stream。
- telemetry batch。

`AppendLog` 表示“可追加的业务流”，不是“可以修改已有文件”。已提交的 `ROFile` 不支持 append。

## DirectoryTree And Workspace

目录树不应是核心文件对象 API。需要稳定表示一组文件时，优先使用 manifest：

```text
FileManifest:
  entries: Array<{ path, fileRef, mode?, sha256 }>
```

manifest 适合：

- package / source tree。
- build artifact bundle。
- 静态网站资源。
- 用户上传的项目文件集合。
- 文档工程及附件。
- 数据集分片集合。

需要可变目录树时，应把目录节点建模到 DB：

```text
FileNode(id, parentId, name, kind, fileRef, version, metadata)
```

rename / move / list / delete 是 DB 操作；文件内容仍指向不可变 `FileRef`。

真正需要 POSIX path 语义的场景应使用显式 workspace / volume 能力，由网络文件系统实现：

- Alibaba Cloud NAS / CPFS。
- AWS EFS。
- Google Filestore。
- Azure Files / Azure NetApp Files。

适用场景：

- git checkout。
- compiler / build workspace。
- language server workspace。
- IDE-like project workspace。
- 需要真实路径的第三方工具。
- unpack / edit / repack 工作流。

Skiff 可以保证 workspace 的持久化、隔离、quota、挂载和生命周期钩子，但不承诺跨应用的强 POSIX transaction。并发正确性由应用自己保证；Skiff 可以后续提供 advisory lock，但不能把目录 mutation 包装成 portable DB transaction。

## Transaction Boundary

文件内容写入不是 service-owned DB transaction 的一部分。事务可见性应由业务 DB 中的引用控制。

推荐提交模型：

```text
1. runtime 写共享后端，生成 staged file 或 staged log。
2. runtime 校验 size / sha256 并写入文件 metadata。
3. 业务 DB transaction 写入 FileRef / AppendLogRef。
4. DB commit 后，文件对业务可达。
5. DB 失败、runtime 崩溃或 writer abort 留下的 staged / orphan 文件由 GC 清理。
```

这个模型保证应用层可见性原子：业务记录要么仍指向旧文件，要么指向一个完整、校验通过的新文件。底层对象写入和 DB transaction 不假装是同一个物理事务。

## Command Execution

Skiff 应提供命令执行能力，但不应暴露成普通请求路径中的 `std.command.exec()`。

命令执行适合的场景：

- 编译 package / service source。
- 运行测试、lint、formatter。
- 构建 artifact。
- 操作 git / source workspace。
- 文档、图片、媒体转换。
- 在受控 worker 中运行用户提供的工具链。

命令执行和普通 service request 的风险不同：

- 可能长时间运行。
- 资源消耗大。
- 依赖 filesystem / workspace。
- 可能需要网络和 secret。
- 不可自然参与 DB transaction。
- 需要审计、隔离、超时和取消。

推荐暴露为 `std.work` 或类似 worker job 能力，而不是同步 shell API。

输入应显式声明：

- argv，默认不接受 shell string。
- container / image / toolchain。
- workspace volume 或 input `FileManifest`。
- stdout / stderr capture policy。
- cpu、memory、disk、timeout 限制。
- network policy。
- secret mounts。
- environment variables。
- allowed filesystem mounts。
- service identity 和 audit context。

输出应结构化：

- exit code。
- stdout / stderr 的 bounded preview 或 `AppendLogRef`。
- produced `FileRef` / `FileManifest`。
- timing / resource usage。
- failure reason。

普通 service 提交 work item，并等待、轮询或订阅结果。底层调度、lease、timeout、cancel 和重试策略应复用 work capability 中的 durable queue / timer 语义。

## 当前不支持

- 把本地 runtime 磁盘作为生产文件 source of truth。
- 把对象存储 append 作为跨后端 portable 文件 append 语义。
- 对已提交 `ROFile` 原地修改、truncate、seek write 或 append。
- 通用 POSIX filesystem API 作为 Skiff 核心 std surface。
- 在普通 request handler 中直接执行任意 shell command。
- 文件写入和业务 DB transaction 的物理原子提交。

## 未定问题

- `FileRef` / `AppendLogRef` 的精确 std 类型和序列化 shape。
- 文件 metadata 是每个 service 独立系统 collection，还是平台级 file registry。
- file GC 的 lease、retention、orphan 扫描和引用追踪规则。
- append log compact 到 `ROFile` 的触发策略和成本模型。
- workspace 的 quota、snapshot、export 和 advisory lock shape。
- `std.work` 的 manifest 字段、worker image 解析、secret 注入和网络策略。
