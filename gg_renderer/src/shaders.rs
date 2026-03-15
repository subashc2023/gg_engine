//! Built-in engine shader SPIR-V bytecode.
//!
//! Generated at build time by `build.rs`, which compiles the `.glsl` files
//! in `src/renderer/shaders/` using `glslc`.
//!
//! Naming convention: `{SHADER_NAME}_{VERT|FRAG}_SPV`

include!(concat!(env!("OUT_DIR"), "/shaders.rs"));
