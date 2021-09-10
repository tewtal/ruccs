use std::collections::HashMap;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::Receiver;
use tokio_serial::{self, SerialPortInfo, SerialStream};
use serialport::SerialPortType;
use tokio::sync::mpsc::{Sender, channel};
use uuid::Uuid;


use crate::devices::device::{DeviceRequest, DeviceResponse, Device, DeviceInfo, DeviceManagerCommand};
use crate::manager::ManagerInfo;
use crate::protocol::{self, Space, Flags, Opcode};



#[allow(dead_code)]
pub struct SD2Snes {
    id: Uuid,
    port_name: String,
    stream: SerialStream,
    name: String,
    block_size: usize
}

impl SD2Snes {
    
    fn pad_or_truncate(&self, data: &[u8]) -> Vec<u8> {
        self.pad_or_truncate_size(self.block_size, data)
    }

    fn pad_or_truncate_size(&self, block_size: usize, data: &[u8]) -> Vec<u8> {
        let pad_size = ((data.len() as f64 / block_size as f64).ceil() as usize) * block_size;
        if data.len() < pad_size {
            let mut padded_data = data.to_vec();
            padded_data.extend(vec![0u8; pad_size - data.len()]);
            padded_data
        } else {
            data[..pad_size].to_vec()
        }
    }

    pub async fn write_data(&mut self, data: &[u8]) {
        let padded_data = self.pad_or_truncate(data);
        for chunk in padded_data.chunks(self.block_size) {
            self.stream.write_all(chunk).await.unwrap(); // Error handling
        }
    }

    pub async fn read_data(&mut self, len: usize) -> Vec<u8> {
        let mut buf = Vec::with_capacity(len);
        let mut read_len = 0;
        while read_len < len {
            let buf_len = self.stream.read_buf(&mut buf).await.unwrap();
            read_len += buf_len;
        }
        buf
    }

    pub async fn send_command(&mut self, opcode: Opcode, space: Space, flags: Flags, _args: &[u8]) -> Vec<u8> {
        let header = self.pad_or_truncate_size(256, &vec![b'U', b'S', b'B', b'A', opcode as u8, space as u8, flags.clone() as u8]);
        let data = self.pad_or_truncate(&header);
        
        if (flags as u8) & (Flags::NORESP as u8) != 0 {
            Vec::new()
        } else {
            self.write_data(&data).await;
            let response = self.read_data(512).await;
            response    
        }
    }

    pub async fn get_information(&mut self) -> Vec<String> {
        let data = self.send_command(Opcode::INFO, Space::SNES, Flags::NONE, &vec![]).await;
        let str_data = String::from_utf8_lossy(&data[16..]).to_string();
        let split_data: Vec<&str> = str_data.split_terminator('\0').filter(|s| s != &"").collect();
        let mut info = Vec::new();                                                                                
        info.push(split_data[1][4..].to_string());
        let v = (i64::from(data[256]) << 24) | (i64::from(data[257]) << 16) | (i64::from(data[258]) << 8) | i64::from(data[259]);
        info.push(format!("{:X}", v));
        info.push(split_data[0].to_string());
        let f = data[6];
        let mut infoflags = Vec::new();
        if f & 0x01 != 0 { infoflags.push("FEAT_DSPX".to_string()); }
        if f & 0x02 != 0 { infoflags.push("FEAT_ST0010".to_string()); }
        if f & 0x04 != 0 { infoflags.push("FEAT_SRTC".to_string()); }
        if f & 0x08 != 0 { infoflags.push("FEAT_MSU1".to_string()); }
        if f & 0x10 != 0 { infoflags.push("FEAT_213F".to_string()); }
        if f & 0x20 != 0 { infoflags.push("FEAT_CMD_UNLOCK".to_string()); }
        if f & 0x40 != 0 { infoflags.push("FEAT_USB1".to_string()); }
        if f & 0x80 != 0 { infoflags.push("FEAT_DMA1".to_string()); }
        info.push(infoflags.join("|"));
        info.push(split_data[2].to_string());
        info
    }

    pub fn new(id: Uuid, port_name: &str, stream: SerialStream) -> Self
    {
        Self {
            id,
            port_name: port_name.to_string(),
            stream,
            name: format!("SD2SnesDevice {} {}", port_name, id),
            block_size: 512
        }
    }

    async fn start(id: Uuid, port_name: &str, _snes_manager_tx: Sender<(String, DeviceInfo)>) -> Device {
        let stream = tokio_serial::SerialStream::open(&tokio_serial::new(port_name, 115200)).unwrap();
        let mut sd2snes = SD2Snes::new(id, port_name, stream);

        /* The main communication channel for this specific device, all clients will have to go through this to use the device */
        let (device_tx, mut device_rx) = channel(128); // (Not sure about what's a reasonable size here)

        let device = Device {          
            id,  
            name: sd2snes.name.to_string(),
            sender: device_tx
        };

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(cmd) = device_rx.recv() => {
                        match cmd {
                            DeviceRequest::Request { req, resp: sender } => {
                                match req.command {
                                    protocol::Command::Info => {
                                        let response = DeviceResponse::Strings(sd2snes.get_information().await);                                    
                                        sender.send(response).unwrap()
                                    },
                                    protocol::Command::GetAddress(addr_info) => {
                                        let (tx, rx) = tokio::sync::mpsc::channel(32);                                        
                                        let response = DeviceResponse::BinaryReader((addr_info[0].size as usize, rx));
                                        sender.send(response).unwrap();
                                        tx.send(vec![0u8; addr_info[0].size as usize]).await.unwrap();                                        
                                    }
                                    _ => sender.send(DeviceResponse::Nothing).unwrap()
                                }                                
                            },
                            DeviceRequest::Close => {
                                let _ = sd2snes.stream.shutdown().await;
                                break
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

pub enum PortInfo {
    PortAdded(String),
    PortRemoved(String)
}

pub struct SD2SnesManager {}
impl SD2SnesManager {
    pub async fn start(sender: Sender<ManagerInfo>) -> Sender<DeviceManagerCommand> {
        let (device_manager_tx, mut device_manager_rx) = tokio::sync::mpsc::channel(32);
        let mut devices: HashMap<String, Device> = HashMap::new();
        
        tokio::spawn(async move {
            let (snes_manager_tx, mut snes_manager_rx) = tokio::sync::mpsc::channel(32);
            
            /* Start a new task that sits in a sleep-loop and looks for new ports, sending us a message
               when a new device is found, or an existing device is removed */
            let mut port_info_rx = SD2SnesManager::start_port_enumerator().await;
    
            /* Start the main message handling loop for this device manager */
            loop {
                tokio::select! {
                    Some(message) = port_info_rx.recv() => {
                        match message {
                            PortInfo::PortAdded(port_name) => {
                                let id = Uuid::new_v4();
                                let device = SD2Snes::start(id, &port_name, snes_manager_tx.clone()).await;
                                sender.send(ManagerInfo::DeviceCreated(device.clone())).await.unwrap();
                                devices.insert(port_name, device);
                            },
                            PortInfo::PortRemoved(port_name) => {
                                let device = &devices[&port_name];
                                sender.send(ManagerInfo::DeviceRemoved(device.id)).await.unwrap();
                                devices.remove(&port_name);
                            }
                        }
                    },
                    Some((port_name, message)) = snes_manager_rx.recv() => {
                        match message {
                            DeviceInfo::ConnectionClosed(_reason) => {
                                let device = &devices[&port_name];
                                sender.send(ManagerInfo::DeviceRemoved(device.id)).await.unwrap();
                                devices.remove(&port_name);
                            }
                        }
                    },
                    Some(message) = device_manager_rx.recv() => {
                        match message {
                            DeviceManagerCommand::Close => {
                                for (_, device) in &devices {
                                    device.sender.send(DeviceRequest::Close).await.unwrap();
                                    sender.send(ManagerInfo::DeviceRemoved(device.id)).await.unwrap();
                                }
                            }
                        }
                    }
                }
            }
        });

        device_manager_tx
    }
    
    async fn start_port_enumerator() -> Receiver<PortInfo> {
        let (port_tx, port_rx) = tokio::sync::mpsc::channel(32);
        
        tokio::spawn(async move {
            let mut current_ports: Vec<SerialPortInfo> = Vec::new();
            loop {
                /* Check every 100ms for changed ports */
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                /* Get all serial ports and check if any port matches the USB2SNES vid/pid */
                let available_ports: Vec<SerialPortInfo> = tokio_serial::available_ports().unwrap().drain(..).filter(|p| match &p.port_type {
                    SerialPortType::UsbPort(usb_info) if usb_info.vid == 4617 && usb_info.pid == 23074 => true,
                    _ => false
                }).collect();

                let removed_ports = current_ports.iter().filter(|p| !available_ports.contains(p));
                let added_ports = available_ports.iter().filter(|p| !current_ports.contains(p));

                for removed in removed_ports {
                    let _ = port_tx.send(PortInfo::PortRemoved(removed.port_name.to_string())).await;
                }

                for added in added_ports {
                    let _ = port_tx.send(PortInfo::PortAdded(added.port_name.to_string())).await;
                }

                current_ports = available_ports;
            }
        });

        port_rx
    }
}