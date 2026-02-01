//! Build script for gvthread-runtime
//!
//! Handles configuration merging:
//! 1. Start with library defaults
//! 2. If GVT_CONFIG_RS env var is set, parse user's config file
//! 3. Merge user values over defaults (user wins)
//! 4. Generate OUT_DIR/gvt_merged_config.rs
//!
//! User only needs to specify values they want to change.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

/// Configuration parameter definition
struct ConfigParam {
    name: &'static str,
    rust_type: &'static str,
    default_value: &'static str,
}

/// All configuration parameters with their defaults
const CONFIG_PARAMS: &[ConfigParam] = &[
    ConfigParam {
        name: "NUM_WORKERS",
        rust_type: "usize",
        default_value: "4",
    },
    ConfigParam {
        name: "NUM_LOW_PRIORITY_WORKERS",
        rust_type: "usize",
        default_value: "1",
    },
    ConfigParam {
        name: "MAX_GVTHREADS",
        rust_type: "usize",
        default_value: "1_048_576", // 1M
    },
    ConfigParam {
        name: "TIME_SLICE_MS",
        rust_type: "u64",
        default_value: "10",
    },
    ConfigParam {
        name: "GRACE_PERIOD_MS",
        rust_type: "u64",
        default_value: "1",
    },
    ConfigParam {
        name: "TIMER_INTERVAL_MS",
        rust_type: "u64",
        default_value: "1",
    },
    ConfigParam {
        name: "TIMER_MAX_SLEEP_MS",
        rust_type: "u64",
        default_value: "10",
    },
    ConfigParam {
        name: "ENABLE_FORCED_PREEMPT",
        rust_type: "bool",
        default_value: "true",
    },
    ConfigParam {
        name: "DEBUG_LOGGING",
        rust_type: "bool",
        default_value: "false",
    },
    ConfigParam {
        name: "STACK_SIZE",
        rust_type: "usize",
        default_value: "16 * 1024 * 1024", // 16MB
    },
    ConfigParam {
        name: "LOCAL_QUEUE_CAPACITY",
        rust_type: "usize",
        default_value: "256",
    },
    ConfigParam {
        name: "GLOBAL_QUEUE_CAPACITY",
        rust_type: "usize",
        default_value: "65536",
    },
    ConfigParam {
        name: "IDLE_SPINS",
        rust_type: "u32",
        default_value: "10",
    },
    ConfigParam {
        name: "PARK_TIMEOUT_MS",
        rust_type: "u64",
        default_value: "100",
    },
];

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest_path = Path::new(&out_dir).join("gvt_merged_config.rs");

    // Start with defaults
    let mut config: HashMap<&str, String> = CONFIG_PARAMS
        .iter()
        .map(|p| (p.name, p.default_value.to_string()))
        .collect();

    // If user config specified, parse and merge
    if let Ok(user_path) = env::var("GVT_CONFIG_RS") {
        println!("cargo:rerun-if-changed={}", user_path);
        
        match fs::read_to_string(&user_path) {
            Ok(content) => {
                parse_and_merge(&content, &mut config);
                println!("cargo:warning=Using custom config: {}", user_path);
            }
            Err(e) => {
                println!(
                    "cargo:warning=Failed to read GVT_CONFIG_RS ({}): {}",
                    user_path, e
                );
            }
        }
    }
    
    println!("cargo:rerun-if-env-changed=GVT_CONFIG_RS");

    // Generate merged config file
    let output = generate_config(&config);
    fs::write(&dest_path, &output).expect("Failed to write merged config");
}

/// Parse user's config file and merge values into config map
fn parse_and_merge(content: &str, config: &mut HashMap<&str, String>) {
    // Simple parser for: pub const NAME: TYPE = VALUE;
    // Handles multi-line and various spacing
    
    for line in content.lines() {
        let line = line.trim();
        
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        
        // Look for "pub const NAME"
        if !line.starts_with("pub const ") {
            continue;
        }
        
        // Extract name and value
        if let Some(parsed) = parse_const_line(line) {
            let (name, value) = parsed;
            
            // Only accept known parameters
            if CONFIG_PARAMS.iter().any(|p| p.name == name) {
                config.insert(
                    CONFIG_PARAMS.iter().find(|p| p.name == name).unwrap().name,
                    value,
                );
            } else {
                println!("cargo:warning=Unknown config parameter: {}", name);
            }
        }
    }
}

/// Parse a single const line and return (name, value)
fn parse_const_line(line: &str) -> Option<(String, String)> {
    // Format: pub const NAME: TYPE = VALUE;
    
    // Remove "pub const "
    let rest = line.strip_prefix("pub const ")?.trim();
    
    // Find name (before ':')
    let colon_pos = rest.find(':')?;
    let name = rest[..colon_pos].trim().to_string();
    
    // Find value (after '=', before ';')
    let eq_pos = rest.find('=')?;
    let semi_pos = rest.rfind(';').unwrap_or(rest.len());
    
    let value = rest[eq_pos + 1..semi_pos].trim().to_string();
    
    Some((name, value))
}

/// Generate the merged config Rust file
fn generate_config(config: &HashMap<&str, String>) -> String {
    let mut output = String::new();
    
    output.push_str("// Auto-generated by build.rs - do not edit\n");
    output.push_str("// Configuration merged from library defaults");
    
    if env::var("GVT_CONFIG_RS").is_ok() {
        output.push_str(" and user's gvt_config.rs");
    }
    output.push_str("\n\n");
    
    // Generate each constant
    for param in CONFIG_PARAMS {
        let value = config.get(param.name).unwrap();
        output.push_str(&format!(
            "pub const {}: {} = {};\n",
            param.name, param.rust_type, value
        ));
    }
    
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_const_line() {
        let result = parse_const_line("pub const NUM_WORKERS: usize = 8;");
        assert_eq!(result, Some(("NUM_WORKERS".into(), "8".into())));

        let result = parse_const_line("pub const ENABLE_FORCED_PREEMPT: bool = false;");
        assert_eq!(
            result,
            Some(("ENABLE_FORCED_PREEMPT".into(), "false".into()))
        );

        let result = parse_const_line("pub const STACK_SIZE: usize = 16 * 1024 * 1024;");
        assert_eq!(
            result,
            Some(("STACK_SIZE".into(), "16 * 1024 * 1024".into()))
        );
    }

    #[test]
    fn test_parse_and_merge() {
        let mut config: HashMap<&str, String> = HashMap::new();
        config.insert("NUM_WORKERS", "4".into());
        config.insert("TIME_SLICE_MS", "10".into());

        let user_config = r#"
            // Custom config
            pub const NUM_WORKERS: usize = 16;
            pub const TIME_SLICE_MS: u64 = 5;
        "#;

        parse_and_merge(user_config, &mut config);

        assert_eq!(config.get("NUM_WORKERS"), Some(&"16".to_string()));
        assert_eq!(config.get("TIME_SLICE_MS"), Some(&"5".to_string()));
    }
}