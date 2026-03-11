# actr-version

🔒 **基于语义 Proto 分析的协议兼容性分析库**

`actr-version` 通过利用 `proto-sign` 进行语义破坏性变更检测和 `actr-protocol` 服务结构，提供专业级的 protobuf 兼容性分析，实现全面的服务版本管理。

## 🎯 设计哲学

此库解决了协议兼容性分析的**真正问题** - 不仅仅是比较哈希值，而是理解 protobuf 模式变更的**语义含义**。

```rust
// 使用 proto-sign 进行真正的语义分析
let result = ServiceCompatibility::analyze_compatibility(&old_service, &new_service)?;
// 检测：字段移除、类型变更、向后兼容性等
```

## 📦 核心功能

- **语义 Proto 分析**：使用 `proto-sign` 进行专业的破坏性变更检测
- **服务级兼容性**：分析来自 `actr-protocol` 的完整 `ServiceSpec` 结构
- **破坏性变更检测**：识别具体的破坏性变更并提供详细说明
- **高效指纹比较**：直接使用 `ServiceSpec` 内置的语义指纹
- **全面结果**：提供可操作见解的详细兼容性分析

## 🏗️ 依赖项

```toml
[dependencies]
actr-version = { git = "https://github.com/actor-rtc/actr-version" }
actr-protocol = { git = "https://github.com/actor-rtc/actr-protocol" }
```

## 🚀 快速开始

```rust
use actr_version::{ServiceCompatibility, CompatibilityLevel, Fingerprint, ProtoFile};
use actr_protocol::ServiceSpec;

// 创建包含实际 proto 内容的基础服务
let proto_files = vec![ProtoFile {
    name: "user.proto".to_string(),
    content: r#"
        syntax = "proto3";
        message User {
            string name = 1;
            string email = 2;
        }
    "#.to_string(),
    path: None,
}];

let base_fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)?;

let base_service = ServiceSpec {
    version: "1.0.0".to_string(),
    description: Some("用户管理服务".to_string()),
    fingerprint: base_fingerprint,
    protobufs: proto_files.into_iter().map(|pf| {
        actr_protocol::service_spec::Protobuf {
            uri: format!("actr://user-service/{}", pf.name),
            content: pf.content,
            fingerprint: "file_fp".to_string(),
        }
    }).collect(),
};

// 创建有破坏性变更的候选服务
let candidate_proto = vec![ProtoFile {
    name: "user.proto".to_string(),
    content: r#"
        syntax = "proto3";
        message User {
            string name = 1;
            // email 字段被移除 - 这是一个破坏性变更！
            int32 age = 3;  // 添加了新字段
        }
    "#.to_string(),
    path: None,
}];

let candidate_fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&candidate_proto)?;

let candidate_service = ServiceSpec {
    version: "1.1.0".to_string(),
    description: Some("用户管理服务".to_string()),
    fingerprint: candidate_fingerprint,
    protobufs: candidate_proto.into_iter().map(|pf| {
        actr_protocol::service_spec::Protobuf {
            uri: format!("actr://user-service/{}", pf.name),
            content: pf.content,
            fingerprint: "file_fp".to_string(),
        }
    }).collect(),
};

// 执行语义兼容性分析
let result = ServiceCompatibility::analyze_compatibility(&base_service, &candidate_service)?;

match result.level {
    CompatibilityLevel::FullyCompatible => {
        println!("✅ 未检测到变更 - 可安全部署");
    },
    CompatibilityLevel::BackwardCompatible => {
        println!("⚠️ 向后兼容的变更 - 可安全升级");
        for change in &result.changes {
            println!("  - {}: {}", change.change_type, change.description);
        }
    },
    CompatibilityLevel::BreakingChanges => {
        println!("❌ 检测到破坏性变更 - 需要协调升级！");
        for breaking_change in &result.breaking_changes {
            println!("  - {}: {}", breaking_change.rule, breaking_change.message);
        }
    }
}
```

## 🎯 核心 API

```rust
// 指纹计算
Fingerprint::calculate_proto_semantic_fingerprint(content: &str) -> Result<String>
Fingerprint::calculate_service_semantic_fingerprint(files: &[ProtoFile]) -> Result<String>

// 兼容性分析
ServiceCompatibility::analyze_compatibility(base: &ServiceSpec, candidate: &ServiceSpec)
    -> Result<CompatibilityAnalysisResult>
ServiceCompatibility::has_breaking_changes(base: &ServiceSpec, candidate: &ServiceSpec)
    -> Result<bool>
ServiceCompatibility::get_breaking_changes(base: &ServiceSpec, candidate: &ServiceSpec)
    -> Result<Vec<BreakingChange>>
ServiceCompatibility::are_semantically_identical(base: &ServiceSpec, candidate: &ServiceSpec)
    -> Result<bool>
```

## 🔍 实现细节

### 语义指纹系统

使用 `proto-sign` 库进行基于 AST 的语义分析：

```rust
// 单个 proto 文件的语义指纹
let fingerprint = Fingerprint::calculate_proto_semantic_fingerprint(proto_content)?;
// 格式: "semantic:abc123..."

// 服务级多文件语义指纹
let service_fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)?;
// 格式: "service_semantic:def456..."
```

### 兼容性分析引擎

分析流程：

1. **快速路径**：比较语义指纹，相同则返回 `FullyCompatible`
2. **深度分析**：使用 `proto-sign` 进行逐文件语义比较
3. **变更分类**：将变更分为破坏性和非破坏性
4. **结果汇总**：生成详细的兼容性报告

### 破坏性变更规则

以下变更被视为破坏性：

- ❌ 删除字段
- ❌ 修改字段类型
- ❌ 修改字段编号
- ❌ 删除 RPC 方法
- ❌ 修改方法签名
- ❌ 删除 proto 文件

### 向后兼容变更

以下变更被视为向后兼容：

- ✅ 添加 `optional` 字段
- ✅ 添加新的 RPC 方法
- ✅ 添加新的消息类型
- ✅ 添加新的 proto 文件

## 🧪 测试

本库拥有 **95%+ 测试覆盖率**，包含以下全面测试：

- ✅ 错误处理和边缘情况
- ✅ 破坏性变更检测
- ✅ 文件添加/移除场景
- ✅ 语义指纹计算
- ✅ 输入验证

```bash
cargo test
```

## 📄 许可证

Apache-2.0 License - 查看 [LICENSE](LICENSE) 文件
