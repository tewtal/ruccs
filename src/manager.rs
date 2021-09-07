use crate::devices::{device::Device, lua::Lua};
use tokio::sync::RwLock;
use std::sync::Arc;

// pub enum Response {
//     Text(String),
//     Binary(Vec<u8>),
//     Empty,
//     Close
// }

pub struct Manager {
    pub name: String,
    pub devices: Vec<RwLock<Box<dyn Device + Send + Sync>>>,
}

impl Manager {
    pub fn new() -> Self {
        let name = "Device Manager".into();
        let lua_device = Lua::new();
        Self {
            name,
            devices: vec![RwLock::new(Box::new(lua_device) as Box<dyn Device + Send + Sync>)],
        }
    }

    pub async fn get_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for d in &self.devices {
            names.push(d.read().await.get_name().to_owned())
        }
        names.push(self.name.to_string());
        names
    }
}

/* This is a task for periodically updating the manager instance.
   It handles things like periodically polling for new devices and so on */
pub async fn run_manager(manager: Arc<RwLock<Manager>>) {
    let mut i = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        
        {
            let mut mgr = manager.write().await;
            mgr.name = format!("Device Manager {}", i);            
        }
        
        i += 1;
    }
}