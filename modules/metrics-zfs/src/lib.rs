use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use serde::{Deserialize, Serialize};
use metrics_api::{MonitorModule, ModuleMetadata, DbBackend, Metric, write_metrics};

#[derive(Debug, Deserialize, Serialize)]
pub struct ZfsConfig {
    pub pools: Vec<String>,
    pub mock: bool,
}

pub struct ZfsModule;

impl Default for ZfsModule {
    fn default() -> Self {
        Self
    }
}

fn parse_zfs_size(s: &str) -> f64 {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return 0.0;
    }
    let val_part: String = s.chars().take_while(|c| c.is_numeric() || *c == '.').collect();
    let unit_part: String = s.chars().skip_while(|c| c.is_numeric() || *c == '.').collect();
    
    let val: f64 = val_part.parse().unwrap_or(0.0);
    match unit_part.as_str() {
        "T" | "TB" => val * 1_099_511_627_776.0,
        "G" | "GB" => val * 1_073_741_824.0,
        "M" | "MB" => val * 1_048_576.0,
        "K" | "KB" => val * 1_024.0,
        _ => val,
    }
}

impl MonitorModule for ZfsModule {
    fn metadata(&self) -> ModuleMetadata {
        ModuleMetadata {
            name: "ZFS Storage".to_string(),
            version: "0.1.0".to_string(),
            author: "Metrics Core Team".to_string(),
            description: "Check status and usage of local ZFS pools".to_string(),
            default_config: "{\n  \"pools\": [\"tank\"],\n  \"mock\": true\n}".to_string(),
        }
    }

    fn collect<'a>(
        &'a self,
        config_str: &'a str,
        backend: &'a DbBackend,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let config: ZfsConfig = serde_json::from_str(config_str)
                .map_err(|e| format!("Failed to parse config: {}", e))?;

            let mut metrics = Vec::new();
            let timestamp = chrono::Utc::now().timestamp_millis();

            if config.mock {
                // Generate simulated metrics for testing
                for pool in &config.pools {
                    let mut labels = HashMap::new();
                    labels.insert("pool".to_string(), pool.clone());

                    metrics.push(Metric {
                        name: "zfs_pool_size_bytes".to_string(),
                        value: 4_398_046_511_104.0, // 4 TB
                        labels: labels.clone(),
                        timestamp_ms: timestamp,
                    });
                    metrics.push(Metric {
                        name: "zfs_pool_used_bytes".to_string(),
                        value: 1_099_511_627_776.0, // 1 TB
                        labels: labels.clone(),
                        timestamp_ms: timestamp,
                    });
                    metrics.push(Metric {
                        name: "zfs_pool_free_bytes".to_string(),
                        value: 3_298_534_883_328.0, // 3 TB
                        labels: labels.clone(),
                        timestamp_ms: timestamp,
                    });
                    metrics.push(Metric {
                        name: "zfs_pool_health".to_string(),
                        value: 1.0, // 1 = ONLINE
                        labels: labels,
                        timestamp_ms: timestamp,
                    });
                }
            } else {
                // Attempt to run actual zpool CLI command
                let output = tokio::process::Command::new("zpool")
                    .args(["list", "-H", "-o", "name,size,alloc,free,health"])
                    .output()
                    .await
                    .map_err(|e| format!("Failed to run zpool CLI: {}", e))?;

                if !output.status.success() {
                    let err = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("zpool command returned error: {}", err));
                }

                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() < 5 {
                        continue;
                    }
                    let pool_name = parts[0].to_string();
                    
                    // Only process pools specified in config
                    if !config.pools.contains(&pool_name) {
                        continue;
                    }

                    let size = parse_zfs_size(parts[1]);
                    let alloc = parse_zfs_size(parts[2]);
                    let free = parse_zfs_size(parts[3]);
                    let health_status = parts[4];
                    let health_val = if health_status == "ONLINE" { 1.0 } else { 0.0 };

                    let mut labels = HashMap::new();
                    labels.insert("pool".to_string(), pool_name);

                    metrics.push(Metric {
                        name: "zfs_pool_size_bytes".to_string(),
                        value: size,
                        labels: labels.clone(),
                        timestamp_ms: timestamp,
                    });
                    metrics.push(Metric {
                        name: "zfs_pool_used_bytes".to_string(),
                        value: alloc,
                        labels: labels.clone(),
                        timestamp_ms: timestamp,
                    });
                    metrics.push(Metric {
                        name: "zfs_pool_free_bytes".to_string(),
                        value: free,
                        labels: labels.clone(),
                        timestamp_ms: timestamp,
                    });
                    metrics.push(Metric {
                        name: "zfs_pool_health".to_string(),
                        value: health_val,
                        labels: labels,
                        timestamp_ms: timestamp,
                    });
                }
            }

            write_metrics(backend, &metrics).await
        })
    }

    fn health<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            Ok(())
        })
    }
}

#[no_mangle]
#[allow(improper_ctypes_definitions)]
pub extern "C" fn create_module() -> *mut dyn MonitorModule {
    let module = ZfsModule::default();
    let boxed = Box::new(module) as Box<dyn MonitorModule>;
    Box::into_raw(boxed)
}
