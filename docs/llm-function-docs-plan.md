# 基于 SCIP 的函数文档系统

## 1. 核心设计

系统只需要四个部分：

1. 用户点击函数。
2. 如果函数没有文档，就把一个任务放入 RabbitMQ。
3. 唯一的 worker 每次从队列取一个任务，调用 LLM 生成文档。
4. 文档以 JSON 文件保存，前端在右侧面板显示。

```text
浏览器点击函数
      |
      v
检查 JSON 文档是否存在
      |
      +-- 存在 --> 直接显示
      |
      `-- 不存在 --> 加入队列
                         |
                         v
                  单个 worker 取一个任务
                         |
                         v
                  在固定预算内分析代码
                         |
                         v
                    写入 JSON 文档
```

RabbitMQ 只使用一个普通队列和一个 worker。worker 一次只处理一个任务，所以
不需要数据库、任务文件、分布式锁、lease 或事务。

## 2. 页面布局

- 左侧：文件列表和浏览历史。
- 中间：源代码。
- 右侧：当前函数的文档或生成状态。

用户点击一个函数时，页面通过 SCIP occurrence 找到它的定义和 symbol。

## 3. 函数标识

不能使用函数名或行号作为 Key，因为函数可能重载，行号也会变化。

全局 SCIP symbol 使用：

```text
repo + commit + scipSymbol
```

SCIP local symbol 还要加入文件路径：

```text
repo + commit + filePath + scipSymbol
```

对以上字符串计算 SHA-256，得到 `docKey`。

文档路径：

```text
docs/{repo}/{commit}/{docKey}.json
```

每个 commit 使用独立文档，不能把旧 commit 的文档直接当成新 commit 的
文档。

## 4. SCIP 需要提供的数据

生成静态网站时，从 SCIP 中为每个函数保留：

```json
{
  "docKey": "...",
  "repo": "github-com-example-project",
  "commit": "abc123",
  "file": "src/example.cc",
  "language": "cpp",
  "scipSymbol": "...",
  "kind": "Function",
  "displayName": "example",
  "signature": "bool example(Context* ctx)",
  "definitionRange": [120, 4, 120, 11],
  "enclosingRange": [119, 0, 168, 1],
  "existingDocumentation": [],
  "relationships": [],
  "diagnostics": [],
  "sourceHash": "..."
}
```

主要使用这些 SCIP 数据：

- `SymbolInformation.symbol`
- `SymbolInformation.kind`
- `display_name`
- `documentation`
- `signature_documentation`
- `relationships`
- definition occurrence 的 `symbol_roles`
- occurrence range 和 enclosing range
- diagnostics

SCIP relationship 不是完整调用图。函数体中引用另一个函数 symbol，只能记录
为“引用的函数”，不能直接断言一定发生了调用。

当前 `site.rs` 没有保留全部这些字段，实现时需要补充输出。

## 5. RabbitMQ 任务

使用一个队列：

```text
function-doc.generate
```

任务消息只保存目标函数，不在消息中保存状态或预算：

```json
{
  "taskId": "task-20260705-0001",
  "docKey": "...",
  "repo": "github-com-example-project",
  "commit": "abc123",
  "createdAt": "2026-07-06T03:15:00Z",
  "retryCount": 0
}
```

预算是 worker 的固定配置，不能由消息或浏览器指定。

### 入队规则

用户点击没有文档的函数时：

1. 再检查一次 `docs/{repo}/{commit}/{docKey}.json`。
2. 如果文档已经存在，直接返回。
3. 否则向 `function-doc.generate` 发送消息。

不需要查询 RabbitMQ 中是否已经有相同任务。偶尔产生重复消息没有关系，worker
消费后会再次检查 JSON 文档；文档已经存在就直接 ACK 并跳过。

### 消费规则

只运行一个 consumer，并设置 `prefetch=1`。worker 永远只做：

```text
从 RabbitMQ 取一个任务
-> 检查目标 JSON 是否已经存在
-> 在固定预算内生成文档
-> 写入 JSON
-> ACK 消息
-> 取下一个任务
```

一次任务可以为目标函数和相关函数生成多个 JSON 文档，但不会为相关函数创建
新任务。所有相关函数共享原任务的预算。

LLM 或网络临时失败时，如果 `retryCount` 是 0，就重新发送一次
`retryCount: 1` 的消息并 ACK 原消息。第二次仍然失败则直接 ACK 丢弃。预算耗尽
不是系统错误：保存已经生成成功的文档，然后 ACK。

不设计死信队列、延迟重试、任务状态持久化或结果队列。前端入队后只需要每隔
几秒请求目标 JSON；文件出现后显示文档，超时后显示“稍后重试”。

## 6. worker 如何使用 Codex 执行任务

Codex 不负责消费 RabbitMQ，也不直接写最终文档。worker 取到消息后只调用一次
非交互式 `codex exec`。

### 准备输入

worker 根据 `docKey` 读取 SCIP 数据，然后创建一个临时只读任务目录：

```text
/tmp/function-doc/{taskId}/
  task.json
  target.cc
  related/
    01-helper.cc
    02-validate.cc
```

- `task.json` 保存目标函数的 SCIP symbol、签名、原有文档、diagnostics 和源码
  范围。
- `target.cc` 只保存目标函数源码。
- `related/` 最多保存预算允许的相关函数源码。
- 不把整个仓库交给 Codex，避免 Codex 无限制搜索代码。

### 调用 Codex

worker 在临时目录中执行：

```bash
codex exec \
  --ephemeral \
  --sandbox read-only \
  --ignore-user-config \
  --ignore-rules \
  --model "$CODEX_MODEL" \
  --json \
  --output-schema /app/function-doc-output.schema.json \
  -o "/tmp/function-doc/${taskId}/result.json" \
  -
```

最后的 `-` 表示从 stdin 读取完整任务 prompt。worker 应该通过进程 API 传递
参数和 stdin，不要拼接 shell 字符串。

`--ephemeral` 避免保存 Codex session，`read-only` 防止修改任务目录，
`--output-schema` 要求最终结果符合固定 JSON Schema。worker 从 `--json` 的
`turn.completed.usage` 事件读取实际 token 用量，从 `result.json` 读取最终文档。

prompt 保持简单：

```text
阅读 task.json、target.cc 和 related/ 中提供的有限代码。
为目标函数生成 contract、implementation 和 possibleBugs。
可以为确实理解充分的相关函数生成文档，但不得超过输出 schema 的数量限制。
SCIP referenced symbol 不能直接描述成确定的函数调用。
Possible bug 必须包含源码位置和验证方法；没有证据时返回空数组。
只输出符合 schema 的 JSON，不修改任何文件。
```

### Codex 输出

Codex 一次返回一个对象：

```json
{
  "documents": [
    {
      "docKey": "...",
      "contract": {},
      "implementation": {},
      "possibleBugs": [],
      "references": []
    }
  ]
}
```

worker 不直接相信输出。它负责：

1. 使用 JSON Schema 再验证一次。
2. 检查每个 `docKey` 都来自本次提供的 SCIP functions。
3. 检查源码路径和 range。
4. 加入 commit、SCIP symbol、source hash、模型名称和生成时间。
5. 将每个有效文档写入对应的最终 JSON 文件。
6. Codex 成功但个别相关文档无效，只丢弃无效文档。

Codex 进程设置一个固定超时，例如 120 秒。超时就结束进程，并按照 RabbitMQ
规则最多重试一次。

### Codex 预算

`codex exec` 作为一次完整 agent 执行，CLI 不能在运行过程中精确保证一个总 token
上限。因此严格限制使用可以在执行前确定的三项：

```json
{
  "maxFunctions": 20,
  "maxSourceBytes": 200000,
  "maxDocs": 8,
  "timeoutSeconds": 120
}
```

worker 在启动 Codex 前裁剪临时目录，确保函数数和源码字节数不会超限；JSON
Schema 使用 `maxItems` 限制文档数量；进程超时限制运行时间。token 使用量用于
记录和监控，但不宣称是 Codex CLI 能严格中止的预算。如果必须精确控制 token，
应改用支持对应限制的模型 API，而不是 `codex exec`。

## 7. 什么是函数文档

一个函数文档只包含三部分：契约、实现和可能的 Bug。

### Contract

说明调用者需要知道的行为：

- 函数目的
- 参数和参数约束
- 返回值
- 前置条件和后置条件
- 副作用
- 错误处理
- 所有权和生命周期
- 线程安全

### Implementation

说明当前 commit 中的实现：

- 主要实现策略
- 关键步骤和分支
- 读取或修改的状态
- 引用的相关函数
- 清理和失败路径
- 复杂度或资源使用

### Possible bugs

可能的 Bug 必须是待验证的假设，不是确定结论：

- Bug 类型和严重程度
- 触发条件
- 可能的失败原因
- 影响
- 对应源代码位置
- 如何验证
- 置信度

没有足够证据时，`possibleBugs` 必须是空数组，不能要求 LLM 强行寻找 Bug。

## 8. 文档 JSON 格式

```json
{
  "schemaVersion": 1,
  "docKey": "...",
  "subject": {
    "repo": "github-com-example-project",
    "commit": "abc123",
    "file": "src/example.cc",
    "scipSymbol": "...",
    "kind": "Function",
    "displayName": "example",
    "signature": "bool example(Context* ctx)",
    "definitionRange": [120, 4, 120, 11],
    "sourceHash": "..."
  },
  "contract": {
    "summary": "函数的外部行为。",
    "parameters": [
      {
        "name": "ctx",
        "description": "上下文对象。",
        "constraints": ["不能在调用期间被释放。"]
      }
    ],
    "returns": "成功时返回 true。",
    "preconditions": [],
    "postconditions": [],
    "sideEffects": [],
    "errors": [],
    "ownership": [],
    "threadSafety": "unknown"
  },
  "implementation": {
    "summary": "实现方式。",
    "steps": ["检查输入", "更新状态", "返回结果"],
    "branches": [],
    "stateChanges": [],
    "relatedFunctions": [
      {
        "scipSymbol": "...",
        "relationship": "referenced",
        "description": "用于验证输入。"
      }
    ],
    "failurePaths": [],
    "complexity": "unknown"
  },
  "possibleBugs": [
    {
      "title": "部分失败后可能保留旧状态",
      "category": "correctness",
      "severity": "medium",
      "confidence": 0.63,
      "trigger": "第二步失败时。",
      "reason": "第一步写入的状态没有回滚。",
      "impact": "后续调用可能读取不一致状态。",
      "validation": "注入第二步失败并检查状态。",
      "source": [
        {"file": "src/example.cc", "range": [140, 4, 151, 5]}
      ]
    }
  ],
  "references": [
    {"file": "src/example.cc", "range": [119, 0, 168, 1]}
  ],
  "limitations": ["SCIP 引用不能证明一定发生函数调用。"],
  "generation": {
    "model": "准确的模型名称",
    "provider": "模型供应商",
    "generatedAt": "2026-07-06T03:15:00Z",
    "promptVersion": "function-doc-v1",
    "budgetUsed": {
      "inputTokens": 12000,
      "outputTokens": 2200,
      "functions": 6,
      "docs": 3
    },
    "stopReason": "completed"
  }
}
```

每个文档都必须记录模型名称、commit、SCIP symbol 和 source hash。

## 9. 固定预算

对于 Codex worker，固定预算使用执行前可以严格检查的四项：

```json
{
  "maxFunctions": 20,
  "maxSourceBytes": 200000,
  "maxDocs": 8,
  "timeoutSeconds": 120
}
```

- `maxFunctions`：最多读取多少个函数。
- `maxSourceBytes`：交给 Codex 的源码总字节数。
- `maxDocs`：最多写入多少个函数文档。
- `timeoutSeconds`：一次 Codex 进程最长执行时间。

worker 在调用 Codex 前完成相关函数选择和裁剪，不能把超出预算的文件放入临时
目录。目标函数必须最先保留，空间不足时依次丢弃距离较远的相关函数。

模型名称和 Codex 报告的 token usage 仍然记录在最终文档中，用于观察成本。任务
失败最多重试一次。

## 10. 相关函数分析顺序

worker 按以下顺序使用预算：

1. 目标函数源码。
2. 目标函数的 SCIP 签名、原有文档和 diagnostics。
3. 目标函数 enclosing range 内引用的函数 symbols。
4. 同文件相关函数。
5. 仍有预算时再读取跨文件函数。

达到预算后立即停止扩展。目标函数文档优先于相关函数文档。

## 11. 实现顺序

1. 扩展 `site.rs`，输出函数需要的 SCIP 数据和 `docKey`。
2. 页面右侧增加文档面板，左侧保留文件和历史。
3. 实现 JSON 文档读取。
4. 接入一个 RabbitMQ 队列，运行单 consumer，并设置 `prefetch=1`。
5. 根据 SCIP 创建受预算限制的临时任务目录。
6. 使用只读、ephemeral 的 `codex exec` 和 JSON Schema 生成结构化结果。
7. 验证 `docKey`、commit、SCIP symbol、source hash 和源码范围。
8. 支持一次任务生成少量相关函数文档。
