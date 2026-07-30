# P5-F24D：WebSocket Shape Consumer Parity

依赖R25 PASS。与F24B并行；独占`runtime/linked-type-plan/**`、boundary test-support descriptor及直接parity tests，不改
boundary production matcher/eval/Router/artifact-model。一个clean commit。

linked runtime shape与test-support descriptor必须消费F24A canonical spec；若crate DAG禁止直接依赖，则使用由spec生成的
golden/parity corpus并证明所有field/order/nullable/union tag/Context placeholder exact，禁止保留第三份手写schema。
加入双向drift mutation，公开builtin集合不扩大。跑linked/boundary test-support精确tests、DAG check、fmt/diff-check；
禁止real smoke/full/I16/Host/stable。
