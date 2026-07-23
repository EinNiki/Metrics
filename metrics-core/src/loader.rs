use std::sync::Arc;
use libloading::{Library, Symbol};
use metrics_api::MonitorModule;

pub struct LoadedModule {
    // Drop order: module must be dropped BEFORE _lib is dropped.
    // In Rust, fields are dropped in the order they are declared in the struct.
    pub module: Box<dyn MonitorModule>,
    _lib: Arc<Library>,
}

// Since the module is Send + Sync, LoadedModule can be Send + Sync.
unsafe impl Send for LoadedModule {}
unsafe impl Sync for LoadedModule {}

impl LoadedModule {
    pub unsafe fn load(path: &str) -> Result<Self, String> {
        let lib = Library::new(path).map_err(|e| format!("Failed to load library: {}", e))?;
        let lib = Arc::new(lib);
        
        let constructor: Symbol<unsafe extern "C" fn() -> *mut dyn MonitorModule> = lib
            .get(b"create_module\0")
            .map_err(|e| format!("Failed to find symbol 'create_module': {}", e))?;
            
        let raw_ptr = constructor();
        if raw_ptr.is_null() {
            return Err("Symbol 'create_module' returned a null pointer".to_string());
        }
        
        let module = Box::from_raw(raw_ptr);
        
        Ok(LoadedModule {
            module,
            _lib: lib,
        })
    }
}
