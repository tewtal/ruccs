use crate::devices::{device::{Device}, lua::{LuaManager}, retroarch::RetroarchManager, sd2snes::SD2SnesManager, emunwa::EmuNwaManager};
use uuid::Uuid;
use tokio::sync::{RwLock};
use std::sync::Arc;
use std::collections::HashMap;
use crate::protocol::{Request};
use crate::devices::device::DeviceResponse;

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

    pub fn get_names(&self) -> Vec<String> {
        self.devices.iter().map(|(_,d)| d.name.to_string()).collect()
    }

    pub fn get_device(&self, device_name: &str) -> Option<(&Uuid, &Device)> {
        self.devices.iter().find(|(_, d)| d.name == device_name)
    }

    /* Send a request to a device */
    pub async fn request(&self, req: Request, device_id: Uuid) -> Result<DeviceResponse, Box<dyn std::error::Error + Send + Sync>> {
        if self.devices.contains_key(&device_id) {
            let device = &self.devices[&device_id];
            let information = device.request(req).await?;    
            Ok(information)
        } else {
            Err("The requested device does not exist".into())
        }
    }
}

/* This task creates the device manager tasks and handles communication with them
   through message channels to get notifications when devices are added and removed and so on */
pub async fn run_manager(manager: Arc<RwLock<Manager>>) {        
    let (manager_tx, mut manager_rx) = tokio::sync::mpsc::channel(32);    
    
    /* Start the LuaManager task that handles incoming TCP requests for LUA devices */
    let _lua_sender = LuaManager::start(manager_tx.clone()).await;

    /* Start the SD2SnesManager task */
    let _snes_sender = SD2SnesManager::start(manager_tx.clone()).await;

    /* Start the RetroarchManager task */
    let _ra_sender = RetroarchManager::start(manager_tx.clone()).await;

    /* Start the EmuNWAManager task */
    let _emunwa_sender = EmuNwaManager::start(manager_tx.clone()).await;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(message) = manager_rx.recv() => {
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