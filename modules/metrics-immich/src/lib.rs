use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use serde::{Deserialize, Serialize};
use metrics_api::{MonitorModule, ModuleMetadata, DbBackend, Metric, write_metrics};

#[derive(Debug, Deserialize, Serialize)]
pub struct ImmichConfig {
    pub url: String,
    pub api_key: String,
    pub mock: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImmichStats {
    pub photos: f64,
    pub videos: f64,
    pub usage: f64,
}

pub struct ImmichModule;

impl Default for ImmichModule {
    fn default() -> Self {
        Self
    }
}

impl MonitorModule for ImmichModule {
    fn metadata(&self) -> ModuleMetadata {
        ModuleMetadata {
            name: "Immich Statistics".to_string(),
            version: "0.1.0".to_string(),
            author: "Metrics Core Team".to_string(),
            description: "Collects total assets and user counts from Immich API".to_string(),
            default_config: "{\n  \"url\": \"http://localhost:2283\",\n  \"api_key\": \"YOUR_API_KEY\",\n  \"mock\": true\n}".to_string(),
        }
    }

    fn collect<'a>(
        &'a self,
        config_str: &'a str,
        backend: &'a DbBackend,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let config: ImmichConfig = serde_json::from_str(config_str)
                .map_err(|e| format!("Failed to parse config: {}", e))?;

            let mut metrics = Vec::new();
            let timestamp = chrono::Utc::now().timestamp_millis();

            if config.mock {
                // Mock metrics
                metrics.push(Metric {
                    name: "immich_photos_total".to_string(),
                    value: 23145.0,
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });
                metrics.push(Metric {
                    name: "immich_videos_total".to_string(),
                    value: 842.0,
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });
                metrics.push(Metric {
                    name: "immich_storage_used_bytes".to_string(),
                    value: 582_341_983_212.0, // 582 GB
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });
            } else {
                // Fetch from Immich API
                let client = reqwest::Client::new();
                let api_url = format!("{}/api/server-info/statistics", config.url.trim_end_matches('/'));
                
                let res = client.get(&api_url)
                    .header("x-api-key", &config.api_key)
                    .send()
                    .await
                    .map_err(|e| format!("Failed to connect to Immich API: {}", e))?;

                if !res.status().is_success() {
                    let status = res.status();
                    let err = res.text().await.unwrap_or_default();
                    return Err(format!("Immich API returned error ({}): {}", status, err));
                }

                let stats: ImmichStats = res.json()
                    .await
                    .map_err(|e| format!("Failed to parse Immich response: {}", e))?;

                metrics.push(Metric {
                    name: "immich_photos_total".to_string(),
                    value: stats.photos,
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });
                metrics.push(Metric {
                    name: "immich_videos_total".to_string(),
                    value: stats.videos,
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });
                metrics.push(Metric {
                    name: "immich_storage_used_bytes".to_string(),
                    value: stats.usage,
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });
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
    let module = ImmichModule::default();
    let boxed = Box::new(module) as Box<dyn MonitorModule>;
    Box::into_raw(boxed)
}
