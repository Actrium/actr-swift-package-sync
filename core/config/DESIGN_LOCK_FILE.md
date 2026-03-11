# Lock File Design: Reference-Based Proto Storage

## 设计理念

与 cargo/npm 等包管理器类似，lock 文件存储 **proto 文件的引用路径**（相对于 `proto/` 目录），而非嵌入 proto 内容。Proto 文件实际存储在项目的 `proto/remote/` 目录下。

## 设计对比

### 传统设计（cargo/npm 模式）

```
actr.lock.toml (5KB)
  - metadata
  - dependencies (只有引用)

~/.cache/actr/
  user-service/
    user.v1.proto (3KB)
    common.v1.proto (2KB)
  payment-service/
    payment.v1.proto (4KB)
```

**优势**：
- 适合大包（几百 MB）
- 多项目复用
- 减少重复存储

**劣势**：
- 复杂的 cache 管理
- 需要路径/URI 转换
- 容易出现不一致
- 不利于版本控制

### 新设计（引用模式）

```
actr.lock.toml (5KB)
  - metadata
  - dependencies (路径引用 + 指纹)

proto/remote/
  user-service/
    user.v1.proto (3KB)
    common.v1.proto (2KB)
  payment-service/
    payment.v1.proto (4KB)
```

**优势**：
- ✅ **单一数据源**：lock 文件 + proto 目录分离，但逻辑清晰
- ✅ **原子性强**：复制项目需复制 lock 文件和 proto 目录
- ✅ **版本控制友好**：git diff 显示 lock 文件变化
- ✅ **易于调试**：lock 文件结构简单
- ✅ **代码简化**：无需复杂的内容序列化

**适用性**：
- Proto 文件很小（单个 2-10KB）
- 需要明确的文件路径管理
- 支持 proto 文件的独立版本控制

## Lock 文件格式

```toml
[metadata]
version = 1
generated_at = "2025-01-15T10:30:00Z"

[[dependency]]
name = "user-service"
actr_type = "acme+user-service"
description = "User management service"
fingerprint = "service_semantic:a1b2c3d4e5f6"
published_at = 1705315800
tags = ["latest", "stable"]
cached_at = "2025-01-15T10:30:00Z"

  [[dependency.files]]
  path = "user-service/user.v1.proto"
  fingerprint = "semantic:xyz123"

  [[dependency.files]]
  path = "user-service/common.v1.proto"
  fingerprint = "semantic:abc789"
```

## name vs alias 语义说明

Lock 文件中的 `name` 字段与 actr.toml 中的 `alias` 字段有明确的分工：

### actr.toml (依赖配置)

```toml
[dependencies]
# alias 是用户在项目中引用该依赖的本地名称
my-echo = { name = "echo-service-v1", actr_type = "acme+EchoService" }
#        ↑
#        alias (本地引用名，可自定义)

shared-cache = { name = "redis-proxy", actr_type = "acme+CacheService" }
#             ↑
#             name (远程服务的唯一标识)
```

| 字段 | 位置 | 含义 | 作用 |
|------|------|------|------|
| `alias` | actr.toml | 本地引用名 | 代码中 `use crate::my_echo::*;` 导入生成的模块 |
| `name` | actr.toml / Actr.lock.toml | 远程服务唯一标识 | 服务注册表中的服务名称，用于服务发现和指纹验证 |
| `actr_type` | actr.toml / Actr.lock.toml | 服务类型标识 | 如 `"acme+EchoService"`，用于代码生成时的模块路径 |

### Actr.lock.toml (锁定的依赖)

> 说明：Actr.lock.toml **推荐**用于锁定远程依赖和 fingerprint，但对纯本地部署不是硬性要求。
> 缺少该文件时 runtime 仍然可以启动，只是无法使用基于 fingerprint 的精确依赖匹配，
> 远程发现将退化为协商流程或在首次远程调用时提示依赖未锁定。

```toml
[[dependency]]
# name 是远程服务的唯一标识，用于去重和服务发现
name = "echo-service-v1"
# actr_type 是服务类型，用于代码生成
actr_type = "acme+EchoService"
fingerprint = "service_semantic:..."
```

### 去重规则

- **多个 alias 指向同一 name**：只会在 lock 文件中生成一条记录
- **Lock 文件按 name 去重**：同一服务只有一个条目
- **安装时自动去重**：相同服务的多次安装请求会被合并

## 核心数据结构

```rust
/// Lock file structure
pub struct LockFile {
    pub metadata: Option<LockMetadata>,
    pub dependencies: Vec<LockedDependency>,
}

/// A locked dependency entry
pub struct LockedDependency {
    /// Service name (identifier in lock file, matches actr.toml dependency name property)
    pub name: String,

    /// Actor type (e.g., "acme+user-service")
    pub actr_type: String,

    /// Service description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Service-level semantic fingerprint (format: "service_semantic:hash")
    pub fingerprint: String,

    /// Publication timestamp (Unix epoch seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,

    /// Tags like "latest", "stable"
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// When this dependency was cached (ISO 8601)
    pub cached_at: String,

    /// Proto file references (path + fingerprint only, no content)
    #[serde(rename = "files")]
    pub files: Vec<LockedProtoFile>,
}

/// Proto file reference in lock file (NO content, just path and fingerprint)
pub struct LockedProtoFile {
    /// Relative path from project's proto/ folder (e.g., "user-service/user.v1.proto")
    pub path: String,

    /// Semantic fingerprint of the file (format: "semantic:hash")
    pub fingerprint: String,
}

/// Service specification metadata
pub struct ServiceSpecMeta {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub fingerprint: String,
    #[serde(rename = "files")]
    pub protobufs: Vec<ProtoFileMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Package-level protobuf entry in lock file
pub struct ProtoFileMeta {
    /// Relative path to the proto file (e.g., "remote/user-service/user.v1.proto")
    pub path: String,
    pub fingerprint: String,
}
```

## 双向转换

```rust
// ServiceSpec → ServiceSpecMeta
impl From<ServiceSpec> for ServiceSpecMeta {
    fn from(spec: ServiceSpec) -> Self {
        Self {
            name: spec.name,
            description: spec.description,
            fingerprint: spec.fingerprint,
            protobufs: spec
                .protobufs
                .into_iter()
                .map(|proto| ProtoFileMeta {
                    path: format!("{}.proto", proto.package),
                    fingerprint: proto.fingerprint,
                })
                .collect(),
            published_at: spec.published_at,
            tags: spec.tags,
        }
    }
}

// ServiceSpecMeta → ServiceSpec
impl From<ServiceSpecMeta> for ServiceSpec {
    fn from(meta: ServiceSpecMeta) -> Self {
        Self {
            name: meta.name,
            description: meta.description,
            fingerprint: meta.fingerprint,
            protobufs: meta
                .protobufs
                .into_iter()
                .map(|proto| service_spec::Protobuf {
                    package: package_from_path(&proto.path),
                    content: String::new(), // Content is no longer in lock file
                    fingerprint: proto.fingerprint,
                })
                .collect(),
            published_at: meta.published_at,
            tags: meta.tags,
        }
    }
}

fn package_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| path.trim_end_matches(".proto").to_string())
}

## 移除的代码

以下组件已移除或简化：

- ❌ `ProtoFileWithContent`（改用 `ProtoFileMeta`，无 content 字段）
- ❌ 所有 proto 内容序列化逻辑

## 文件大小分析

### 实际案例

假设一个项目有 10 个服务依赖：

```
每个服务：
- 5 个 proto 文件
- 每个文件 5KB
- 总计: 5KB × 5 = 25KB

10 个服务：
- Proto 目录总计: 25KB × 10 = 250KB
- Lock 文件总计: ~5KB (只有路径引用)
```

**结论**：250KB 的 proto 文件在现代开发环境中完全可以接受。

## 版本控制优势

### Git Diff 示例

```diff
  [[dependency.files]]
  path = "user-service/user.v1.proto"
- fingerprint = "semantic:xyz123"
+ fingerprint = "semantic:xyz456"
```

**优势**：
- 直接看到 proto 指纹变化
- 易于追踪服务版本变更
- 历史追溯清晰

## 与其他工具的对比

| 特性          | cargo   | npm     | actr (引用模式) |
| ------------- | ------- | ------- | -------------- |
| 包大小        | 几百 MB | 几十 MB | 几 KB (引用)    |
| 需要 cache    | ✓       | ✓       | ✗              |
| lock 文件大小 | ~100KB  | ~500KB  | ~5KB           |
| 包含内容      | ✗       | ✗       | ✗ (路径引用)    |
| 版本控制友好  | 一般    | 一般    | 优秀           |
| 复杂度        | 高      | 高      | 低             |

## 使用示例

```rust
use actr_config::lock::*;

// 创建 lock 文件
let mut lock_file = LockFile::new();

// 添加依赖
let spec_meta = ServiceSpecMeta {
    name: "user-service".to_string(),
    description: Some("User service".to_string()),
    fingerprint: "service_semantic:abc123".to_string(),
    protobufs: vec![
        ProtoFileMeta {
            path: "user-service/user.v1.proto".to_string(),
            fingerprint: "semantic:xyz".to_string(),
        }
    ],
    published_at: Some(1705315800),
    tags: vec!["latest".to_string()],
};

let dep = LockedDependency::new(
    "acme+user-service".to_string(),
    spec_meta,
);

lock_file.add_dependency(dep);

// 保存
lock_file.save_to_file("actr.lock.toml")?;

// 加载
let restored = LockFile::from_file("actr.lock.toml")?;

// 获取依赖
if let Some(locked_dep) = lock_file.get_dependency("user-service") {
    println!("Service: {}", locked_dep.name);
    println!("Type: {}", locked_dep.actr_type);
    println!("Fingerprint: {}", locked_dep.fingerprint);

    for file in &locked_dep.files {
        println!("Proto: {} (fingerprint: {})", file.path, file.fingerprint);
    }
}
```

## 总结

这个设计充分考虑了 Actor-RTC 的实际使用场景：

1. **Proto 文件小** → 独立文件管理
2. **需要版本控制** → 指纹追踪变更
3. **追求简洁** → 减少架构复杂度
4. **引用模式** → lock 文件 + proto 目录分离

结果是一个更简单、更直观、更易维护的设计。
