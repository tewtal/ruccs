use tokio::net::{TcpStream};
use std::collections::HashMap;

use tokio::sync::mpsc::{Sender, channel, Receiver};
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use uuid::Uuid;

use crate::devices::device::{DeviceRequest, DeviceResponse, Device, DeviceInfo, DeviceManagerCommand};
use crate::manager::ManagerInfo;
use crate::protocol::{Command, MemoryDomain, self, AddressInfo};


#[allow(dead_code)]
pub struct EmuNwa {
    id: Uuid,
    name: String,
    stream: TcpStream,
}

pub enum EmuInfo {
    EmuAdded { name: String, port: u16, address: String },
    EmuRemoved { name: String, port: u16 }
}

#[derive(Debug)]
pub enum CommandResponse {
    Ascii(Vec<HashMap<String, String>>),
    Binary(usize),
    None
}

/* This implements the core of a Lua network device */
impl EmuNwa {
    pub fn new(id: Uuid, stream: TcpStream, name: &str) -> Self
    {
        Self {
            id,
            stream,
            name: name.to_string()
        }
    }

    pub fn domain_to_str(domain: &protocol::MemoryDomain) -> String {
        match domain {
            MemoryDomain::CARTROM => "CARTROM",
            MemoryDomain::CARTRAM => "SRAM",
            MemoryDomain::WRAM => "WRAM"
        }.to_string()
    }

    pub async fn send_command(stream: &mut TcpStream, command: &str, args: Option<&[&str]>, response: bool) -> Result<CommandResponse, Box<dyn std::error::Error + Send + Sync>> {
        /* Prepare and send command */
        let data = match args {
            Some(args) => format!("{} {}\n", command, args.join(";")),
            None => format!("{}\n", command)
        };

        let _ = stream.write(data.as_bytes()).await?;

        match response {
            true => EmuNwa::read_response(stream).await,
            false => Ok(CommandResponse::None)
        }
    }

    pub async fn read_response(stream: &mut TcpStream) -> Result<CommandResponse, Box<dyn std::error::Error + Send + Sync>> {
        match stream.read_u8().await? {
            0 => Ok(CommandResponse::Binary(stream.read_u32().await? as usize)),
            10 => {
                /* Read command into string */
                let mut buf = vec![0; 1024];
                let mut response = String::new();
                
                loop {
                    let bytes = stream.read(&mut buf).await?;
                    response.push_str(&String::from_utf8_lossy(&buf[..bytes]));
                    if (response.len() == 1 && response.ends_with('\n')) || (response.len() >= 2 && response.ends_with("\n\n")) {
                        break;
                    }
                }   

                Ok(CommandResponse::Ascii(EmuNwa::parse_ascii_response(&response)?))
            },
            _ => Err("Invalid response - First byte of response is neither null or newline".into())
        }
    }


    pub fn parse_ascii_response(response: &str) -> Result<Vec<HashMap<String, String>>, Box<dyn std::error::Error + Send + Sync>>  {
        if !response.ends_with('\n') {
            return Err("Invalid response - Does not end with newline".into())
        }
        
        let mut list = Vec::new();
        let mut lines = HashMap::new();

        for line in response.split('\n').filter(|l| !l.trim().is_empty()).map(|l| l.splitn(2, ':',).collect::<Vec<_>>()) {
            if line.len() != 2 {
                return Err("Invalid response - Line does not have a key and a value".into());
            }

            let (key, value) = (line[0].to_string(), line[1].to_string());
            
            if lines.contains_key(&key) {
                list.push(lines);
                lines = HashMap::new();
            }

            lines.insert(key, value);
        }

        if !lines.is_empty() { list.push(lines); }

        Ok(list)
    }

    async fn read_stream(&mut self, tx: Sender<Vec<u8>>, size: usize) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut read_len = 0;
        while read_len < size {
            let mut buf = vec![0u8; size - read_len];
            let bytes = self.stream.read(&mut buf).await?;
            let _ = tx.send(buf[..bytes].to_vec()).await;
            read_len += bytes;
        }

        Ok(read_len)
    }

    async fn write_stream(&mut self, rx: &mut Receiver<Vec<u8>>, size: usize) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        self.stream.write_u8(0).await?;
        self.stream.write_u32(size as u32).await?;

        let mut write_len = 0;
        while write_len < size {
            if let Some(data) = rx.recv().await {
                self.stream.write_all(&data).await?;
                write_len += data.len();
            } else {
                break;
            }
        }

        Ok(write_len)
    }

    async fn handle_info_request(&mut self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        
        let emulator_info = match EmuNwa::send_command(&mut self.stream, "EMULATOR_INFO", None, true).await {
            Ok(CommandResponse::Ascii(r)) => {
                let h = r.first().ok_or("Empty response list")?;
                Ok(h.clone())
            }
            _ => Err("Unexpected info response")
        }?;

        let _emulation_status = match EmuNwa::send_command(&mut self.stream, "EMULATION_STATUS", None, true).await {
            Ok(CommandResponse::Ascii(r)) => {
                let h = r.first().ok_or("Empty response list")?;
                Ok(h.clone())
            },
            _ => Err("Unexpected status response")
        }?;        

        let game_info = match EmuNwa::send_command(&mut self.stream, "GAME_INFO", None, true).await {
            Ok(CommandResponse::Ascii(r)) => {
                let h = r.first().ok_or("Empty response list")?;
                Ok(h.clone())
            },
            _ => Err("Unexpected game info response")
        }?;

        Ok(vec![
            format!("{} v{} (EmuNWA v{})", emulator_info.get("name").ok_or("No name tag")?, emulator_info.get("version").ok_or("No version tag")?, emulator_info.get("nwa_version").ok_or("No nwa_version tag")?),
            "0".to_string(),
            game_info.get("file").ok_or("No file tag")?.to_string(),
            "".to_string(),
        ])
    }

    async fn handle_write_request(&mut self, rx: &mut Receiver<Vec<u8>>, ai: &AddressInfo) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let domain = EmuNwa::domain_to_str(&ai.domain).to_string();
        let address = ai.address.to_string();
        let size = ai.size.to_string();
        let args = vec![domain.as_str(), address.as_str(), size.as_str()];
        
        let _ = EmuNwa::send_command(&mut self.stream, "bCORE_WRITE", Some(&args), false).await?;
        let written_bytes = self.write_stream(rx, ai.size as usize).await?;
        let _ = EmuNwa::read_response(&mut self.stream).await?;

        Ok(written_bytes)
    }

    async fn handle_read_requests(&mut self, tx: Sender<Vec<u8>>, requests: &Vec<(MemoryDomain, Vec<String>)>) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut read_size = 0;

        /* Send all requests without reading responses */
        for req in requests {
            let _ = EmuNwa::send_command(&mut self.stream, "CORE_READ", Some(&req.1.iter().map(|s| &**s).collect::<Vec<_>>()), false).await?;
        }

        /* Read the combined responses */
        for _ in requests {
            let ltx = tx.clone();
            let response = EmuNwa::read_response(&mut self.stream).await?;
            
            match response {
                CommandResponse::Binary(s) if s == 0 => { return Err("Invalid Read Size".into()) },
                CommandResponse::Binary(s) => read_size += self.read_stream(ltx, s as usize).await?,
                _ => return Err("Invalid response".into())
            }
        }

        Ok(read_size)
    }

    async fn start(id: Uuid, port: u16, stream: TcpStream, name: &str, emunwa_manager_tx: Sender<(Uuid, u16, DeviceInfo)>) -> Device {
        stream.set_nodelay(true).unwrap();
        let mut emunwa = EmuNwa::new(id, stream, name);

        /* The main communication channel for this specific device, all clients will have to go through this to use the device */
        let (device_tx, mut device_rx) = channel(1); 

        let device = Device {          
            id,  
            name: emunwa.name.to_string(),
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
                                        let info_strings = emunwa.handle_info_request().await;
                                        if let Ok(info) = info_strings {
                                            let _ = sender.send(DeviceResponse::Strings(info));
                                        } else {
                                            let _ = emunwa.stream.shutdown().await;
                                            let _ = emunwa_manager_tx.send((id, port, DeviceInfo::ConnectionClosed("Info request failed".into()))).await;
                                            return;
                                        }
                                    },
                                    Command::GetAddress(addr_info) => {
                                        let (tx, rx) = tokio::sync::mpsc::channel(32);
                                        let data_size = addr_info.iter().map(|ai| ai.size as usize).sum();

                                        let response = DeviceResponse::BinaryReader((data_size, rx));
                                        sender.send(response).unwrap();

                                        let mut requests = Vec::new();
                                        let mut request = (addr_info[0].domain.clone(), vec![EmuNwa::domain_to_str(&addr_info[0].domain)]);

                                        /* Group the same memory domain into a single request */
                                        for ai in addr_info {
                                            if ai.domain != request.0 {
                                                requests.push(request);
                                                request = (ai.domain.clone(), vec![EmuNwa::domain_to_str(&ai.domain)]);
                                            }

                                            let address = format!("{}", ai.address);
                                            let size = format!("{}", ai.size);
                                            request.1.push(address);
                                            request.1.push(size);
                                        }
                                        requests.push(request);

                                        if let Err(e) = emunwa.handle_read_requests(tx.clone(), &requests).await {
                                            let _ = emunwa.stream.shutdown().await;
                                            let _ = emunwa_manager_tx.send((id, port, DeviceInfo::ConnectionClosed(e.to_string()))).await;
                                            return;  
                                        }

                                        tx.closed().await;

                                    },
                                    Command::PutAddress(addr_info) => {
                                        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
                                        let data_size = addr_info.iter().map(|ai| ai.size as usize).sum();

                                        let response = DeviceResponse::BinaryWriter((data_size, tx));
                                        sender.send(response).unwrap();

                                        for ai in &addr_info {
                                            if let Err(e) = emunwa.handle_write_request(&mut rx, ai).await {
                                                let _ = emunwa.stream.shutdown().await;
                                                let _ = emunwa_manager_tx.send((id, port, DeviceInfo::ConnectionClosed(e.to_string()))).await;
                                                return;  
                                            }
                                        }
                                    },
                                    Command::Boot(path) => {
                                        if let Err(e) = EmuNwa::send_command(&mut emunwa.stream, "LOAD_GAME", Some(&[&path]), true).await {
                                            let _ = emunwa.stream.shutdown().await;
                                            let _ = emunwa_manager_tx.send((id, port, DeviceInfo::ConnectionClosed(e.to_string()))).await;
                                            return;                                             
                                        }

                                        let _ = sender.send(DeviceResponse::Empty);
                                    },
                                    Command::Reset => {
                                        if let Err(e) = EmuNwa::send_command(&mut emunwa.stream, "EMULATION_RESET", None, true).await {
                                            let _ = emunwa.stream.shutdown().await;
                                            let _ = emunwa_manager_tx.send((id, port, DeviceInfo::ConnectionClosed(e.to_string()))).await;
                                            return;                                             
                                        }

                                        let _ = sender.send(DeviceResponse::Empty);
                                    }
                                    _ => sender.send(DeviceResponse::Empty).unwrap()
                                }
                            },
                            DeviceRequest::Close => {
                                let _ = emunwa.stream.shutdown().await;
                                let _ = emunwa_manager_tx.send((id, port, DeviceInfo::ConnectionClosed("Device Closure Requested".into()))).await;
                                return;
                            },
                        }    
                    },
                    else => break        
                }
            }
        });

        device
    }
}

pub struct EmuNwaManager {}
impl EmuNwaManager {
    pub async fn start(sender: Sender<ManagerInfo>) -> Sender<DeviceManagerCommand> {
        let (device_manager_tx, mut device_manager_rx) = tokio::sync::mpsc::channel(32);
        let mut devices = HashMap::new();
        
        tokio::spawn(async move {
            let (emunwa_manager_tx, mut emunwa_manager_rx) = tokio::sync::mpsc::channel(32);
            let (mut emu_info_rx, enumerator_tx) = EmuNwaManager::start_emunwa_enumerator().await;
    
            loop 
            {
                tokio::select! {
                    Some(message) = emu_info_rx.recv() => {
                        match message {
                            EmuInfo::EmuAdded { name, port, address } => {
                                let id = Uuid::new_v4();
                                let stream = tokio::net::TcpStream::connect(address).await;
                                if let Ok(stream) = stream {
                                    let device = EmuNwa::start(id, port, stream, &name, emunwa_manager_tx.clone()).await;
                                    sender.send(ManagerInfo::DeviceCreated(device.clone())).await.unwrap();
                                    devices.insert(port, device);
                                }                                
                            },
                            EmuInfo::EmuRemoved { name: _ , port } => {
                                if devices.contains_key(&port) {
                                    let device = &devices[&port];
                                    sender.send(ManagerInfo::DeviceRemoved(device.id)).await.unwrap();
                                    let _ = enumerator_tx.send(EmuInfo::EmuRemoved { name: device.name.clone(), port}).await;
                                    devices.remove(&port);
                                }
                            }
                        }
                    },
                    Some((id, port, device_info)) = emunwa_manager_rx.recv() => {
                        match device_info {
                            DeviceInfo::ConnectionClosed(_reason) => {
                                sender.send(ManagerInfo::DeviceRemoved(id)).await.unwrap();
                                /* As long as the device on this port wasn't removed by the manager, let's try to reconnect */
                                if devices.contains_key(&port) {
                                    let old_device = &devices[&port];
                                    let id = Uuid::new_v4();
                                    let stream = tokio::net::TcpStream::connect(("localhost", port)).await;
                                    if let Ok(stream) = stream {
                                        let device = EmuNwa::start(id, port, stream, &old_device.name, emunwa_manager_tx.clone()).await;
                                        sender.send(ManagerInfo::DeviceCreated(device.clone())).await.unwrap();
                                        devices.remove(&port);
                                        devices.insert(port, device);
                                    } else {
                                        /* If it's not possible to reconnect, remove the device from this port and wait for it to be enumerated again */
                                        let _ = enumerator_tx.send(EmuInfo::EmuRemoved { name: old_device.name.clone(), port}).await;
                                        devices.remove(&port);
                                    }
                                }
                            }
                        }
                    },
                    Some(message) = device_manager_rx.recv() => {
                        match message {
                            DeviceManagerCommand::Close => {
                                /* Close everything  */
                                return;
                            }
                        }
                    }
                }
            }
        });

        device_manager_tx
    }

    async fn start_emunwa_enumerator() -> (Receiver<EmuInfo>, Sender<EmuInfo>) {
        let (emu_tx, emu_rx) = tokio::sync::mpsc::channel(32);
        let (enumerator_tx, mut enumerator_rx) = tokio::sync::mpsc::channel(32);
        let base_port = 0xbeef;
        tokio::spawn(async move {
            let mut current_emus: HashMap<u16, String> = HashMap::new();
            loop {
                for p in base_port..base_port+8 {
                    /* Check every 500ms for an open port */
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    if !current_emus.contains_key(&p) {
                        let stream = tokio::net::TcpStream::connect(("localhost", p)).await;
                        if let Ok(mut stream) = stream {
                            let emulator_info = EmuNwa::send_command(&mut stream, "EMULATOR_INFO", None, true).await;
                            if let Ok(CommandResponse::Ascii(emulator_info)) = emulator_info {
                                /* We got an emulator info response back, let's check if it makes sense */
                                let emulator_info = &emulator_info[0];
                                if emulator_info.contains_key("name") {
                                    current_emus.insert(p, format!("{} (127.0.0.1:{})", emulator_info["name"], emulator_info["id"]));
                                    let _ = emu_tx.send(EmuInfo::EmuAdded { name: current_emus[&p].to_string(), port: p, address: format!("localhost:{}", p) }).await;
                                }
                            }

                            /* Shutdown stream so we don't keep multiple open connections to emunwa hosts */
                            let _ = stream.shutdown().await;
                        }
                    }

                    /* Check if we got a message from the manager that this port has been disabled */
                    if let Ok(EmuInfo::EmuRemoved { name: _, port }) = enumerator_rx.try_recv() {
                        current_emus.remove(&port);
                    }
                }
            }
        });

        (emu_rx, enumerator_tx)
    }

}