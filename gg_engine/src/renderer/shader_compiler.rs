//! Runtime GLSL → SPIR-V compilation for shader hot-reload.
//!
//! Replicates the logic from `build.rs` but returns `Result` instead of
//! panicking, so the editor can display errors and keep the old pipelines.

use std::path::Path;
use std::process::Command;

/// Compiled SPIR-V bytecode for a single shader file (vertex + fragment).
pub(crate) struct CompiledShader {
    pub vert_spv: Vec<u8>,
    pub frag_spv: Vec<u8>,
}

/// Compiled SPIR-V bytecode for a compute shader.
pub(crate) struct CompiledComputeShader {
    #[allow(dead_code)]
    // Used for validation; hot-reload of compute pipelines not yet implemented.
    pub comp_spv: Vec<u8>,
}

/// Compile a `.glsl` source file into vertex and fragment SPIR-V bytecode.
///
/// The source must contain `#type vertex` and `#type fragment` markers
/// (same format as the build-time shader pipeline).
pub(crate) fn compile_glsl(path: &Path) -> Result<CompiledShader, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read '{}': {e}", path.display()))?;

    let (vert_src, frag_src) = split_glsl_source(&source, path)?;

    let temp_dir = std::env::temp_dir().join("gg_shader_hotreload");
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("Cannot create temp dir: {e}"))?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("Invalid shader path: {}", path.display()))?;

    let vert_tmp = temp_dir.join(format!("{stem}.vert"));
    let frag_tmp = temp_dir.join(format!("{stem}.frag"));
    std::fs::write(&vert_tmp, &vert_src).map_err(|e| format!("Cannot write temp vert: {e}"))?;
    std::fs::write(&frag_tmp, &frag_src).map_err(|e| format!("Cannot write temp frag: {e}"))?;

    let vert_spv_path = temp_dir.join(format!("{stem}_vert.spv"));
    let frag_spv_path = temp_dir.join(format!("{stem}_frag.spv"));

    run_glslc(&vert_tmp, &vert_spv_path)?;
    run_glslc(&frag_tmp, &frag_spv_path)?;

    let vert_spv = std::fs::read(&vert_spv_path)
        .map_err(|e| format!("Cannot read compiled vert SPIR-V: {e}"))?;
    let frag_spv = std::fs::read(&frag_spv_path)
        .map_err(|e| format!("Cannot read compiled frag SPIR-V: {e}"))?;

    Ok(CompiledShader { vert_spv, frag_spv })
}

/// Compile a `.glsl` compute shader source file into SPIR-V bytecode.
///
/// The source must contain a `#type compute` marker.
pub(crate) fn compile_compute_glsl(path: &Path) -> Result<CompiledComputeShader, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read '{}': {e}", path.display()))?;

    let comp_src = extract_compute_source(&source, path)?;

    let temp_dir = std::env::temp_dir().join("gg_shader_hotreload");
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("Cannot create temp dir: {e}"))?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("Invalid shader path: {}", path.display()))?;

    let comp_tmp = temp_dir.join(format!("{stem}.comp"));
    std::fs::write(&comp_tmp, &comp_src).map_err(|e| format!("Cannot write temp comp: {e}"))?;

    let comp_spv_path = temp_dir.join(format!("{stem}_comp.spv"));
    run_glslc(&comp_tmp, &comp_spv_path)?;

    let comp_spv = std::fs::read(&comp_spv_path)
        .map_err(|e| format!("Cannot read compiled comp SPIR-V: {e}"))?;

    Ok(CompiledComputeShader { comp_spv })
}

fn split_glsl_source(source: &str, path: &Path) -> Result<(String, String), String> {
    let mut vert_lines: Vec<&str> = Vec::new();
    let mut frag_lines: Vec<&str> = Vec::new();
    let mut current: Option<&str> = None;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "#type vertex" {
            current = Some("vertex");
        } else if trimmed == "#type fragment" {
            current = Some("fragment");
        } else {
            match current {
                Some("vertex") => vert_lines.push(line),
                Some("fragment") => frag_lines.push(line),
                _ => {}
            }
        }
    }

    if vert_lines.is_empty() {
        return Err(format!(
            "'{}': missing '#type vertex' section",
            path.display()
        ));
    }
    if frag_lines.is_empty() {
        return Err(format!(
            "'{}': missing '#type fragment' section",
            path.display()
        ));
    }

    Ok((vert_lines.join("\n"), frag_lines.join("\n")))
}

fn extract_compute_source(source: &str, path: &Path) -> Result<String, String> {
    let mut comp_lines: Vec<&str> = Vec::new();
    let mut in_compute = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "#type compute" {
            in_compute = true;
        } else if in_compute {
            comp_lines.push(line);
        }
    }

    if comp_lines.is_empty() {
        return Err(format!(
            "'{}': has '#type compute' marker but no source code",
            path.display()
        ));
    }

    Ok(comp_lines.join("\n"))
}

fn run_glslc(input: &Path, output: &Path) -> Result<(), String> {
    let result = Command::new("glslc")
        .arg("--target-env=vulkan1.2")
        .arg(input)
        .arg("-o")
        .arg(output)
        .output();

    match result {
        Ok(output_result) => {
            if output_result.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output_result.stderr);
                Err(format!(
                    "glslc failed for '{}':\n{}",
                    input.display(),
                    stderr
                ))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err("'glslc' not found in PATH. Install the Vulkan SDK.".to_string())
        }
        Err(e) => Err(format!("Failed to run glslc: {e}")),
    }
}
