use std::sync::Mutex;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use metrics_api::{MonitorModule, ModuleMetadata, DbBackend, Metric, write_metrics};
use sysinfo::{System, Disks};

pub struct SystemModule {
    sys: Mutex<System>,
}

impl Default for SystemModule {
    fn default() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        Self {
            sys: Mutex::new(sys),
        }
    }
}

impl MonitorModule for SystemModule {
    fn metadata(&self) -> ModuleMetadata {
        ModuleMetadata {
            name: "System Resources".to_string(),
            version: "0.1.0".to_string(),
            author: "Metrics Core Team".to_string(),
            description: "Monitor CPU, RAM, and disk utilization".to_string(),
            default_config: "{}".to_string(),
        }
    }

    fn collect<'a>(
        &'a self,
        _config: &'a str,
        backend: &'a DbBackend,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            {
                let mut sys = self.sys.lock().unwrap();
                sys.refresh_cpu_all();
                sys.refresh_memory();
            }
            
            // Brief sleep to get correct CPU usage delta
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;

            let mut metrics = Vec::new();
            let timestamp = chrono::Utc::now().timestamp_millis();

            {
                let mut sys = self.sys.lock().unwrap();
                sys.refresh_cpu_all();

                // CPU Usage
                let cpu_usage = sys.global_cpu_usage() as f64;
                metrics.push(Metric {
                    name: "system_cpu_usage".to_string(),
                    value: cpu_usage,
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });

                // Memory Usage
                let total_mem = sys.total_memory() as f64;
                let used_mem = sys.used_memory() as f64;
                metrics.push(Metric {
                    name: "system_mem_total".to_string(),
                    value: total_mem,
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });
                metrics.push(Metric {
                    name: "system_mem_used".to_string(),
                    value: used_mem,
                    labels: HashMap::new(),
                    timestamp_ms: timestamp,
                });

                // Disk Usage
                let mut disks = Disks::new();
                disks.refresh_list();
                for disk in &disks {
                    let mount = disk.mount_point().to_string_lossy().to_string();
                    let total = disk.total_space() as f64;
                    let available = disk.available_space() as f64;
                    let used = total - available;

                    let mut labels = HashMap::new();
                    labels.insert("mount_point".to_string(), mount);

                    metrics.push(Metric {
                        name: "system_disk_total".to_string(),
                        value: total,
                        labels: labels.clone(),
                        timestamp_ms: timestamp,
                    });
                    metrics.push(Metric {
                        name: "system_disk_used".to_string(),
                        value: used,
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
    let module = SystemModule::default();
    let boxed = Box::new(module) as Box<dyn MonitorModule>;
    Box::into_raw(boxed)
}
