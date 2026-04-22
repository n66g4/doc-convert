//! 随包资源与开发目录解析（安装包 resource_dir vs 仓库内路径）

use std::path::{Path, PathBuf};

/// 安装包内预留：`resource_dir/python/bin/python3`（Windows：`Scripts/python.exe` 或根目录 `python.exe`），存在则优先于仓库 venv。
pub fn bundled_resource_python_exe(resource_root: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        for p in [
            resource_root
                .join("python")
                .join("Scripts")
                .join("python.exe"),
            resource_root.join("python").join("python.exe"),
        ] {
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        for name in ["python3", "python"] {
            let p = resource_root.join("python").join("bin").join(name);
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
}

/// 内置插件目录：`resource_dir/plugins/bundled` 存在则用；否则回退到仓库 `plugins/bundled`（tauri dev / core-only）。
pub fn resolve_bundled_plugins_dir(from_tauri: Option<PathBuf>) -> PathBuf {
    if let Some(p) = from_tauri {
        if p.is_dir() {
            return p;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for base in [cwd.clone(), cwd.join("..")] {
            let p = base.join("plugins").join("bundled");
            if p.is_dir() {
                return p;
            }
        }
    }
    std::env::current_dir()
        .unwrap_or_default()
        .join("plugins")
        .join("bundled")
}

/// 内置 `routes.toml`：优先安装包 `resource_root/config/routes.toml`，其次当前工作区 `config/routes.toml`。
pub fn resolve_bundled_routes_toml(resource_root: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(r) = resource_root {
        candidates.push(r.join("config").join("routes.toml"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("config").join("routes.toml"));
        candidates.push(cwd.join("..").join("config").join("routes.toml"));
    }
    candidates.into_iter().find(|p| p.is_file())
}
