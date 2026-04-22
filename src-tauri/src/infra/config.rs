use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub data_root: PathBuf,
    pub max_file_size_bytes: u64,
    pub max_concurrent_tasks: usize,
    pub task_result_ttl_secs: u64,
    pub python_executable: PathBuf,
    pub pandoc_executable: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            data_root: data_root_default(),
            // 默认可覆盖「几百 MB」级 PDF；更大请改 config.json 或 DOCCONVERT_MAX_FILE_BYTES
            max_file_size_bytes: 2 * 1024 * 1024 * 1024, // 2 GiB
            max_concurrent_tasks: 10,
            task_result_ttl_secs: 72 * 3600, // 72 hours
            python_executable: default_python(),
            pandoc_executable: default_pandoc(),
        }
    }
}

fn data_root_default() -> PathBuf {
    if let Ok(v) = std::env::var("DOCCONVERT_DATA_DIR") {
        return PathBuf::from(v);
    }
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DocConvert")
}

fn default_python() -> PathBuf {
    #[cfg(target_os = "windows")]
    return PathBuf::from("python");
    #[cfg(not(target_os = "windows"))]
    PathBuf::from("python3")
}

fn default_pandoc() -> PathBuf {
    PathBuf::from("pandoc")
}

fn is_default_python_command(p: &std::path::Path) -> bool {
    matches!(
        p.as_os_str().to_string_lossy().as_ref(),
        "python" | "python3" | "python.exe"
    )
}

/// 开发时：`doc-convert/src-tauri/resources/pandoc/pandoc`（或从 `src-tauri` 子目录启动时的相对路径）
fn dev_workspace_bundled_pandoc() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    #[cfg(target_os = "windows")]
    let name = "pandoc.exe";
    #[cfg(not(target_os = "windows"))]
    let name = "pandoc";
    for base in [cwd.join("src-tauri"), cwd.join("..").join("src-tauri")] {
        let p = base.join("resources").join("pandoc").join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// 开发时若存在仓库内 `python/.venv`，优先使用该解释器（支持从仓库根或 `src-tauri` 启动）。
fn project_venv_python() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    for base in [cwd.clone(), cwd.join("..")] {
        let venv_dir = base.join("python").join(".venv");
        if venv_dir.is_dir() {
            #[cfg(target_os = "windows")]
            {
                return Some(venv_dir.join("Scripts").join("python.exe"));
            }
            #[cfg(not(target_os = "windows"))]
            {
                return Some(venv_dir.join("bin").join("python3"));
            }
        }
    }
    None
}

/// `DOCCONVERT_MAX_FILE_BYTES`：单文件上限（字节），覆盖 `config.json` 与默认值。
fn apply_max_file_bytes_env(cfg: &mut AppConfig) {
    if let Ok(s) = std::env::var("DOCCONVERT_MAX_FILE_BYTES") {
        if let Ok(v) = s.trim().parse::<u64>() {
            if v > 0 {
                cfg.max_file_size_bytes = v;
            }
        }
    }
}

impl AppConfig {
    pub fn load_or_default(data_root: &std::path::Path) -> Self {
        let config_path = data_root.join("config.json");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(mut cfg) = serde_json::from_str::<AppConfig>(&content) {
                    cfg.apply_env_overrides();
                    return cfg;
                }
            }
        }
        let mut cfg = AppConfig {
            data_root: data_root.to_path_buf(),
            ..Default::default()
        };
        cfg.apply_env_overrides();
        cfg
    }

    /// 从磁盘加载后仍可应用环境变量覆盖（与 `load_or_default` 无文件分支一致）
    pub fn apply_env_overrides(&mut self) {
        if let Ok(p) = std::env::var("DOCCONVERT_PYTHON") {
            self.python_executable = PathBuf::from(p);
        }
        if let Ok(p) = std::env::var("DOCCONVERT_PANDOC") {
            self.pandoc_executable = PathBuf::from(p);
        }
        apply_max_file_bytes_env(self);
    }

    /// 解析 Python：`DOCCONVERT_PYTHON` > 随包 `Resources/python` > 仓库 `python/.venv` > `config.json` / 默认 `python3`。
    ///
    /// 安装包内随带已含 MarkItDown 的 venv，必须优先于 `config.json` 里曾保存的系统路径（否则易指向 Python 3.9 且无 markitdown）。
    /// 仅当**没有**随包解释器时，才保留「非默认路径」的自定义解释器。
    pub fn resolve_python_executable(&mut self, resource_root: Option<&std::path::Path>) {
        if std::env::var("DOCCONVERT_PYTHON").is_ok() {
            return;
        }
        if let Some(root) = resource_root {
            if let Some(p) = super::bundled_paths::bundled_resource_python_exe(root) {
                if p.is_file() {
                    self.python_executable = p;
                    return;
                }
            }
        }
        if !is_default_python_command(&self.python_executable) {
            return;
        }
        if let Some(v) = project_venv_python() {
            if v.exists() {
                self.python_executable = v;
            }
        }
    }

    /// 解析 Pandoc：`DOCCONVERT_PANDOC` > 随包/开发目录中的二进制 > PATH `pandoc`。
    /// 若 `config.json` 已保存非默认的 `pandoc_executable`（非裸名 `pandoc`），不覆盖。
    pub fn resolve_pandoc_executable(&mut self, bundled_from_tauri: Option<PathBuf>) {
        if std::env::var("DOCCONVERT_PANDOC").is_ok() {
            return;
        }
        if self.pandoc_executable.as_path() != std::path::Path::new("pandoc") {
            return;
        }
        if let Some(p) = bundled_from_tauri {
            if p.is_file() {
                self.pandoc_executable = p;
                return;
            }
        }
        if let Some(p) = dev_workspace_bundled_pandoc() {
            if p.is_file() {
                self.pandoc_executable = p;
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.data_root)?;
        let config_path = self.data_root.join("config.json");
        let tmp = config_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &config_path)?;
        Ok(())
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.data_root.join("temp")
    }

    pub fn tasks_dir(&self) -> PathBuf {
        self.data_root.join("tasks")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_root.join("logs")
    }

    pub fn plugins_extra_dir(&self) -> PathBuf {
        self.data_root.join("plugins_extra")
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.data_root.join("runtime")
    }

    /// RapidOCR 下载的 ONNX 等模型目录（可写）。默认在数据目录下，避免写入 `.app` 内只读/不应修改的 site-packages。
    ///
    /// 可由环境变量 `DOCCONVERT_RAPIDOCR_MODEL_DIR` 在启动 Core 时覆盖；Python 子进程由 `workers` 注入该变量。
    pub fn rapidocr_models_dir(&self) -> PathBuf {
        self.data_root.join("cache").join("rapidocr")
    }
}
