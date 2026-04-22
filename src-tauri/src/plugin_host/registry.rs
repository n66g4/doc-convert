/// 插件元数据与注册表（架构 §6.1, §6.2）
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCapabilities {
    #[serde(default)]
    pub inputs: HashSet<String>,
    #[serde(default)]
    pub outputs: HashSet<String>,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_tier: Option<String>,
}

fn default_priority() -> i32 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRuntime {
    #[serde(rename = "type")]
    pub runtime_type: String, // python | pandoc_wrapper | native
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>, // e.g. "plugin_main:run"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDependencies {
    #[serde(default)]
    pub python_packages: Vec<String>,
}

/// plugin.toml 的 [plugin] section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_api_version: Option<String>,
}

/// 完整的 plugin.toml 文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToml {
    pub plugin: PluginManifest,
    #[serde(rename = "plugin.capabilities")]
    pub capabilities: Option<PluginCapabilities>,
    #[serde(rename = "plugin.runtime")]
    pub runtime: Option<PluginRuntime>,
    #[serde(rename = "plugin.dependencies")]
    pub dependencies: Option<PluginDependencies>,
}

/// 运行时插件元数据（注册表中存储）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: String,
    pub enabled: bool,
    pub capabilities: PluginCapabilities,
    pub runtime_type: String,
    pub entry: Option<String>,
    pub plugin_dir: PathBuf,
    pub source: PluginSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_api_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PluginSource {
    Bundled,
    Extra,
}

impl PluginMeta {
    pub fn from_toml_file(path: &Path, source: PluginSource) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        // plugin.toml 使用嵌套表，需要手动解析
        let raw: toml::Value = toml::from_str(&content)?;

        let plugin = raw
            .get("plugin")
            .ok_or_else(|| anyhow::anyhow!("Missing [plugin] section"))?;

        let id = plugin
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing plugin.id"))?
            .to_string();

        let name = plugin
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();

        let version = plugin
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0")
            .to_string();

        let authors = plugin
            .get("authors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let description = plugin
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let host_api_version = plugin
            .get("host_api_version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // [plugin.capabilities]
        let caps = raw.get("plugin").and_then(|p| p.get("capabilities"));
        let capabilities = if let Some(c) = caps {
            let inputs: HashSet<String> = c
                .get("input")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let outputs: HashSet<String> = c
                .get("output")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let priority = c.get("priority").and_then(|v| v.as_integer()).unwrap_or(10) as i32;
            let quality_tier = c
                .get("quality_tier")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            PluginCapabilities {
                inputs,
                outputs,
                priority,
                quality_tier,
            }
        } else {
            PluginCapabilities {
                inputs: HashSet::new(),
                outputs: HashSet::new(),
                priority: 10,
                quality_tier: None,
            }
        };

        // [plugin.runtime]
        let rt = raw.get("plugin").and_then(|p| p.get("runtime"));
        let (runtime_type, entry) = if let Some(r) = rt {
            let rt_type = r
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("python")
                .to_string();
            let entry = r
                .get("entry")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (rt_type, entry)
        } else {
            ("python".to_string(), None)
        };

        let plugin_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

        Ok(PluginMeta {
            id,
            name,
            version,
            authors,
            description,
            enabled: true,
            capabilities,
            runtime_type,
            entry,
            plugin_dir,
            source,
            host_api_version,
        })
    }
}

/// 插件注册表（运行时内存视图）
pub struct PluginRegistry {
    plugins: std::collections::HashMap<String, PluginMeta>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: std::collections::HashMap::new(),
        }
    }

    pub fn register(&mut self, meta: PluginMeta) {
        // 内置优先：若已有同 id 的内置插件，extra 不得静默覆盖
        if let Some(existing) = self.plugins.get(&meta.id) {
            if existing.source == PluginSource::Bundled && meta.source == PluginSource::Extra {
                warn!(
                    plugin_id = %meta.id,
                    "Extra plugin attempts to override bundled — ignored (use override_bundled=true)"
                );
                return;
            }
        }
        info!(
            id = %meta.id,
            version = %meta.version,
            source = ?meta.source,
            "Plugin registered"
        );
        self.plugins.insert(meta.id.clone(), meta);
    }

    pub fn get_plugin(&self, id: &str) -> Option<&PluginMeta> {
        self.plugins.get(id)
    }

    pub fn get_plugin_mut(&mut self, id: &str) -> Option<&mut PluginMeta> {
        self.plugins.get_mut(id)
    }

    pub fn enabled_plugins(&self) -> impl Iterator<Item = &PluginMeta> {
        self.plugins.values().filter(|p| p.enabled)
    }

    pub fn all_plugins(&self) -> impl Iterator<Item = &PluginMeta> {
        self.plugins.values()
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(p) = self.plugins.get_mut(id) {
            p.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// 从目录扫描 plugin.toml（双根发现，架构 §6.1）
    pub fn discover_from_dir(&mut self, dir: &Path, source: PluginSource) {
        if !dir.exists() {
            return;
        }
        for entry in WalkDir::new(dir).max_depth(3).into_iter().flatten() {
            if entry.file_name() == "plugin.toml" {
                match PluginMeta::from_toml_file(entry.path(), source.clone()) {
                    Ok(meta) => self.register(meta),
                    Err(e) => error!(
                        path = %entry.path().display(),
                        error = %e,
                        "Failed to load plugin.toml"
                    ),
                }
            }
        }
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}
