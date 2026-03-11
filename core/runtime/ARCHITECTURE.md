# actr-runtime 内部架构

**文档目的**：为贡献者和高级用户提供 runtime crate 的内部架构视图

**最后更新**：2025-11-07
**对应版本**：actr v0.9.x

---

## 1. 模块概览

`actr-runtime` crate 实现了 Actor 运行时的核心功能，包括生命周期管理、消息路由、传输层抽象、持久化等。

```
actr-runtime/
├── lifecycle/          # Actor 生命周期管理
│   ├── actr_system.rs  # ActrSystem（基础设施）
│   └── actr_node.rs    # ActrNode（完整节点）
├── inbound/            # 入站消息处理
│   ├── data_stream_registry.rs     # DataStream 快车道注册表
│   ├── media_frame_registry.rs     # MediaFrame 快车道注册表
│   └── inbound_packet_dispatcher.rs # 入站消息分发器
├── outbound/           # 出站消息处理
│   ├── inproc_out_gate.rs   # 进程内出站门
│   └── outproc_out_gate.rs  # 跨进程出站门
├── transport/          # 传输层抽象
│   ├── lane.rs              # DataLane 统一抽象
│   ├── route_table.rs       # PayloadType 路由表
│   ├── manager.rs           # TransportManager trait
│   ├── inproc_manager.rs    # 进程内传输管理
│   ├── dest_transport.rs    # 目标传输抽象
│   ├── wire_pool.rs         # Wire 连接池
│   └── wire_handle.rs       # Wire 句柄
├── wire/               # 底层传输协议
│   ├── webrtc/              # WebRTC 实现
│   │   ├── coordinator.rs   # WebRTC 协调器
│   │   ├── gate.rs          # WebRTC 门（入站）
│   │   ├── connection.rs    # WebRTC 连接
│   │   ├── negotiator.rs    # SDP 协商器
│   │   └── signaling.rs     # 信令客户端
│   ├── websocket/           # WebSocket 实现
│   │   └── connection.rs    # WebSocket 连接
│   └── mpsc.rs              # 进程内 mpsc 实现
├── context.rs          # Context 实现
├── context_factory.rs  # Context 工厂
├── actr_ref.rs         # ActrRef（Actor 引用）
└── error.rs            # 错误类型定义
```

---

## 2. 分层架构

runtime 采用 4 层架构设计：

```
┌─────────────────────────────────────────┐
│  Layer 3: Application (Workload)        │  用户业务逻辑
├─────────────────────────────────────────┤
│  Layer 2: Outbound/Inbound              │  出入站门抽象
│    - OutGate (InprocOut/OutprocOut)     │
│    - WebRtcGate / Inproc 接收循环        │
├─────────────────────────────────────────┤
│  Layer 1: Transport (DataLane)          │  传输通道抽象
│    - DataLane (Mpsc/WebRtcDataChannel)  │
│    - TransportManager                   │
│    - WirePool                           │
├─────────────────────────────────────────┤
│  Layer 0: Wire (Protocol)               │  物理传输协议
│    - WebRTC (DataChannel + RTP)         │
│    - WebSocket                          │
│    - tokio::sync::mpsc                  │
└─────────────────────────────────────────┘
```

### 关键设计原则

1. **层次分离**：每层只依赖下一层，不跨层调用
2. **统一抽象**：通过 DataLane 统一 Inproc 和 Outproc 路径
3. **PayloadType 路由**：编译时确定消息路由策略
4. **零成本抽象**：Inproc 路径零序列化，Outproc 路径零拷贝

---

## 3. 核心组件职责

### 3.1 生命周期管理（lifecycle/）

**ActrSystem**：
- 职责：提供无泛型的基础设施（Mailbox、SignalingClient、ContextFactory）
- 生命周期：从创建到 attach(workload) 转换为 ActrNode
- 关键方法：`new()`, `attach<W>()`

**ActrNode<W>**：
- 职责：泛型化的完整节点，持有 Workload 和运行时组件
- 生命周期：从 attach 到 startup() 启动
- 关键方法：`startup()`, `handle_incoming()`, `shutdown()`

### 3.2 入站处理（inbound/）

**WebRtcGate**：
- 职责：消费 `WebRtcCoordinator` 聚合的入站数据，直接根据 PayloadType 分发
- 路由规则：
  - RpcReliable/RpcSignal → Mailbox（按优先级入队，同时处理 pending_requests）
  - StreamReliable/StreamLatencyFirst → DataStreamRegistry（快车道回调）
  - MediaRtp → 直接丢弃并提示应走 WebRTC Track（MediaFrameRegistry 由 PeerConnection 注册）

**Inproc 接收循环**：
- 职责：`ActrNode` 内的两个 tokio 循环（Shell→Workload、Workload→Shell）直接从 `InprocTransportManager` 的 `DataLane::Mpsc` 收包
- Shell→Workload：解出 `RpcEnvelope` 后调用 `handle_incoming()`
- Workload→Shell：根据 `request_id` 调用 `complete_response()` 唤醒请求方

**DataStreamRegistry**：
- 职责：管理 DataStream 回调注册表（stream_id → callback）
- 并发安全：使用 DashMap 支持多线程并发访问
- 回调签名：`FnMut(DataStream, ActrId) -> BoxFuture<ActorResult<()>>`

**MediaFrameRegistry**：
- 职责：管理 MediaTrack 回调注册表（track_id → callback）
- 并发安全：使用 DashMap
- 回调签名：`FnMut(MediaSample, ActrId) -> BoxFuture<ActorResult<()>>`

### 3.3 出站处理（outbound/）

**OutGate** enum：
- `InprocOut(Arc<InprocOutGate>)`：进程内出站
- `OutprocOut(Arc<OutprocOutGate>)`：跨进程出站
- 设计优势：静态分发，零虚拟调用开销

**InprocOutGate**：
- 职责：通过 InprocTransportManager 发送进程内消息
- 特点：零序列化，直接传递 RpcEnvelope 对象
- 延迟：~10μs

**OutprocOutGate**：
- 职责：通过 OutprocTransportManager 发送跨进程消息
- 特点：Protobuf 序列化，通过 WebRTC/WebSocket 传输
- 延迟：1-50ms（取决于网络）
- pending_requests：管理 RPC 请求-响应匹配

### 3.4 传输层（transport/）

**DataLane** enum：
- `Mpsc { payload_type, tx, rx }`：进程内 tokio mpsc 通道
- `WebRtcDataChannel { data_channel, rx }`：WebRTC DataChannel
- `WebSocket { sink, payload_type, rx }`：WebSocket 连接

**PayloadTypeExt** trait：
- 核心方法：`data_lane_types() -> &'static [DataLaneType]`
- 作用：提供 PayloadType 到 DataLaneType 的静态路由表
- 优势：编译时确定，零运行时开销

**TransportManager** trait：
- 职责：管理传输通道的生命周期（创建、缓存、复用）
- 实现：
  - `InprocTransportManager`：管理进程内 mpsc 通道
  - `OutprocTransportManager`：管理 WebRTC/WebSocket 连接

### 3.5 Wire 层（wire/）

**WebRtcCoordinator**：
- 职责：管理所有 WebRTC peer connections 的生命周期
- 关键功能：
  - 启动多 PayloadType 接收循环（RpcReliable, RpcSignal, StreamReliable, StreamLatencyFirst）
  - 聚合所有 peer 的消息到统一的 message_rx
  - 提供 `send_message()` 和 `receive_message()` 接口

**WebRtcGate**：
- 职责：WebRTC 入站消息路由（Coordinator → Mailbox/Registry）
- 路由逻辑：
  - 根据 PayloadType 分发消息
  - RPC 消息检查 pending_requests（响应匹配）
  - DataStream 消息直接派发到 DataStreamRegistry

**WebRtcConnection**：
- 职责：封装单个 RTCPeerConnection，管理 DataChannel 和 MediaTrack
- 关键方法：`create_data_channel()`, `get_lane()`, `add_track()`

---

## 4. 关键数据流

### 4.1 RPC 请求-响应流程

**发送端（OutprocOutGate）**：
```rust
1. send_request(target, envelope)
2. 生成 request_id，注册 oneshot::Sender 到 pending_requests
3. 序列化 RpcEnvelope → Bytes
4. TransportManager → DataLane → WebRTC
```

**接收端（WebRtcGate）**：
```rust
1. Coordinator.receive_message() → (from, data, RpcReliable)
2. 反序列化 Bytes → RpcEnvelope
3. 检查 request_id 是否在 pending_requests 中
4. 如果是响应：唤醒 oneshot::Sender
5. 如果是请求：enqueue(Mailbox)
```

### 4.2 DataStream 快车道流程

**发送端**：
```rust
1. ctx.send_data_stream(target, stream_id, chunk)
2. OutGate::send_stream_data(target, StreamReliable, data)
3. TransportManager → DataLane(StreamReliable) → WebRTC
```

**接收端**：
```rust
1. Coordinator.receive_message() → (from, data, StreamReliable)
2. WebRtcGate 识别 PayloadType::StreamReliable
3. 反序列化 Bytes → DataStream
4. DataStreamRegistry.dispatch(chunk, sender_id)
5. 调用注册的回调函数
```

---

## 5. 性能优化设计

### 5.1 零拷贝设计

- **Inproc 路径**：直接传递 `RpcEnvelope` 对象，无序列化
- **Outproc 路径**：使用 `Bytes` 类型（Arc<Vec<u8>>），浅拷贝
- **MediaTrack**：WebRTC 原生 RTP 通道，绕过 Protobuf 序列化

### 5.2 编译时路由

- **PayloadTypeExt**：路由表在编译时确定，无运行时查表
- **MessageDispatcher**：静态分发，零 dyn trait object 开销
- **OutGate enum**：静态分发，优于 trait object

### 5.3 细粒度并发

- **DashMap**：用于 Registry，支持高并发读写
- **独立接收循环**：每个 PayloadType 独立 tokio 任务
- **无锁设计**：尽可能使用 mpsc/oneshot 避免锁竞争

---

## 6. 错误处理策略

### 6.1 错误类型层次

```rust
RuntimeError
├── TransportError      # 传输层错误（连接断开、超时）
├── ProtocolError       # 协议错误（反序列化失败）
└── Other(anyhow::Error)  # 其他错误
```

### 6.2 错误传播

- **传输层错误**：自动重试（带 exponential backoff）
- **协议错误**：记录日志，丢弃消息
- **应用层错误**：通过 RpcEnvelope.error 返回给调用方

---

## 7. 测试策略

### 7.1 单元测试

- `transport/lane.rs`：DataLane 创建和收发测试
- `inbound/data_stream_registry.rs`：回调注册和触发测试
- `outbound/inproc_out_gate.rs`：进程内消息发送测试

### 7.2 集成测试

- `tests/local_echo_integration_test.rs`：Inproc RPC 完整流程
- `examples/data-stream/`：Outproc DataStream 端到端测试
- `examples/media-relay/`：MediaTrack 端到端测试

---

## 8. 依赖图

```
actr-runtime
├── actr-framework     (trait 定义)
├── actr-protocol      (协议定义)
├── actr-mailbox       (持久化 Mailbox)
├── tokio              (异步运行时)
├── webrtc             (WebRTC 实现)
├── tokio-tungstenite  (WebSocket 实现)
├── dashmap            (并发哈希表)
└── anyhow             (错误处理)
```

---

## 9. 贡献指南

### 9.1 代码组织原则

1. **单一职责**：每个模块只负责一个清晰的功能
2. **依赖倒置**：高层模块依赖抽象（trait），不依赖具体实现
3. **开闭原则**：通过 enum 和 trait 扩展功能，而非修改现有代码

### 9.2 命名约定

- **Manager**：管理生命周期的组件（如 TransportManager）
- **Registry**：管理回调注册的组件（如 DataStreamRegistry）
- **Gate**：消息出入口抽象（如 WebRtcGate, OutGate）
- **Coordinator**：协调多个相关组件的组件（如 WebRtcCoordinator）

### 9.3 提交 PR 前检查清单

- [ ] 单元测试通过
- [ ] 集成测试通过
- [ ] 更新相关文档（README, ARCHITECTURE.md）
- [ ] 代码符合 rustfmt 和 clippy 规范
- [ ] 性能敏感路径无明显回归

---

## 10. 参考资料

- [用户文档：Runtime 设计](../../actor-rtc.github.io/zh-hans/appendix-runtime-design.zh.md)
- [用户文档：术语表](../../actor-rtc.github.io/zh-hans/appendix-glossary.zh.md)
- [用户文档：Lane 选择策略](../../actor-rtc.github.io/zh-hans/appendix-lane-selection-strategy.zh.md)
- [actr-protocol README](../protocol/README.md)
- [actr-framework README](../framework/README.md)

---

**维护者**：actr 核心团队
**问题反馈**：https://github.com/actor-rtc/actr/issues
