use qt_core::q_meta_object::Connection;
use tokio::net::{UdpSocket};
use tokio::time::error::Elapsed;
use tokio::time::{Duration, timeout};
use std::collections::HashMap;
use std::net::SocketAddr;

use tokio::sync::mpsc::{Sender, channel};
use tokio::io::{AsyncWriteExt};
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use uuid::Uuid;

use crate::devices::device::{DeviceRequest, DeviceResponse, Device, DeviceInfo, DeviceManagerCommand};
use crate::manager::ManagerInfo;
use crate::protocol::{AddressInfo, Command, MemoryDomain};


#[allow(dead_code)]
pub struct Retroarch {
    id: Uuid,
    name: String,
    socket: UdpSocket,
    addr: SocketAddr,
    ra_version: String,
    rom_access: bool,
    rom_name: String
}

/* This implements the core of a Lua network device */
impl Retroarch {
    pub fn new(id: Uuid, socket: UdpSocket, addr: SocketAddr, ra_version: &str, rom_access: bool, rom_name: &str) -> Self
    {
        Self {
            id,
            name: format!("RetroArchDevice {} :: {:?}", id, addr),
            socket,
            addr,
            ra_version: ra_version.to_string(),
            rom_access,
            rom_name: rom_name.to_string()
            
        }
    }

    fn pc_to_snes(&self, addr: i64) -> i64 {
        let bank = addr / 0x8000;
        let offset = (addr % 0x8000) + 0x8000;
        (bank << 16) | offset
    }

    pub async fn read_stream(&mut self, tx: Sender<Vec<u8>>, data_size: usize, addr_info: &[AddressInfo]) -> Result<usize, Box<dyn std::error::Error + Sync + Send>> {
        let mut buf: Vec<u8> = vec![0u8; 16384];
        
        for addr in addr_info {                        
            let (mut target_addr, target_cmd) = match addr.domain {
                MemoryDomain::WRAM => (addr.address, "READ_CORE_RAM"),
                MemoryDomain::CARTRAM => (addr.address + 0x20000, "READ_CORE_RAM"),
                MemoryDomain::CARTROM => (if !self.rom_access { return Err("ROM Reads are not supported on this device".into()) } else { self.pc_to_snes(addr.address) }, "READ_CORE_MEMORY")
            };


            let mut remaining_len = addr.size;            
            while remaining_len > 0 {
                let mut data: Vec<u8> = Vec::new();
                let cmd_size = if remaining_len > 512 { 512 } else { remaining_len };
                self.socket.send(format!("{} {:X} {}\n", target_cmd, target_addr, cmd_size).as_bytes()).await?;
                
                let mut read_len = 0;
                let resp_len = format!("{} {:X} ", target_cmd, target_addr).len();
                let target_len = resp_len + (cmd_size * 3) as usize;

                while read_len < target_len {
                    let recv_len = timeout(Duration::from_secs(1),self.socket.recv(&mut buf)).await??;
                    data.extend(buf[..recv_len].iter());
                    
                    /* Parse data as string to test for -1 */
                    if String::from_utf8_lossy(&data).contains("-1") {
                        return Err("Tried to read from invalid address".into())
                    }

                    read_len += recv_len;
                }

                /* Parse this block of data */
                let hex_str = String::from_utf8_lossy(&data[resp_len..]).trim_end().to_string();
                let bin_data: Vec<u8> = hex_str.split(' ').into_iter().map(|h| u8::from_str_radix(h, 16).unwrap()).collect();
                let _ = tx.send(bin_data).await;
                remaining_len -= cmd_size;
                target_addr += cmd_size;
            }
        }

        tx.closed().await;

        Ok(data_size)
    }

    /* This starts a device when an incoming connection is opened to the LUA Device port */
    /* It spawns a tokio task that listens for both incoming device messages and tcp/ip events */
    async fn start(id: Uuid, socket: UdpSocket, addr: SocketAddr, ra_version: &str, rom_access: bool, rom_name: &str, ra_manager_tx: Sender<(Uuid, DeviceInfo)>) -> Device {
        let mut ra = Retroarch::new(id, socket, addr, ra_version, rom_access, rom_name);

        /* The main communication channel for this specific device, all clients will have to go through this to use the device */
        let (device_tx, mut device_rx) = channel(1);

        let device = Device {          
            id,  
            name: ra.name.to_string(),
            sender: device_tx
        };

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(command) = device_rx.recv() => {
                        match command {
                            DeviceRequest::Request { req, resp: sender } => {
                                match req.command {
                                    Command::Info => {
                                        let features = "NO_CONTROL_CMD | NO_FILE_CMD".to_string() + (if !ra.rom_access { " | NO_ROM_READ | NO_ROM_WRITE" } else { "" });
                                        let response = DeviceResponse::Strings(vec![ra.ra_version.to_string(), "0".to_string(), ra.rom_name.to_string(), features, "RetroArch".to_string()]);
                                        let _ = sender.send(response);
                                    },
                                    Command::GetAddress(addr_info) => {
                                        let (tx, rx) = tokio::sync::mpsc::channel(32);

                                        /* Calculate the total size and padded size of all requests */
                                        let data_size = addr_info.iter().map(|a| a.size as usize).sum();

                                        /* Create and send response */
                                        let response = DeviceResponse::BinaryReader((data_size as usize, rx));
                                        sender.send(response).unwrap();

                                        /* Read data to the end */
                                        let _ = ra.read_stream(tx, data_size, &addr_info).await;

                                        //tx.send(vec![0u8; addr_info[0].size as usize]).await.unwrap();                                        
                                    },
                                    Command::PutAddress(addr_info) => {
                                        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
                                        let response = DeviceResponse::BinaryWriter((addr_info[0].size as usize, tx));
                                        sender.send(response).unwrap();
                                        let mut remaining = addr_info[0].size as usize;
                                        while remaining > 0 {
                                            if let Some(data) = rx.recv().await {
                                                remaining -= data.len();
                                                //stream.write(&data).await.unwrap();
                                            } else {
                                                break;
                                            }
                                        }
                                    },
                                    _ => () //sender.send(DeviceResponse::Empty).unwrap()
                                }
                            },
                            DeviceRequest::Close => {
                                //stream.shutdown().await.unwrap();                                
                                return;
                            },
                        }    
                    },                  
                }
            }
        });

        device
    }
}

enum RetroarchState {
    Searching,
    Connected,
    Playing,
    Disconnected
}

pub struct RetroarchManager {}
impl RetroarchManager {
    pub async fn start(sender: Sender<ManagerInfo>) -> Sender<DeviceManagerCommand> {
        let (device_manager_tx, mut device_manager_rx) = tokio::sync::mpsc::channel(32);

            
    tokio::spawn(async move {
        let (ra_manager_tx, mut ra_manager_rx) = tokio::sync::mpsc::channel(32);
        let remote_addr: SocketAddr = "127.0.0.1:55355".parse().unwrap();
        let local_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let socket = UdpSocket::bind(local_addr).await.unwrap();
        let _ = socket.connect(&remote_addr).await;        
        let mut state = RetroarchState::Searching;
        let mut recvbuf = vec![0u8; 65_507];
        let mut devices: HashMap<String, Device> = HashMap::new();
        let mut ra_version: String = "Unknown".to_string();

        loop {
            match state {
                RetroarchState::Searching => {
                    let data = "VERSION\n".as_bytes();
                    if socket.send(data).await.is_ok() {
                        if let Ok(len) = socket.recv(&mut recvbuf).await {
                            ra_version = String::from_utf8_lossy(&recvbuf[..len]).to_string();
                            println!("Found RA {}", &ra_version);                           
                            state = RetroarchState::Connected;                            
                        }
                    } 
                },
                RetroarchState::Connected => {
                    let data = "GET_STATUS\n".as_bytes();
                    if socket.send(data).await.is_ok() {
                        if let Ok(len) = socket.recv(&mut recvbuf).await {
                            let status_string = String::from_utf8_lossy(&recvbuf[..len]);
                            println!("Found Status {}", status_string);
                            if !status_string.contains("CONTENTLESS") {
                                /* Detect core features */
                                
                                let _ = socket.send("READ_CORE_RAM 0 10\n".as_bytes()).await;
                                if let Ok(len) = socket.recv(&mut recvbuf).await {
                                    let read_data = String::from_utf8_lossy(&recvbuf[..len]);
                                    if read_data.contains("-1") {
                                        state = RetroarchState::Searching;
                                        continue;
                                    }
                                } else {
                                    state = RetroarchState::Searching;
                                    continue;
                                }

                                let (rom_name, rom_access) = {
                                    let _ = socket.send("READ_CORE_MEMORY FFC0 21\n".as_bytes()).await;
                                    if let Ok(len) = socket.recv(&mut recvbuf).await {
                                        let hex_str = String::from_utf8_lossy(&recvbuf[22..len]).trim_end().to_string();
                                        let bin_data: Vec<u8> = hex_str.split(' ').into_iter().map(|h| u8::from_str_radix(h, 16).unwrap()).collect();
                                        let rom_name = String::from_utf8_lossy(&bin_data).to_string();
                                        (rom_name.to_string(), true)
                                    } else {
                                        (String::default(), false)
                                    }
                                };

                                let id = Uuid::new_v4();
                                let socket = UdpSocket::bind(local_addr).await.unwrap();
                                socket.connect(remote_addr).await.unwrap();                                
                                let device = Retroarch::start(id, socket, remote_addr, &ra_version, rom_access, &rom_name, ra_manager_tx.clone()).await;
                                sender.send(ManagerInfo::DeviceCreated(device.clone())).await.unwrap();
                                devices.insert(remote_addr.to_string(), device);
                                state = RetroarchState::Playing;
                            }
                        } else {
                            state = RetroarchState::Searching;
                        }
                    } else {
                        state = RetroarchState::Searching;
                    }
                },
                RetroarchState::Playing => {
                    
                },
                RetroarchState::Disconnected => {
                    /* Not sure where and when this will be set */
                }                
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }    
    });

        device_manager_tx
    }
}