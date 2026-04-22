/// runtime/core.json — 记录 Core 进程的监听端口，供壳进程读取。
/// 实现原子写入与陈旧锁（stale lock）检测策略（架构 §9.4）。
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreLock {
    pub schema: u32,
    pub pid: u32,
    pub bind: String,
    pub port: u16,
    pub started_at_unix_ms: u64,
    pub api_base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_ms: Option<u64>,
}

impl CoreLock {
    pub fn new(port: u16) -> Self {
        let pid = std::process::id();
        let started_at = now_unix_ms();
        CoreLock {
            schema: 1,
            pid,
            bind: "127.0.0.1".to_string(),
            port,
            started_at_unix_ms: started_at,
            api_base: format!("http://127.0.0.1:{}", port),
            boot_id: Some(generate_boot_id()),
            last_heartbeat_ms: None,
        }
    }

    pub fn api_base(&self) -> &str {
        &self.api_base
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn generate_boot_id() -> String {
    // Try platform boot_id first, fall back to random UUID
    #[cfg(target_os = "linux")]
    if let Ok(id) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") {
        return id.trim().to_string();
    }
    uuid::Uuid::new_v4().to_string()
}

pub struct LockfileManager {
    path: PathBuf,
}

impl LockfileManager {
    pub fn new(runtime_dir: &Path) -> Self {
        Self {
            path: runtime_dir.join("core.json"),
        }
    }

    /// 原子写入 lockfile（先写 .tmp，再 rename）
    pub fn write(&self, lock: &CoreLock) -> anyhow::Result<()> {
        let dir = self.path.parent().unwrap();
        std::fs::create_dir_all(dir)?;
        let tmp = self.path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(lock)?;
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &self.path)?;
        info!(path = %self.path.display(), port = lock.port, "Lockfile written");
        Ok(())
    }

    /// 更新心跳时间戳
    pub fn update_heartbeat(&self) -> anyhow::Result<()> {
        if let Ok(mut lock) = self.read() {
            lock.last_heartbeat_ms = Some(now_unix_ms());
            self.write(&lock)?;
        }
        Ok(())
    }

    /// 读取 lockfile
    pub fn read(&self) -> anyhow::Result<CoreLock> {
        let content = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// 删除 lockfile
    pub fn remove(&self) {
        if self.path.exists() {
            let stale_path = self
                .path
                .with_extension(format!("json.stale.{}", now_unix_ms()));
            let _ = std::fs::rename(&self.path, &stale_path);
            info!(path = %self.path.display(), "Lockfile marked stale");
        }
    }

    /// 判断是否存在陈旧锁。
    /// 陈旧判定策略（MVP 宽松模式）：
    /// 1. HTTP GET /health 返回 2xx + 可解析 JSON → 有效
    /// 2. 否则 → 陈旧
    pub fn check_stale(&self) -> StalenessResult {
        if !self.path.exists() {
            return StalenessResult::NoLock;
        }
        let lock = match self.read() {
            Ok(l) => l,
            Err(e) => {
                warn!(error = %e, "Failed to parse lockfile, treating as stale");
                return StalenessResult::Stale;
            }
        };

        let health_url = format!("{}/health", lock.api_base);
        match check_health_sync(&health_url) {
            Ok(true) => {
                info!(pid = lock.pid, port = lock.port, "Existing Core is alive");
                StalenessResult::Alive(lock)
            }
            Ok(false) | Err(_) => {
                warn!(
                    pid = lock.pid,
                    port = lock.port,
                    "Lockfile is stale (health check failed)"
                );
                StalenessResult::Stale
            }
        }
    }
}

#[derive(Debug)]
pub enum StalenessResult {
    NoLock,
    Alive(CoreLock),
    Stale,
}

/// 同步 HTTP 探活（超时 300ms）
fn check_health_sync(url: &str) -> anyhow::Result<bool> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    // 解析 host:port
    let url = url.trim_start_matches("http://");
    let (addr, path) = url.split_once('/').unwrap_or((url, "health"));
    let path = format!("/{}", path);

    let mut stream = TcpStream::connect_timeout(&addr.parse()?, Duration::from_millis(300))?;
    stream.set_read_timeout(Some(Duration::from_millis(300)))?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, addr
    );
    stream.write_all(request.as_bytes())?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    // 检查 HTTP 200 且 body 是 JSON
    let is_ok = response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200");
    if is_ok {
        // 尝试解析 JSON body（严格模式：status == "ok"）
        if let Some(body_start) = response.find("\r\n\r\n") {
            let body = &response[body_start + 4..];
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
                return Ok(v.get("status").and_then(|s| s.as_str()) == Some("ok"));
            }
        }
    }
    Ok(false)
}
