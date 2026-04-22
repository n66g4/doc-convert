/// 插件路由决策引擎（架构 §7，FR-014）
use crate::infra::AppError;
use crate::plugin_host::{PluginMeta, PluginRegistry};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteStep {
    pub plugin_id: String,
    pub in_format: String,
    pub out_format: String,
    pub step_index: usize,
}

/// `resolve` 的完整结果：多跳见 `steps`；单跳且存在多个候选时，`single_hop_fallback_ids` 为除首选外按 priority 降序的备用插件（用于自动重试）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedRoute {
    pub steps: Vec<RouteStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub single_hop_fallback_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutesConfig {
    #[serde(default)]
    pub recipes: Vec<Recipe>,
    #[serde(default)]
    pub defaults: Vec<DefaultRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub input: String,
    pub output: String,
    pub steps: Vec<RecipeStep>,
    #[serde(default)]
    pub recipe_priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeStep {
    pub plugin_id: String,
    pub out_format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultRoute {
    pub input: String,
    pub output: String,
    pub prefer_plugin_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferredPlugins {
    #[serde(default = "default_mode")]
    pub mode: String,
    pub plugins: Vec<String>,
}

fn default_mode() -> String {
    "single".to_string()
}

impl PreferredPlugins {
    pub fn parse(json: &str) -> Result<Self, AppError> {
        // 支持简化形式 ["plugin_id"]
        if let Ok(ids) = serde_json::from_str::<Vec<String>>(json) {
            return Ok(PreferredPlugins {
                mode: if ids.len() == 1 {
                    "single".to_string()
                } else {
                    "chain".to_string()
                },
                plugins: ids,
            });
        }
        serde_json::from_str(json).map_err(|e| AppError::InvalidOptions {
            message: format!("Invalid preferred_plugins: {}", e),
        })
    }
}

pub struct Router {
    routes_config: RoutesConfig,
}

impl Router {
    pub fn new(routes_config: RoutesConfig) -> Self {
        Self { routes_config }
    }

    /// 主路由决策管线（架构 §7.4）
    pub fn resolve(
        &self,
        input_format: &str,
        output_format: &str,
        preferred_plugins: Option<&str>,
        registry: &PluginRegistry,
    ) -> Result<ResolvedRoute, AppError> {
        let input = normalize_format(input_format);
        let output = normalize_format(output_format);

        // 步骤 2：preferred_plugins 非空 → 验证链
        if let Some(pp_json) = preferred_plugins {
            if !pp_json.trim().is_empty() {
                let pp = PreferredPlugins::parse(pp_json)?;
                let steps = self.resolve_preferred(&input, &output, &pp, registry)?;
                return Ok(ResolvedRoute {
                    steps,
                    single_hop_fallback_ids: vec![],
                });
            }
        }

        // 步骤 3b：单跳集合
        let single_hop: Vec<&PluginMeta> = registry
            .enabled_plugins()
            .filter(|p| {
                p.capabilities.inputs.contains(&input) && p.capabilities.outputs.contains(&output)
            })
            .collect();

        match single_hop.len() {
            1 => {
                let p = single_hop[0];
                tracing::info!(
                    input = %input, output = %output,
                    plugin = %p.id, "Route: single-hop"
                );
                Ok(ResolvedRoute {
                    steps: vec![RouteStep {
                        plugin_id: p.id.clone(),
                        in_format: input,
                        out_format: output,
                        step_index: 0,
                    }],
                    single_hop_fallback_ids: vec![],
                })
            }
            0 => {
                // 步骤 3c：匹配 recipe
                let steps = self.resolve_recipe(&input, &output, registry)?;
                Ok(ResolvedRoute {
                    steps,
                    single_hop_fallback_ids: vec![],
                })
            }
            _ => {
                // 多插件单跳平局 → 查 defaults
                let candidates: Vec<String> = single_hop.iter().map(|p| p.id.clone()).collect();
                if let Some(def) = self.find_default(&input, &output) {
                    let plugin_id = &def.prefer_plugin_id;
                    if registry
                        .get_plugin(plugin_id)
                        .map(|p| p.enabled)
                        .unwrap_or(false)
                    {
                        let fallbacks = ordered_single_hop_fallbacks(&single_hop, plugin_id);
                        tracing::info!(
                            input = %input, output = %output,
                            plugin = %plugin_id,
                            fallbacks = ?fallbacks,
                            "Route: defaults resolved tie (with fallbacks)"
                        );
                        return Ok(ResolvedRoute {
                            steps: vec![RouteStep {
                                plugin_id: plugin_id.clone(),
                                in_format: input,
                                out_format: output,
                                step_index: 0,
                            }],
                            single_hop_fallback_ids: fallbacks,
                        });
                    } else {
                        tracing::warn!(
                            plugin_id = %plugin_id,
                            "defaults.prefer_plugin_id not found or disabled, falling back to NO_ROUTE"
                        );
                    }
                }
                tracing::info!(
                    input = %input, output = %output,
                    candidates = ?candidates, "Route: NO_ROUTE (tie, use preferred_plugins)"
                );
                Err(AppError::NoRoute {
                    input,
                    output,
                    candidates,
                })
            }
        }
    }

    fn resolve_preferred(
        &self,
        input: &str,
        output: &str,
        pp: &PreferredPlugins,
        registry: &PluginRegistry,
    ) -> Result<Vec<RouteStep>, AppError> {
        if pp.plugins.is_empty() {
            return Err(AppError::InvalidOptions {
                message: "preferred_plugins.plugins is empty".into(),
            });
        }

        match pp.mode.as_str() {
            "single" => {
                let plugin_id = &pp.plugins[0];
                let meta =
                    registry
                        .get_plugin(plugin_id)
                        .ok_or_else(|| AppError::PluginNotFound {
                            plugin_id: plugin_id.clone(),
                        })?;
                if !meta.capabilities.inputs.contains(input)
                    || !meta.capabilities.outputs.contains(output)
                {
                    return Err(AppError::InvalidOptions {
                        message: format!(
                            "Plugin {} does not support {}->{}",
                            plugin_id, input, output
                        ),
                    });
                }
                Ok(vec![RouteStep {
                    plugin_id: plugin_id.clone(),
                    in_format: input.to_string(),
                    out_format: output.to_string(),
                    step_index: 0,
                }])
            }
            "chain" => {
                let mut steps = Vec::new();
                let mut current_format = input.to_string();
                for (i, plugin_id) in pp.plugins.iter().enumerate() {
                    let meta =
                        registry
                            .get_plugin(plugin_id)
                            .ok_or_else(|| AppError::PluginNotFound {
                                plugin_id: plugin_id.clone(),
                            })?;
                    let out_fmt = if i == pp.plugins.len() - 1 {
                        output.to_string()
                    } else {
                        // 找下一个插件支持的中间格式交集
                        let next_id = &pp.plugins[i + 1];
                        let next_meta = registry.get_plugin(next_id).ok_or_else(|| {
                            AppError::PluginNotFound {
                                plugin_id: next_id.clone(),
                            }
                        })?;
                        meta.capabilities
                            .outputs
                            .iter()
                            .find(|o| next_meta.capabilities.inputs.contains(*o))
                            .cloned()
                            .ok_or_else(|| AppError::InvalidOptions {
                                message: format!(
                                    "No compatible intermediate format between {} and {}",
                                    plugin_id, next_id
                                ),
                            })?
                    };
                    if !meta.capabilities.inputs.contains(&current_format) {
                        return Err(AppError::InvalidOptions {
                            message: format!(
                                "Plugin {} does not accept {} at step {}",
                                plugin_id, current_format, i
                            ),
                        });
                    }
                    steps.push(RouteStep {
                        plugin_id: plugin_id.clone(),
                        in_format: current_format.clone(),
                        out_format: out_fmt.clone(),
                        step_index: i,
                    });
                    current_format = out_fmt;
                }
                Ok(steps)
            }
            mode => Err(AppError::InvalidOptions {
                message: format!("Unknown preferred_plugins.mode: {}", mode),
            }),
        }
    }

    fn resolve_recipe(
        &self,
        input: &str,
        output: &str,
        registry: &PluginRegistry,
    ) -> Result<Vec<RouteStep>, AppError> {
        let mut matching: Vec<&Recipe> = self
            .routes_config
            .recipes
            .iter()
            .filter(|r| r.input == input && r.output == output)
            .collect();

        // 按 recipe_priority 降序，再按 id 字典序
        matching.sort_by(|a, b| {
            b.recipe_priority
                .cmp(&a.recipe_priority)
                .then(a.id.cmp(&b.id))
        });

        for recipe in matching {
            if let Ok(steps) = self.expand_recipe(recipe, input, output, registry) {
                tracing::info!(
                    input = %input, output = %output,
                    recipe_id = %recipe.id, "Route: recipe resolved"
                );
                return Ok(steps);
            }
        }

        Err(AppError::NoRoute {
            input: input.to_string(),
            output: output.to_string(),
            candidates: vec![],
        })
    }

    fn expand_recipe(
        &self,
        recipe: &Recipe,
        input: &str,
        output: &str,
        registry: &PluginRegistry,
    ) -> Result<Vec<RouteStep>, AppError> {
        let mut steps = Vec::new();
        let mut current_format = input.to_string();

        for (i, step) in recipe.steps.iter().enumerate() {
            let meta =
                registry
                    .get_plugin(&step.plugin_id)
                    .ok_or_else(|| AppError::PluginNotFound {
                        plugin_id: step.plugin_id.clone(),
                    })?;
            if !meta.enabled {
                return Err(AppError::PluginNotFound {
                    plugin_id: step.plugin_id.clone(),
                });
            }
            let out_fmt = if i == recipe.steps.len() - 1 {
                output.to_string()
            } else {
                step.out_format.clone()
            };
            if !meta.capabilities.inputs.contains(&current_format) {
                return Err(AppError::InvalidOptions {
                    message: format!(
                        "Recipe step {}: plugin {} does not accept {}",
                        i, step.plugin_id, current_format
                    ),
                });
            }
            steps.push(RouteStep {
                plugin_id: step.plugin_id.clone(),
                in_format: current_format.clone(),
                out_format: out_fmt.clone(),
                step_index: i,
            });
            current_format = out_fmt;
        }
        Ok(steps)
    }

    fn find_default(&self, input: &str, output: &str) -> Option<&DefaultRoute> {
        self.routes_config
            .defaults
            .iter()
            .find(|d| d.input == input && d.output == output)
    }
}

/// 单跳平局时，除首选插件外按 `capabilities.priority` 降序排列的备用 id（同 priority 时按 id 字典序）。
fn ordered_single_hop_fallbacks(single_hop: &[&PluginMeta], primary_id: &str) -> Vec<String> {
    let mut rest: Vec<&PluginMeta> = single_hop
        .iter()
        .copied()
        .filter(|p| p.id != primary_id)
        .collect();
    rest.sort_by(|a, b| {
        b.capabilities
            .priority
            .cmp(&a.capabilities.priority)
            .then(a.id.cmp(&b.id))
    });
    rest.into_iter().map(|p| p.id.clone()).collect()
}

/// 格式归一化（扩展名/MIME → canonical id）
pub fn normalize_format(fmt: &str) -> String {
    let s = fmt.to_lowercase();
    let s = s.trim_start_matches('.');
    match s {
        "md" | "markdown" | "gfm" => "markdown",
        "docx" | "word" => "docx",
        "xlsx" | "excel" => "xlsx",
        "pptx" | "powerpoint" => "pptx",
        "txt" | "plain" | "text" => "plain",
        "htm" | "html" => "html",
        "pdf" => "pdf",
        "rtf" => "rtf",
        "png" | "jpg" | "jpeg" | "tiff" | "tif" => s,
        "json" => "json",
        "xml" => "xml",
        "latex" | "tex" => "latex",
        "rst" => "rst",
        _ => s,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::normalize_format;

    #[test]
    fn normalize_markdown_aliases() {
        assert_eq!(normalize_format("MD"), "markdown");
        assert_eq!(normalize_format("gfm"), "markdown");
    }

    #[test]
    fn normalize_office_and_strips_dot() {
        assert_eq!(normalize_format(".docx"), "docx");
        assert_eq!(normalize_format(".doc"), "doc");
        assert_eq!(normalize_format("PDF"), "pdf");
    }

    #[test]
    fn normalize_unknown_passes_lowercase() {
        assert_eq!(normalize_format("weird"), "weird");
    }
}
