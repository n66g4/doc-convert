pub mod registry;
pub mod smoke_test;

pub use registry::{PluginCapabilities, PluginMeta, PluginRegistry, PluginSource};
pub use smoke_test::{run_plugin_test, smoke_test_plugin, PluginSmokeTestResult, PluginTestDepth};
