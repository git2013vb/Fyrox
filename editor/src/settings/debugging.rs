use fyrox::core::reflect::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, PartialEq, Clone, Debug, Reflect, Eq)]
pub struct DebuggingSettings {
    pub show_physics: bool,
    pub show_bounds: bool,
    pub show_tbn: bool,
}

impl Default for DebuggingSettings {
    fn default() -> Self {
        Self {
            show_physics: true,
            show_bounds: true,
            show_tbn: false,
        }
    }
}
