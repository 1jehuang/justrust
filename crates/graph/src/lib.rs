//! Execution graph validation and CLI reporting for the justrust client.
#![cfg(any(target_os = "linux", target_os = "macos", windows))]

mod execution_graph;
mod execution_graph_compiler;
mod execution_graph_compiler_cli;
mod execution_graph_continuation;
mod execution_graph_flow;
mod execution_graph_manifest;
mod execution_graph_query;
mod execution_graph_readiness;
mod execution_graph_reads;
mod execution_graph_resource;
mod execution_graph_resource_attachment;
mod execution_graph_task_bindings;
mod execution_graph_task_dag;
mod execution_graph_timing;
pub use execution_graph::command_with_latch_readiness_dag;
pub use execution_graph_compiler_cli::command as compiler_command;

#[cfg(test)]
mod paths {
    pub fn home() -> anyhow::Result<std::path::PathBuf> {
        use anyhow::Context;
        if let Some(home) = std::env::var_os("JUSTRUST_HOME") {
            return Ok(home.into());
        }
        Ok(dirs::home_dir()
            .context("no home directory")?
            .join(".justrust"))
    }
}
