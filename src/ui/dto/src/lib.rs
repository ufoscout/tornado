#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use typescript_definitions::TypescriptDefinition;

pub mod config;

// This static string will be injected into the TypeScript definition file.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(typescript_custom_section)]
const TS_APPEND_CONTENT: &'static str = r#"
export type Value = any;
"#;

#[derive(Clone, Serialize, Deserialize, TypescriptDefinition, Default)]
pub struct EventDto {
    #[serde(rename = "type")]
    pub event_type: String,
    pub created_ms: u64,
    pub payload: HashMap<String, Value>,
}
