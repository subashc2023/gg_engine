use std::collections::HashMap;
use std::sync::Arc;

use super::shader::Shader;

/// A name-keyed registry of compiled [`Shader`]s.
///
/// Stores `Arc<Shader>` so multiple pipelines can share the same shader.
pub struct ShaderLibrary {
    shaders: HashMap<String, Arc<Shader>>,
}

impl ShaderLibrary {
    pub fn new() -> Self {
        Self {
            shaders: HashMap::new(),
        }
    }

    /// Add a shader, keyed by its [`Shader::name`].
    pub fn add(&mut self, shader: Arc<Shader>) {
        self.shaders.insert(shader.name().to_string(), shader);
    }

    /// Retrieve a shader by name.
    pub fn get(&self, name: &str) -> Option<Arc<Shader>> {
        self.shaders.get(name).cloned()
    }

    /// Returns `true` if the library contains a shader with the given name.
    pub fn contains(&self, name: &str) -> bool {
        self.shaders.contains_key(name)
    }
}

impl Default for ShaderLibrary {
    fn default() -> Self {
        Self::new()
    }
}
