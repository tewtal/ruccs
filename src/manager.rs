use crate::devices::{device::{self, Device, Command}, lua::{Lua, LuaManager}};
use uuid::Uuid;
use tokio::sync::{RwLock, oneshot::channel};
use std::sync::Arc;
use std::collections::HashMap;

// pub enum Response {
//     Text(String),
//     Binary(Vec<u8>),
//     Empty,
//     Close
// }

#[derive(Debug)]
pub enum ManagerInfo {
    DeviceCreated(Device),
    DeviceRemoved(Uuid)
}

#[derive(Debug)]
pub struct Manager {
    pub name: String,
    pub devices: HashMap<Uuid, Device>,
}

impl Manager {
    pub fn new() -> Self {
        let name = "Device Manager".into();
        Self {
            name,
            devices: HashMap::new()
        }
    }

    pub async fn get_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for d in &self.devices {
            names.push(d.1.name.to_string());
        }
        names.push(self.name.to_string());
        names
    }
}

/* This task creates the device manager tasks and handles communication with them
   through message channels to get notifications when devices are added and removed and so on */
pub async fn run_manager(manager: Arc<RwLock<Manager>>) {        
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);    
    let _lua_sender = LuaManager::start(tx.clone()).await;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(message) = rx.recv() => {
                    match message {
                        ManagerInfo::DeviceCreated(device) => {
                            {
                                println!("DeviceManager: Created device {:?}", &device);
                                let mut mgr = manager.write().await;
                                mgr.devices.insert(device.id, device);
                            }
                        },
                        ManagerInfo::DeviceRemoved(device_id) => {
                            {
                                println!("DeviceManager: Removed device {:?}", &device_id);
                                let mut mgr = manager.write().await;
                                mgr.devices.remove(&device_id);
                            }
                        }
                    }
                }
            }
        }
    });

}