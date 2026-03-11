# actr-framework Utilities

本模块提供可选的工具函数，独立于核心框架接口。

## GeoIP 地理位置查询

提供基于 MaxMind GeoLite2 数据库的 IP 地址到地理坐标转换功能。

### 快速开始

#### 1. 启用 geoip feature

在 `Cargo.toml` 中添加：

```toml
[dependencies]
actr-framework = { path = "../actr/crates/framework", features = ["geoip"] }
```

#### 2. 准备 GeoIP 数据库

**自动下载（推荐）：**

首次调用 `GeoIpService::new()` 时自动下载（需要设置环境变量）：

```bash
# 获取 License Key：https://www.maxmind.com/en/geolite2/signup
export MAXMIND_LICENSE_KEY="your-key-here"

# 首次运行时自动下载（~70MB，约 30 秒）
cargo run --features geoip
```

**手动下载（生产环境）：**

```bash
curl -o GeoLite2-City.tar.gz \
  "https://download.maxmind.com/app/geoip_download?edition_id=GeoLite2-City&license_key=YOUR_KEY&suffix=tar.gz"
tar -xzf GeoLite2-City.tar.gz --strip-components=1 -C data/geoip/ "*/GeoLite2-City.mmdb"
```

#### 3. 在 Actor 中使用

```rust
use actr_framework::util::geoip::GeoIpService;
use actr_protocol::{RegisterRequest, ServiceLocation};

// 初始化 GeoIP 服务
let geoip = GeoIpService::new("data/geoip/GeoLite2-City.mmdb")?;

// 查询本机 IP 的坐标
let my_ip = local_ip_address::local_ip()?;
let location = geoip.lookup(my_ip).map(|(lat, lon)| ServiceLocation {
    region: "auto-detected".to_string(),
    latitude: Some(lat),
    longitude: Some(lon),
});

// 在注册时提供坐标
let request = RegisterRequest {
    realm: Realm { realm_id: 1 },
    actr_type: my_type,
    geo_location: location,
    // ... 其他字段
};
```

### API 文档

#### `GeoIpService::new(db_path)`

初始化 GeoIP 服务。

**参数：**
- `db_path` - GeoLite2-City.mmdb 数据库文件路径

**返回：**
- `Result<GeoIpService>` - 成功返回服务实例，失败返回错误

**错误：**
- 数据库文件不存在
- 数据库格式错误

#### `GeoIpService::lookup(ip)`

查询 IP 地址的地理坐标。

**参数：**
- `ip: IpAddr` - 要查询的 IP 地址

**返回：**
- `Option<(f64, f64)>` - 成功返回 `(latitude, longitude)`，失败返回 `None`

**说明：**
- 精度为城市级（50-100km 误差）
- 内网 IP 通常无法查询到坐标
- 部分公网 IP 可能也不在数据库中

### 完整示例

```rust
use actr_framework::util::geoip::GeoIpService;
use actr_framework::{Workload, Context};
use actr_protocol::*;
use anyhow::Result;

pub struct MyService {
    geoip: GeoIpService,
}

impl MyService {
    pub fn new() -> Result<Self> {
        let geoip = GeoIpService::new("data/geoip/GeoLite2-City.mmdb")?;
        Ok(Self { geoip })
    }

    fn get_my_location(&self) -> Option<ServiceLocation> {
        // 获取本机 IP
        let my_ip = local_ip_address::local_ip().ok()?;

        // 查询坐标
        self.geoip.lookup(my_ip).map(|(lat, lon)| ServiceLocation {
            region: format!("auto-{}", my_ip),
            latitude: Some(lat),
            longitude: Some(lon),
        })
    }
}

#[async_trait]
impl Workload for MyService {
    type Dispatcher = MyServiceDispatcher;

    async fn on_start<C: Context>(&mut self, ctx: &C) -> ActorResult<()> {
        // 获取坐标
        let location = self.get_my_location();
        let actor_type = ctx.self_id().r#type;
        // 注册到信令服务（带坐标）
        let register_req = RegisterRequest {
            realm: Realm { realm_id: 1 },
            actr_type,
            geo_location: location,
            // ... 其他字段
        };

        // 发送注册请求
        // ctx.call(&signaling_dest, register_req).await?;

        Ok(())
    }
}
```

### 性能特性

- **数据库大小** - ~70MB（GeoLite2-City）
- **查询延迟** - 内存查询，< 1ms
- **精度** - 城市级，50-100km 误差范围
- **覆盖率** - 大部分公网 IP，内网 IP 无法查询

### 故障排查

#### 数据库加载失败

```
Error: Failed to open GeoIP database at "data/geoip/GeoLite2-City.mmdb"
```

**解决方法：**
1. 确认文件存在：`ls -lh data/geoip/GeoLite2-City.mmdb`
2. 检查文件权限：`chmod 644 data/geoip/GeoLite2-City.mmdb`
3. 设置环境变量并重新运行（自动下载）：`export MAXMIND_LICENSE_KEY="your-key" && cargo run --features geoip`
4. 或按照上述"手动下载"步骤重新下载

#### IP 地址未找到

```
debug: GeoIP lookup: 192.168.1.1 not in database
```

**说明：**
- 内网 IP（192.168.x.x, 10.x.x.x, 172.16-31.x.x）不在数据库中
- 部分公网 IP 也可能不在数据库中
- 这是正常现象，返回 `None` 即可

#### 坐标不准确

**原因：** GeoLite2 精度为城市级（50-100km 误差）

**解决方法：**
- 升级到 GeoIP2 Precision（需付费，精度可达 10km）
- 或使用配置文件提供准确坐标
- 移动设备使用 GPS

### Feature Flag

GeoIP 功能是可选的，通过 feature flag 控制：

```toml
# 默认（不包含 GeoIP）
actr-framework = { path = "..." }

# 启用 GeoIP
actr-framework = { path = "...", features = ["geoip"] }
```

禁用 `geoip` feature 时：
- `GeoIpService::new()` 返回错误
- `GeoIpService::lookup()` 总是返回 `None`
- 不依赖 `maxminddb` crate
- 减少编译时间和二进制大小

### 最佳实践

1. **初始化时机** - 在 Actor 启动时初始化 GeoIpService（不是每次查询时）
2. **错误处理** - 数据库加载失败应记录日志但不阻止 Actor 启动
3. **降级方案** - 查询失败时使用配置文件提供的坐标
4. **数据库更新** - 每月更新一次 GeoLite2 数据库（MaxMind 每周更新）

### 相关文档

- MaxMind GeoLite2: https://dev.maxmind.com/geoip/geolite2-free-geolocation-data
- 信令服务地理负载均衡: `actrix-signaling/crates/signaling/README_GEOIP.md`
