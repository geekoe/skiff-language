# P5-F03C：Runtime Integration Repair Result

状态：complete。commit `d2452e046578f56b219fc2833bfcfbd30b13e50e`、tree
`c5142f4a5238f188e46e688226bcc09f8feaed88`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

Runtime production config删除`services: Vec::new()`旧语义；WS connect成功后发送F23E acquire，receive按connection pin使用旧
generation route，release/session disconnect幂等释放，最后pin释放后回收retired generation。断线后迟到connect与跨session
release replay均fail closed，lifecycle dispatch从混合router-session owner拆出。

聚焦测试3+2+2+25+22+3全部PASS，`cargo check -p runtime`、runtime DAG、diff-check及scoped extra-review均通过。没有修改
F23E wire、Router、compiler/test-runner或公共契约。该提交与F03B合流后仍需D40 provisioning、cheap combined及最终R05。
