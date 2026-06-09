# DChunked

Linux 下的分块并发下载工具，支持 SOCKS5 代理池和断点续传。

像 curl 一样直接用——编译成单个二进制文件，跟上参数即可下载。

## 功能特性

- **分块并发下载** — 将文件按指定大小切分为多个块，通过 HTTP Range 头并发下载
- **固定线程池** — 指定并发工作线程数，每个线程从共享队列中取块下载，避免资源耗尽
- **SOCKS5h 代理池** — 支持单个代理或从 TOML 配置文件加载代理池，自动轮换分配
- **故障自动转移** — 代理失败自动标记，租用新代理重试；连续失败超过阈值自动跳过并重置
- **断点续传** — 下载中断后重新运行，自动跳过已完成的块，未完成的块从断点续传
- **实时进度条** — 显示总进度、各工作线程进度、下载速度和预计剩余时间
- **零外部依赖** — 使用 rustls 而非 OpenSSL，编译产物无需安装系统级 SSL 库

## 快速开始

### 从源码编译

需要 Rust 1.70+ 和 Cargo。

```bash
git clone https://github.com/bevin1984/dchunked.git
cd dchunked
cargo build --release
```

编译产物位于 `target/release/dchunked`，拷贝到 `/usr/local/bin/` 即可全局使用：

```bash
sudo cp target/release/dchunked /usr/local/bin/
```

### 打包为 rpm/deb

**RPM (CentOS/RHEL)：**

```bash
cargo install cargo-generate-rpm
cargo build --release
strip target/release/dchunked
cargo generate-rpm
# 产物在 target/generate-rpm/*.rpm
sudo rpm -i target/generate-rpm/*.rpm
```

**DEB (Ubuntu/Debian)：**

```bash
cargo install cargo-deb
cargo deb
# 产物在 target/debian/*.deb
sudo dpkg -i target/debian/*.deb
```

### 从 rpm/deb 包安装

从 [Releases](https://github.com/bevin1984/dchunked/releases) 页面下载对应包后安装：

**RPM (CentOS/RHEL)：**

```bash
sudo rpm -i dchunked-*.x86_64.rpm
```

**DEB (Ubuntu/Debian)：**

```bash
sudo dpkg -i dchunked_*_amd64.deb
```

安装完成后即可全局使用 `dchunked` 命令。卸载方式：

```bash
# RPM
sudo rpm -e dchunked

# DEB
sudo dpkg -r dchunked
```

## 使用说明

### 基本用法

```bash
# 最简用法，10MB 一块，8 个工作线程
dchunked https://example.com/bigfile.zip

# 指定输出文件名、块大小和并发线程数
dchunked https://example.com/bigfile.zip -o movie.mp4 -b 5M -c 16
```

### 工作原理

文件按 `-b` 指定的块大小切分为多个块。`-c` 指定并发工作线程数，每个线程从共享队列中取一个块下载，完成后自动取下一个未下载的块，直到所有块下载完毕。

例如，下载一个 1GB 的文件，`-b 10M -c 4`：
- 文件被切分为约 100 个 10MB 的块
- 同时只有 4 个线程在下载，每个线程下载完一个块后继续取下一个
- 不会出现 100 个线程同时运行的情况

### 断点续传

下载过程中断（Ctrl+C 或网络故障），重新运行相同的命令即可续传：

```bash
# 首次下载（中断）
dchunked https://example.com/bigfile.zip -o movie.mp4

# 重新运行相同命令，自动续传
dchunked https://example.com/bigfile.zip -o movie.mp4
```

已下载完成的块会自动跳过，未完成的块从断点继续。下载状态保存在输出文件同目录下的隐藏目录 `.filename/` 中，下载完成后自动清理。

### 使用代理

```bash
# 通过单个 SOCKS5 代理下载（DNS 在代理端解析）
dchunked https://example.com/bigfile.zip -x socks5h://127.0.0.1:1080

# 带认证的代理
dchunked https://example.com/bigfile.zip -x socks5h://user:pass@proxy-host:1080
```

`socks5h://` 表示 DNS 解析在代理端完成（保护本地 DNS 隐私）。如果使用 `socks5://`，DNS 会在本地解析。

### 使用代理池

创建代理池配置文件 `proxies.toml`：

```toml
[[proxies]]
addr = "socks5h://user:pass@host1:1080"

[[proxies]]
addr = "socks5h://host2:1080"

[[proxies]]
addr = "socks5h://user:pass@host3:7890"
```

下载时指定配置文件：

```bash
dchunked https://example.com/bigfile.zip -p proxies.toml -c 12
```

代理池以 round-robin 方式将代理分配给各工作线程，失败自动切换。

### 完整参数

```
Usage: dchunked [OPTIONS] <URL>

Arguments:
  <URL>                          下载地址

Options:
  -o, --output <OUTPUT>          输出文件路径（默认从 URL 提取文件名）
  -c, --chunks <CHUNKS>          并发工作线程数 [default: 8]
  -b, --block-size <BLOCK_SIZE>  块大小（支持 K/M/G 后缀，如 10M、1G）[default: 10M]
  -x, --proxy <PROXY>            单个 SOCKS5 代理地址
  -p, --proxy-file <PROXY_FILE>  代理池配置文件路径 (TOML)
      --retry <RETRY>            每个块最大重试次数 [default: 3]
      --timeout <TIMEOUT>        连接超时时间（秒） [default: 30]
  -h, --help                     显示帮助
  -V, --version                  显示版本
```

## 项目结构

```
DChunked/
├── Cargo.toml              # 项目配置和依赖
├── config.example.toml     # 代理池配置示例
└── src/
    ├── main.rs             # 入口，编排下载流程
    ├── cli.rs              # 命令行参数定义 (clap)
    ├── config.rs           # TOML 配置文件加载
    ├── block.rs            # 块规划、调度器、清单文件、断点续传
    ├── worker.rs           # 块下载核心逻辑（重试 + 代理故障转移）
    ├── proxy.rs            # 代理池管理（轮换、故障追踪、自动恢复）
    ├── merger.rs           # 块文件合并
    ├── progress.rs         # 进度条显示 (indicatif)
    └── error.rs            # 统一错误类型定义
```

### 下载流程

```
1. 解析 CLI 参数和配置文件
2. HEAD 请求获取文件大小，检测 Range 支持
3. 按块大小计算所有块的字节范围 (bytes=start-end)
4. 检查隐藏目录，扫描已完成的块（断点续传）
5. 创建调度器，启动 N 个工作线程
   ├── 从调度器获取一个未占用的块（无锁 CAS 抢占）
   ├── 租用代理（或直连）
   ├── GET 请求 + Range 头
   ├── 流式写入隐藏目录中的块文件
   ├── 失败时标记代理，租用新代理重试
   └── 完成后标记块已完成，更新清单，继续取下一个块
6. 所有块完成后，按顺序合并到最终文件
7. 清理隐藏目录，输出完成信息
```

### 隐藏目录结构

下载过程中，输出文件同目录下会创建隐藏目录存放块数据：

```
./bigfile.zip              ← 最终输出文件
./.bigfile.zip/            ← 隐藏目录（下载完成后自动清理）
    manifest.toml          ← 下载状态清单
    0.block                ← 块数据文件
    1.block
    2.block
    ...
```

### 关键依赖

| 依赖 | 用途 |
|------|------|
| tokio | 异步运行时，并发下载 |
| reqwest | HTTP 客户端，socks feature 提供 SOCKS5 支持 |
| clap | CLI 参数解析 |
| indicatif | 终端进度条 |
| serde + toml | 代理池配置和下载清单解析 |
| thiserror / anyhow | 错误处理 |

## 开发

```bash
# 开发编译
cargo build

# 运行
cargo run -- <URL> [OPTIONS]

# 编译 release
cargo build --release
```
