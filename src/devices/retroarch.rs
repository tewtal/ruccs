use tokio::net::{UdpSocket};
use tokio::time::{Duration, timeout};
use std::collections::{HashMap};

use tokio::sync::mpsc::{Sender, channel, Receiver};
use uuid::Uuid;

use crate::devices::device::{DeviceRequest, DeviceResponse, Device, DeviceInfo, DeviceManagerCommand};
use crate::manager::ManagerInfo;
use crate::protocol::{AddressInfo, Command, MemoryDomain};

use crate::util::snes::{RomHeader};

const VERSION_DATA: &[u8] = "VERSION\n".as_bytes();
const STATUS_DATA: &[u8] = "GET_STATUS\n".as_bytes();
const RAM_CHECK_DATA: &[u8] = "READ_CORE_RAM 0 10\n".as_bytes();
const ROM_CHECK_DATA: &[u8] = "READ_CORE_MEMORY FFC0 32\n".as_bytes();

const CMD_SIZE: i64 = 512;

#[allow(dead_code)]
#[derive(Debug)]
pub struct Retroarch {
    id: Uuid,
    name: String,
    socket: UdpSocket,
    addr: String,
    ra_version: String,
    rom_access: bool,
    rom_name: String,
    rom_header: RomHeader,
}

#[derive(Debug)]
pub enum RaInfo {
    RaAdded { name: String, url: String, version: String, rom_access: bool, rom_name: String, rom_header: RomHeader },
    RaRemoved { name: String, url: String }
}

/* This implements the core of a Lua network device */
impl Retroarch {
    pub fn new(id: Uuid, socket: UdpSocket, addr: String, ra_version: &str, rom_access: bool, rom_name: &str, rom_header: RomHeader) -> Self
    {
        Self {
            id,
            name: format!("RetroArch {} ({})", ra_version, addr),
            socket,
            addr,
            ra_version: ra_version.to_string(),
            rom_access,
            rom_name: rom_name.to_string(),
            rom_header
        }
    }

    fn map_addresses(&self, addr_info: &[AddressInfo]) -> Result<Vec<AddressInfo>, Box<dyn std::error::Error + Sync + Send>> {
        let mut ra_addr_info = Vec::new();
        for addr in addr_info {
            match addr.domain {
                MemoryDomain::WRAM => ra_addr_info.push(addr.clone()),
                MemoryDomain::CARTRAM => ra_addr_info.push(addr.clone()),
                MemoryDomain::CARTROM => {
                    let snes_addr = self.rom_header.rom_mapping().to_snes(&addr)?;
                    let mut cur_snes_address = snes_addr;
                    let mut cur_address = addr.address;
                    let mut remaining_len = addr.size;
                    loop {
                        if (cur_snes_address >> 16) == ((cur_snes_address + remaining_len as i64) >> 16) {
                            /* If we're not wrapping banks, exit the loop and add this chunk */
                            ra_addr_info.push(AddressInfo { domain: addr.domain.clone(), address: cur_snes_address, orig_address: 0, size: remaining_len as i64 });
                            break;
                        } else {
                            /* This is a wrapping access and it needs to be split */
                            let split_size = 0x10000 - (cur_snes_address & 0xFFFF);
                            ra_addr_info.push(AddressInfo { domain: addr.domain.clone(), address: cur_snes_address, orig_address: 0, size: split_size });
                            cur_address += split_size;
                            
                            let mut cur_ai = addr.clone();
                            cur_ai.address = cur_address;

                            cur_snes_address = self.rom_header.rom_mapping().to_snes(&cur_ai)?;
                            remaining_len -= split_size;
                        }

                    }
                }
            }
        }

        Ok(ra_addr_info)
    }

    pub async fn read_stream(&mut self, tx: Sender<Vec<u8>>, data_size: usize, addr_info: &[AddressInfo]) -> Result<usize, Box<dyn std::error::Error + Sync + Send>> {
        let mut buf: Vec<u8> = vec![0u8; 16384];
        
        for addr in self.map_addresses(addr_info)? {
            
            let (mut target_addr, target_cmd) = match addr.domain {
                MemoryDomain::WRAM => (addr.address, "READ_CORE_RAM"),
                MemoryDomain::CARTRAM => (addr.address + 0x20000, "READ_CORE_RAM"),
                MemoryDomain::CARTROM => (if !self.rom_access { return Err("ROM Reads are not supported on this device".into()) } else { addr.address }, "READ_CORE_MEMORY")
            };

            let mut remaining_len = addr.size;            
            while remaining_len > 0 {
                let mut data: Vec<u8> = Vec::new();
                let cmd_size = if remaining_len > CMD_SIZE { CMD_SIZE } else { remaining_len };
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

    pub async fn write_stream(&mut self, rx: &mut Receiver<Vec<u8>>, data_size: usize, addr_info: &[AddressInfo]) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut recv_buf: Vec<u8> = Vec::new();
        let mut buf: Vec<u8> = vec![0u8; 16384];

        for addr in self.map_addresses(addr_info)? {
            let (mut target_addr, target_cmd) = match addr.domain {
                MemoryDomain::WRAM => (addr.address, "WRITE_CORE_RAM"),
                MemoryDomain::CARTRAM => (addr.address + 0x20000, "WRITE_CORE_RAM"),
                MemoryDomain::CARTROM => (if !self.rom_access { return Err("ROM Writes are not supported on this device".into()) } else { addr.address }, "WRITE_CORE_MEMORY")
            };

            let mut remaining_len = addr.size;
            while remaining_len > 0 {
                let cmd_size = if remaining_len > CMD_SIZE { CMD_SIZE } else { remaining_len };

                /* Get data until we have enough in our receive buffer */
                loop {
                    if recv_buf.len() >= cmd_size as usize {
                        break;
                    }
                    
                    if let Some(data) = rx.recv().await {
                        recv_buf.extend(data.iter());
                    } else {
                        return Err("Got zero data".into())
                    }                    
                }
            
                let rest = recv_buf.split_off(cmd_size as usize);
                let hex_str = recv_buf.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                recv_buf = rest;

                let _ = timeout(Duration::from_secs(1), self.socket.send(format!("{} {:X} {}\n", target_cmd, target_addr, hex_str).as_bytes())).await??;
                
                if addr.domain == MemoryDomain::CARTROM {
                    /* CARTROM writes can fail, so wait up to 100ms for an error response from RA */
                    let bytes = timeout(Duration::from_millis(100), self.socket.recv(&mut buf)).await;
                    if let Ok(Ok(bytes)) = bytes {
                        if String::from_utf8_lossy(&buf[..bytes]).contains("-1") {
                            return Err("Writes not allowed on this core".into());
                        }
                    }
                }
                
                remaining_len -= cmd_size;
                target_addr += cmd_size;
            }                        
        }

        Ok(data_size)
    }

    async fn start(id: Uuid, addr: String, ra_version: &str, rom_access: bool, rom_name: &str, rom_header: RomHeader, ra_manager_tx: Sender<(Uuid, String, DeviceInfo)>) -> Device {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let _ = socket.connect(&addr).await.unwrap();

        let mut ra = Retroarch::new(id, socket, addr.clone(), ra_version, rom_access, rom_name, rom_header);

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
                                        if let Err(result) = ra.read_stream(tx, data_size, &addr_info).await {
                                            let _ = ra_manager_tx.send((id, addr, DeviceInfo::ConnectionClosed(result.to_string()))).await;
                                            return;
                                        }
                                    },
                                    Command::PutAddress(addr_info) => {
                                        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
                                        let data_size = addr_info.iter().map(|ai| ai.size as usize).sum();


                                        let response = DeviceResponse::BinaryWriter((data_size, tx));
                                        sender.send(response).unwrap();
                                        if let Err(result) = ra.write_stream(&mut rx, data_size, &addr_info).await {
                                            let _ = ra_manager_tx.send((id, addr, DeviceInfo::ConnectionClosed(result.to_string()))).await;
                                            return;
                                        }

                                    },
                                    _ => sender.send(DeviceResponse::Empty).unwrap()
                                }
                            },
                            DeviceRequest::Close => {
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

pub struct RetroarchManager {}
impl RetroarchManager {
    pub async fn start(sender: Sender<ManagerInfo>) -> Sender<DeviceManagerCommand> {
        let (device_manager_tx, mut device_manager_rx) = tokio::sync::mpsc::channel(32);
        let mut devices = HashMap::new();

        tokio::spawn(async move {
            let (ra_manager_tx, mut ra_manager_rx) = tokio::sync::mpsc::channel(32);
            let (mut ra_info_rx, enumerator_tx) = RetroarchManager::start_ra_enumerator().await;

            loop {
                tokio::select! {
                    Some(message) = ra_info_rx.recv() => {
                        match message {
                            RaInfo::RaAdded { name: _, url, version, rom_access, rom_name, rom_header } => {
                                let id = Uuid::new_v4();
                                let device = Retroarch::start(id, url.to_string(), &version, rom_access, &rom_name, rom_header, ra_manager_tx.clone()).await;
                                sender.send(ManagerInfo::DeviceCreated(device.clone())).await.unwrap();
                                devices.insert(url.to_string(), device);
                            },
                            RaInfo::RaRemoved { name: _, url } => {
                                let device = &devices[&url];
                                let _ = device.sender.send(DeviceRequest::Close).await;
                                sender.send(ManagerInfo::DeviceRemoved(device.id)).await.unwrap();                                
                                devices.remove(&url);
                            }
                        }
                    },
                    Some((_, url, device_info)) = ra_manager_rx.recv() => {
                        match device_info {
                            DeviceInfo::ConnectionClosed(_reason) => {
                                if devices.contains_key(&url) {
                                    let old_device = &devices[&url];
                                    let _ = enumerator_tx.send(RaInfo::RaRemoved { name: old_device.name.clone(), url: url.clone()}).await;
                                    sender.send(ManagerInfo::DeviceRemoved(old_device.id)).await.unwrap();
                                    devices.remove(&url);
                                }
                            }
                        }
                    },
                    Some(message) = device_manager_rx.recv() => {
                        match message {
                            DeviceManagerCommand::Close => {
                                for (url, device) in &devices {
                                    let _ = device.sender.send(DeviceRequest::Close).await;
                                    let _ = enumerator_tx.send(RaInfo::RaRemoved { name: device.name.clone(), url: url.clone()}).await;
                                }
                                
                                devices.clear();

                                return;
                            }
                        }
                    }
                }
            }

        });

            
        device_manager_tx
    }

    async fn start_ra_enumerator() -> (Receiver<RaInfo>, Sender<RaInfo>) {
        let (ra_info_tx, ra_info_rx) = tokio::sync::mpsc::channel(32);
        let (enumerator_tx, mut enumerator_rx) = tokio::sync::mpsc::channel(32);
        let ra_address = "127.0.0.1:55355";
        let ra_addrs = vec![ra_address];

        tokio::spawn(async move {
            let mut current_devices = HashMap::new();
            loop {
                for ra_addr in &ra_addrs {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    if !current_devices.contains_key(&ra_addr.to_string()) {
                        if let Ok(ra_info) = RetroarchManager::try_ra_connect(ra_addr).await {
                            current_devices.insert(ra_addr.to_string(), ra_addr.to_string());
                            let _ = ra_info_tx.send(ra_info).await;
                        }
                    } else {
                        if RetroarchManager::check_ra_status(ra_addr).await.is_err() {
                            current_devices.remove(&ra_addr.to_string());
                            let _ = ra_info_tx.send(RaInfo::RaRemoved { name: String::default(), url: ra_addr.to_string() }).await;
                        }
                    }
                }

                match enumerator_rx.try_recv() {
                    Ok(msg) => {
                        match msg {
                            RaInfo::RaRemoved { name: _, url } => {
                                current_devices.remove(&url);
                            },
                            _ => ()
                        }
                    },
                    _ => ()
                }
            }
        });

        (ra_info_rx, enumerator_tx)
    }

    async fn try_ra_connect(ra_addr: &str) -> Result<RaInfo, Box<dyn std::error::Error + Send + Sync>> {


        let mut buf = vec![0u8; 65_507];

        let socket = UdpSocket::bind("127.0.0.1:0").await?;
        let _ = socket.send_to(VERSION_DATA, ra_addr).await?;
        let (len, _) = socket.recv_from(&mut buf).await?;

        let ra_version = String::from_utf8_lossy(&buf[..len]).trim().to_string();

        let _ = socket.send_to(STATUS_DATA, ra_addr).await?;
        let (len, _) = socket.recv_from(&mut buf).await?;

        let ra_status = String::from_utf8_lossy(&buf[..len]).trim().to_string();
        if ra_status.contains("CONTENTLESS") {
            return Err("Retroarch found, but no content loaded".into());
        }

        let _ = socket.send_to(RAM_CHECK_DATA, ra_addr).await?;
        let (len, _) = socket.recv_from(&mut buf).await?;
        let ram_check = String::from_utf8_lossy(&buf[..len]).to_string();
        if ram_check.contains("-1") {
            return Err("Retroarch found, but core returned -1 on READ_CORE_RAM".into());
        }

        let (rom_access, rom_name, rom_header) = {
            let _ = socket.send_to(ROM_CHECK_DATA, ra_addr).await?;
            let (len, _) = socket.recv_from(&mut buf).await?;
            let data = String::from_utf8_lossy(&buf[..len]).to_string();
            if !data.contains("-1") {
                let hex_str = String::from_utf8_lossy(&buf[22..len]).trim_end().to_string();
                let bin_data: Vec<u8> = hex_str.split(' ').into_iter().map(|h| u8::from_str_radix(h, 16).unwrap()).collect();
                let rom_name = String::from_utf8_lossy(&bin_data[..21]).to_string();
                if let Ok(rom_header) = RomHeader::from_le_bytes(&bin_data) {
                    (true, rom_name.to_string(), rom_header)  
                } else {
                    (false, String::default(), RomHeader::default())                    
                }                
            } else {
                (false, String::default(), RomHeader::default())
            }
        };

        Ok(RaInfo::RaAdded { name: format!("RetroArch {} ({})", ra_version, ra_addr), url: ra_addr.to_string(), version: ra_version, rom_access, rom_name, rom_header})
    }

    async fn check_ra_status(ra_addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut buf = vec![0u8; 65_507];
        let socket = UdpSocket::bind("127.0.0.1:0").await?;
        let _ = socket.send_to(STATUS_DATA, ra_addr).await?;
        let (len, _) = socket.recv_from(&mut buf).await?;

        let ra_status = String::from_utf8_lossy(&buf[..len]).to_string();
        if ra_status.contains("CONTENTLESS") {
            return Err("Retroarch found, but no content loaded".into());
        }

        Ok(())
    }
}