use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleMetadata {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub default_config: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub value: f64,
    pub labels: HashMap<String, String>,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DbBackend {
    InfluxDb {
        url: String,
        token: String,
        org: String,
        bucket: String,
    },
    PrometheusPushgateway {
        url: String,
        job: String,
        token: Option<String>,
    },
}

pub trait MonitorModule: Send + Sync {
    fn metadata(&self) -> ModuleMetadata;
    fn collect<'a>(
        &'a self,
        config: &'a str,
        backend: &'a DbBackend,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
    fn health<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

// Helper to write metrics to InfluxDB or Prometheus Pushgateway
pub async fn write_metrics(backend: &DbBackend, metrics: &[Metric]) -> Result<(), String> {
    if metrics.is_empty() {
        return Ok(());
    }

    let client = reqwest::Client::new();

    match backend {
        DbBackend::InfluxDb { url, token, org, bucket } => {
            // InfluxDB Line Protocol
            // Measurement name is the metric name.
            let mut payload = String::new();
            for m in metrics {
                let mut line = m.name.clone();
                for (k, v) in &m.labels {
                    line.push_str(&format!(",{}={}", k, v));
                }
                line.push_str(&format!(" value={} {}", m.value, m.timestamp_ms));
                payload.push_str(&line);
                payload.push('\n');
            }

            let write_url = format!("{}/api/v2/write?org={}&bucket={}&precision=ms", url.trim_end_matches('/'), org, bucket);
            let res = client.post(&write_url)
                .header("Authorization", format!("Token {}", token))
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(payload)
                .send()
                .await
                .map_err(|e| format!("InfluxDB request failed: {}", e))?;

            if !res.status().is_success() {
                let err_body = res.text().await.unwrap_or_default();
                return Err(format!("InfluxDB write failed ({}): {}", write_url, err_body));
            }
        }
        DbBackend::PrometheusPushgateway { url, job, token } => {
            // Prometheus Text Format
            let mut payload = String::new();
            for m in metrics {
                // Sanitize name (replace invalid chars with underscores)
                let sanitized_name = m.name.replace(|c: char| !c.is_alphanumeric() && c != '_', "_");
                payload.push_str(&format!("# TYPE {} gauge\n", sanitized_name));
                
                if m.labels.is_empty() {
                    payload.push_str(&format!("{} {}\n", sanitized_name, m.value));
                } else {
                    let label_str = m.labels.iter()
                        .map(|(k, v)| format!("{}=\"{}\"", k, v))
                        .collect::<Vec<_>>()
                        .join(",");
                    payload.push_str(&format!("{}{{{}}} {}\n", sanitized_name, label_str, m.value));
                }
            }

            let write_url = format!("{}/metrics/job/{}", url.trim_end_matches('/'), job);
            let mut req = client.post(&write_url)
                .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
                .body(payload);

            if let Some(t) = token {
                req = req.header("Authorization", format!("Bearer {}", t));
            }

            let res = req.send().await.map_err(|e| format!("Prometheus Pushgateway request failed: {}", e))?;

            if !res.status().is_success() {
                let err_body = res.text().await.unwrap_or_default();
                return Err(format!("Prometheus Pushgateway write failed ({}): {}", write_url, err_body));
            }
        }
    }

    Ok(())
}
