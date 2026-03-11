# actr-mailbox

🗄️ **Actor-RTC 持久化邮箱层**

`actr-mailbox` 提供 Actor-RTC 框架的消息持久化功能，基于 SQLite 实现，确保消息的可靠存储和检索。

## 📦 功能特性

- **消息持久化**: 可靠的消息队列和邮箱存储，基于 SQLite。
- **优先级队列**: 支持高、普通两种优先级的消息。
- **ACID 保证**: 利用 SQLite 事务确保消息操作的原子性。

## 🚀 快速开始

在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
actr-mailbox = { path = "path/to/actr-mailbox" }
```

### 基本使用

```rust,no_run
use actr_mailbox::prelude::*;
use std::time::Duration;

async fn message_processor(mailbox: impl Mailbox) {
    loop {
        // 1. 从队列中获取一批消息
        //    此操作会自动处理优先级，并将消息标记为“处理中”
        match mailbox.dequeue().await {
            Ok(messages) => {
                if messages.is_empty() {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }

                println!("处理 {} 条消息...", messages.len());
                for msg in messages {
                    // 2. 在这里执行你的业务逻辑
                    //    例如: process_data(msg.payload).await;
                    
                    // 3. 成功处理后，确认消息，将其从队列永久删除
                    if let Err(e) = mailbox.ack(msg.id).await {
                        eprintln!("消息 {} 确认失败: {}", msg.id, e);
                    }
                }
            }
            Err(e) => {
                eprintln!("从队列拉取消息失败: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
```

## 🏗️ 架构层次

```text
┌─────────────────────────────────────────┐
│              actr-core                  │
├─────────────────────────────────────────┤
│  actr-sdk  │  actr-mailbox │ actr-runtime│ (此层)
├─────────────────────────────────────────┤
│  actr-protocol │    actr-version         │
├─────────────────────────────────────────┤
│  actr-types    │     actr-uri           │
└─────────────────────────────────────────┘
```

## 📄 许可证

MIT License
