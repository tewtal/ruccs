use tokio::net::{TcpListener, TcpStream};
use std::collections::HashMap;
use std::net::SocketAddr;

use tokio::sync::mpsc::{Sender, channel};
use tokio::io::{AsyncWriteExt};
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use uuid::Uuid;

use crate::devices::device::{DeviceRequest, DeviceResponse, Device, DeviceInfo, DeviceManagerCommand};
use crate::manager::ManagerInfo;
use crate::protocol::{Command};


#[allow(dead_code)]
pub struct Lua {
    id: Uuid,
    name: String,
    stream: TcpStream,
    addr: SocketAddr
}

/* This implements the core of a Lua network device */
impl Lua {
    pub fn new(id: Uuid, stream: TcpStream, addr: SocketAddr) -> Self
    {
        Self {
            id,
            name: format!("LuaDevice {} :: {:?}", id, addr),
            stream,
            addr
        }
    }

    /* This starts a device when an incoming connection is opened to the LUA Device port */
    /* It spawns a tokio task that listens for both incoming device messages and tcp/ip events */
    async fn start(id: Uuid, stream: TcpStream, addr: SocketAddr, lua_manager_tx: Sender<(Uuid, DeviceInfo)>) -> Device {
        let lua = Lua::new(id, stream, addr);

        /* The main communication channel for this specific device, all clients will have to go through this to use the device */
        let (device_tx, mut device_rx) = channel(128); // (Not sure about what's a reasonable size here)

        let device = Device {          
            id,  
            name: lua.name.to_string(),
            sender: device_tx
        };

        tokio::spawn(async move {
            let mut buf = vec![];
            let mut stream = BufReader::new(lua.stream);
            loop {
                tokio::select! {
                    Some(command) = device_rx.recv() => {
                        match command {
                            DeviceRequest::Request { req, resp: sender } => {
                                match req.command {
                                    Command::Info => sender.send(DeviceResponse::Strings(vec!["1.0".to_string(), "0000".to_string(), "n/a".to_string(), format!("{}", lua.name)])).unwrap(),
                                    Command::GetAddress(addr_info) => {
                                        let (tx, rx) = tokio::sync::mpsc::channel(32);                                        
                                        let response = DeviceResponse::BinaryReader((addr_info[0].size as usize, rx));
                                        sender.send(response).unwrap();
                                        tx.send(vec![0u8; addr_info[0].size as usize]).await.unwrap();                                        
                                    }
                                    _ => sender.send(DeviceResponse::Nothing).unwrap()
                                }
                            },
                            DeviceRequest::Close => {
                                stream.shutdown().await.unwrap();
                                return;
                            },
                        }    
                    },
                    result = stream.read_until(b'\n', &mut buf) => {
                        match result {
                            Ok(len) => { 
                                println!("Received {} bytes: {:?}", len, buf);
                                buf.clear();
                            }
                            Err(e) => {
                                lua_manager_tx.send((id, DeviceInfo::ConnectionClosed(e.to_string()))).await.unwrap();
                                return;
                            }
                        }
                    },                    
                }
            }
        });

        device
    }
}

pub struct LuaManager {}
impl LuaManager {
    pub async fn start(sender: Sender<ManagerInfo>) -> Sender<DeviceManagerCommand> {
        let (device_manager_tx, mut device_manager_rx) = tokio::sync::mpsc::channel(32);
        let mut devices = HashMap::new();
        
        tokio::spawn(async move {
            let (lua_manager_tx, mut lua_manager_rx) = tokio::sync::mpsc::channel(32);
            let listener = TcpListener::bind("127.0.0.1:4141").await.unwrap();
    
            loop {
                tokio::select! {
                    Ok((socket, addr)) = listener.accept() => {
                        let id = Uuid::new_v4();
                        let device = Lua::start(id, socket, addr, lua_manager_tx.clone()).await;
                        {
                            sender.send(ManagerInfo::DeviceCreated(device.clone())).await.unwrap();
                            devices.insert(id, device);
                        }
                    },
                    Some((device_id, message)) = lua_manager_rx.recv() => {
                        match message {
                            DeviceInfo::ConnectionClosed(_reason) => {
                                sender.send(ManagerInfo::DeviceRemoved(device_id)).await.unwrap();
                                devices.remove(&device_id);
                            }
                        }
                    },
                    Some(message) = device_manager_rx.recv() => {
                        match message {
                            DeviceManagerCommand::Close => {
                                for (id, device) in &devices {
                                    device.sender.send(DeviceRequest::Close).await.unwrap();
                                    sender.send(ManagerInfo::DeviceRemoved(*id)).await.unwrap();
                                }
                            }
                        }
                    }
                }
            }
        });

        device_manager_tx
    }
}