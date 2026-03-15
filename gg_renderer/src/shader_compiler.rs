//! Runtime GLSL → SPIR-V compilation for shader hot-reload.
//!
//! Replicates the logic from `build.rs` but returns `Result` instead of
//! panicking, so the editor can display errors and keep the old pipelines.

use std::path::Path;
use std::process::Command;

use gg_core::error::{EngineError, EngineResult};

/// Compiled SPIR-V bytecode for a single shader file (vertex + fragment).
pub struct CompiledShader {
    pub vert_spv: Vec<u8>,
    pub frag_spv: Vec<u8>,
}

/// Compiled SPIR-V bytecode for a compute shader.
pub struct CompiledComputeShader {
    pub comp_spv: Vec<u8>,
}

/// Compile a `.glsl` source file into vertex and fragment SPIR-V bytecode.
///
/// The source must contain `#type vertex` and `#type fragment` markers
/// (same format as the build-time shader pipeline).
pub fn compile_glsl(path: &Path) -> EngineResult<CompiledShader> {
    compile_glsl_with_defines(path, &[])
}

/// Compile a `.glsl` source file into vertex and fragment SPIR-V bytecode
/// with the `OFFSCREEN` preprocessor define enabled.
pub fn compile_glsl_offscreen(path: &Path) -> EngineResult<CompiledShader> {
    compile_glsl_with_defines(path, &["OFFSCREEN"])
}

/// Compile a `.glsl` source file into vertex and fragment SPIR-V bytecode
/// with the given preprocessor defines (passed as `-DNAME` to glslc).
pub fn compile_glsl_with_defines(
    path: &Path,
    defines: &[&str],
) -> EngineResult<CompiledShader> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| EngineError::Gpu(format!("Cannot read '{}': {e}", path.display())))?;

    let (vert_src, frag_src) = split_glsl_source(&source, path)?;

    let temp_dir = std::env::temp_dir().join("gg_shader_hotreload");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| EngineError::Gpu(format!("Cannot create temp dir: {e}")))?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| EngineError::Gpu(format!("Invalid shader path: {}", path.display())))?;

    let vert_tmp = temp_dir.join(format!("{stem}.vert"));
    let frag_tmp = temp_dir.join(format!("{stem}.frag"));
    std::fs::write(&vert_tmp, &vert_src)
        .map_err(|e| EngineError::Gpu(format!("Cannot write temp vert: {e}")))?;
    std::fs::write(&frag_tmp, &frag_src)
        .map_err(|e| EngineError::Gpu(format!("Cannot write temp frag: {e}")))?;

    let vert_spv_path = temp_dir.join(format!("{stem}_vert.spv"));
    let frag_spv_path = temp_dir.join(format!("{stem}_frag.spv"));

    run_glslc_with_defines(&vert_tmp, &vert_spv_path, defines)?;
    run_glslc_with_defines(&frag_tmp, &frag_spv_path, defines)?;

    let vert_spv = std::fs::read(&vert_spv_path)
        .map_err(|e| EngineError::Gpu(format!("Cannot read compiled vert SPIR-V: {e}")))?;
    let frag_spv = std::fs::read(&frag_spv_path)
        .map_err(|e| EngineError::Gpu(format!("Cannot read compiled frag SPIR-V: {e}")))?;

    Ok(CompiledShader { vert_spv, frag_spv })
}

/// Returns `true` if the shader source contains an `#ifdef OFFSCREEN` guard,
/// indicating it needs both an offscreen and swapchain compilation variant.
pub fn has_offscreen_ifdef(source: &str) -> bool {
    source.contains("#ifdef OFFSCREEN")
}

/// Compile a `.glsl` compute shader source file into SPIR-V bytecode.
///
/// The source must contain a `#type compute` marker.
pub fn compile_compute_glsl(path: &Path) -> EngineResult<CompiledComputeShader> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| EngineError::Gpu(format!("Cannot read '{}': {e}", path.display())))?;

    let comp_src = extract_compute_source(&source, path)?;

    let temp_dir = std::env::temp_dir().join("gg_shader_hotreload");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| EngineError::Gpu(format!("Cannot create temp dir: {e}")))?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| EngineError::Gpu(format!("Invalid shader path: {}", path.display())))?;

    let comp_tmp = temp_dir.join(format!("{stem}.comp"));
    std::fs::write(&comp_tmp, &comp_src)
        .map_err(|e| EngineError::Gpu(format!("Cannot write temp comp: {e}")))?;

    let comp_spv_path = temp_dir.join(format!("{stem}_comp.spv"));
    run_glslc(&comp_tmp, &comp_spv_path)?;

    let comp_spv = std::fs::read(&comp_spv_path)
        .map_err(|e| EngineError::Gpu(format!("Cannot read compiled comp SPIR-V: {e}")))?;

    Ok(CompiledComputeShader { comp_spv })
}

fn split_glsl_source(source: &str, path: &Path) -> EngineResult<(String, String)> {
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
        return Err(EngineError::Gpu(format!(
            "'{}': missing '#type vertex' section",
            path.display()
        )));
    }
    if frag_lines.is_empty() {
        return Err(EngineError::Gpu(format!(
            "'{}': missing '#type fragment' section",
            path.display()
        )));
    }

    Ok((vert_lines.join("\n"), frag_lines.join("\n")))
}

fn extract_compute_source(source: &str, path: &Path) -> EngineResult<String> {
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
        return Err(EngineError::Gpu(format!(
            "'{}': has '#type compute' marker but no source code",
            path.display()
        )));
    }

    Ok(comp_lines.join("\n"))
}

fn run_glslc(input: &Path, output: &Path) -> EngineResult<()> {
    run_glslc_with_defines(input, output, &[])
}

fn run_glslc_with_defines(input: &Path, output: &Path, defines: &[&str]) -> EngineResult<()> {
    let mut cmd = Command::new("glslc");
    cmd.arg("--target-env=vulkan1.2");

    for def in defines {
        cmd.arg(format!("-D{def}"));
    }

    let result = cmd.arg(input).arg("-o").arg(output).output();

    match result {
        Ok(output_result) => {
            if output_result.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output_result.stderr);
                Err(EngineError::Gpu(format!(
                    "glslc failed for '{}':\n{}",
                    input.display(),
                    stderr
                )))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(EngineError::Gpu(
            "'glslc' not found in PATH. Install the Vulkan SDK.".to_string(),
        )),
        Err(e) => Err(EngineError::Gpu(format!("Failed to run glslc: {e}"))),
    }
}
