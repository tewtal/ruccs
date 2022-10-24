use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::Receiver;
use tokio_serial::{self, SerialPortInfo, SerialStream};
use serialport::{SerialPort, SerialPortType};
use tokio::sync::mpsc::{Sender, channel};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use crate::devices::device::{DeviceRequest, DeviceResponse, Device, DeviceInfo, DeviceManagerCommand};
use crate::manager::ManagerInfo;
use crate::protocol::{self, AddressInfo, Flags, Opcode, Space};



#[allow(dead_code)]
pub struct SD2Snes {
    id: Uuid,
    port_name: String,
    stream: SerialStream,
    name: String,
    block_size: usize
}

pub enum CommandArg<'a> {
    AddressList(&'a Vec<AddressInfo>),
    Filename(String),
    FilenameAndSize((String, usize)),
    Filenames((String, String)),
    None
}

impl SD2Snes {

    pub fn new(id: Uuid, port_name: &str, stream: SerialStream) -> Self
    {
        Self {
            id,
            port_name: port_name.to_string(),
            stream,
            name: format!("SD2SNES {}", port_name),
            block_size: 512
        }
    }
    
    pub fn pad_size(&self, size: usize, block_size: usize) -> usize {
        ((size as f64 / block_size as f64).ceil() as usize) * block_size
    }

    fn pad_or_truncate(&self, data: &[u8]) -> Vec<u8> {
        self.pad_or_truncate_size(self.block_size, data)
    }

    fn pad_or_truncate_size(&self, block_size: usize, data: &[u8]) -> Vec<u8> {
        let pad_size = self.pad_size(data.len(), block_size);
        if data.len() < pad_size {
            let mut padded_data = data.to_vec();
            padded_data.extend(vec![0u8; pad_size - data.len()]);
            padded_data
        } else {
            let truncate_size = ((data.len() as f64 / block_size as f64).floor() as usize) * block_size;
            data[..truncate_size].to_vec()
        }
    }

    /* Attempts to read from the device until there's nothing left to read
       so that the read buffer gets cleared out
     */
    pub fn clear_read_buffer(&mut self) {
        let mut tmp = vec![0u8; 512];
        let mut clear = false;
        while !clear {
            clear = self.stream.try_read(&mut tmp).is_err();
        }
    }

    /* Writes data to the sd2snes in <block_size> chunks */
    pub async fn write_data(&mut self, data: &[u8], block_size: usize) {
        for chunk in data.chunks(block_size) {
            self.stream.write_all(chunk).await.unwrap();
        }

        let _ = self.stream.flush().await;
    }

    /* Reads <len> bytes of data from the sd2snes */
    pub async fn read_data(&mut self, len: usize) -> Vec<u8> {
        let mut read_len = 0;        
        let mut data = Vec::new();
        
        while read_len < len {
            let read_size = if (len - read_len) < 4096 { len - read_len } else { 4096 };
            let mut buf = vec![0u8; read_size];
            let bytes = self.stream.read(&mut buf).await.unwrap();
            if bytes > 0 {
                read_len += bytes;
                data.extend(buf);        
            }
        }

        data
    }

    /* Streams <len> bytes of data from the sd2snes to a channel sender */
    /* This ignores any errors caused by the remote channel not being available, so that the correct */
    /* amount of data is always read from the sd2snes */
    pub async fn read_stream(&mut self, tx: Sender<Vec<u8>>, padded_size: usize, data_size: usize) -> usize {
        let mut read_len = 0;
                
        while read_len < padded_size {
            let read_size = if (padded_size - read_len) < 4096 { padded_size - read_len } else { 4096 };
            let mut buf = vec![0u8; read_size];
            let bytes = self.stream.read(&mut buf).await.unwrap();
            println!("Read Size: {:?}, Read Len: {:?}, Bytes: {:?}, Data Size: {:?}", read_size, read_len, bytes, data_size);
            if bytes > 0 {
                if read_len > data_size {
                    /* We've already sent the relevant data to the target, do nothing and read more from the device */
                } else if (read_len + bytes) > data_size {
                    let _ = tx.send(buf[..(bytes - ((read_len + bytes) - data_size))].to_vec()).await;
                } else {
                    let _ = tx.send(buf[..bytes].to_vec()).await;
                }
            }
        }

        /* Wait for receiver to finish reading fully before resuming and potentially dropping the sender */
        tx.closed().await;

        read_len
    }

    /* Streams <data_size> bytes read from the receiver, with padding added to <padded_size> */
    pub async fn write_stream(&mut self, rx: &mut Receiver<Vec<u8>>, padded_size: usize, data_size: usize, block_size: usize) -> usize {
        let mut write_len = 0;

        /* Write all data coming from the websocket channel */
        while write_len < data_size {
            if let Some(data) = rx.recv().await {
                self.write_data(&data, block_size).await;
                write_len += data.len();
            } else {
                /* If there's an error receiving data on the channel, we break and then let the padding fill to the correct amount */
                break;
            }
        }

        /* Write padding if needed */
        if write_len < padded_size {
            let padding = vec![0u8; padded_size - write_len];
            self.stream.write_all(&padding).await.unwrap();
            write_len += padding.len();
        }

        write_len
    }

    /* Prepares and sends a command to the sd2snes from the given set of parameters */
    pub async fn send_command(&mut self, opcode: Opcode, space: Space, flags: Flags, args: CommandArg<'_>) -> Option<Vec<u8>> {        
        let mut buf = self.pad_or_truncate_size(256, &[b'U', b'S', b'B', b'A', opcode as u8, space as u8, flags.bits()]);

        match (space, opcode, args) {
            (Space::SNES | Space::CMD | Space::MSU, Opcode::GET | Opcode::PUT, CommandArg::AddressList(addr_list)) => {
                let (addr, size) = (addr_list[0].orig_address, addr_list[0].size);
                buf[252] = ((size >> 24) & 0xFF) as u8;
                buf[253] = ((size >> 16) & 0xFF) as u8;
                buf[254] = ((size >> 8) & 0xFF) as u8;
                buf[255] = (size & 0xFF) as u8;

                buf = self.pad_or_truncate(&buf);
                buf[256] = ((addr >> 24) & 0xFF) as u8;
                buf[257] = ((addr >> 16) & 0xFF) as u8;
                buf[258] = ((addr >> 8) & 0xFF) as u8;
                buf[259] = (addr & 0xFF) as u8;
            },
            (Space::SNES | Space::CMD | Space::MSU, Opcode::VGET | Opcode::VPUT, CommandArg::AddressList(addr_list)) => {
                buf = buf[..64].to_vec();
                for (i, ai) in addr_list.iter().enumerate() {
                    buf[32 + (i*4)] = (ai.size & 0xFF) as u8;
                    buf[33 + (i*4)] = ((ai.orig_address >> 16) & 0xFF) as u8;
                    buf[34 + (i*4)] = ((ai.orig_address >> 8) & 0xFF) as u8;
                    buf[35 + (i*4)] = (ai.orig_address & 0xFF) as u8;                    
                }
            },
            (Space::FILE, Opcode::GET | Opcode::LS | Opcode::MKDIR | Opcode::RM | Opcode::BOOT, CommandArg::Filename(path)) => {
                buf.extend(path.as_bytes());
                buf = self.pad_or_truncate(&buf);
            },
            (Space::FILE, Opcode::MV, CommandArg::Filenames((from, to))) => {
                buf.extend(from.as_bytes());
                to.as_bytes().iter().enumerate().for_each(|(i, b)| buf[8 + i] = *b);
                buf = self.pad_or_truncate(&buf);
            },
            (Space::FILE, Opcode::PUT, CommandArg::FilenameAndSize((path, size))) => {
                buf.extend(path.as_bytes());                
                buf[252] = ((size >> 24) & 0xFF) as u8;
                buf[253] = ((size >> 16) & 0xFF) as u8;
                buf[254] = ((size >> 8) & 0xFF) as u8;
                buf[255] = (size & 0xFF) as u8;
                buf = self.pad_or_truncate(&buf);
            },
            _ => { buf = self.pad_or_truncate(&buf); }
        }

        /* Read out any remaining data from the device */
        self.clear_read_buffer();
        
        /* Send command to sd2snes */
        self.write_data(&buf, buf.len()).await;

        if flags.contains(Flags::NORESP) {
            None
        } else {
            Some(self.read_data(512).await)
        }
    }

    /* Sends and parses an "Info" request */
    pub async fn get_information(&mut self) -> Vec<String> {
        let data = self.send_command(Opcode::INFO, Space::SNES, Flags::NONE, CommandArg::None).await.unwrap();
        let str_data = String::from_utf8_lossy(&data[16..]).to_string();
        let split_data: Vec<&str> = str_data.split_terminator('\0').filter(|s| s != &"").collect();        
        let mut info = vec![split_data[1][6..].to_string()];
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

    async fn start(id: Uuid, port_name: &str, _snes_manager_tx: Sender<(String, DeviceInfo)>) -> Device {
        let mut stream = tokio_serial::SerialStream::open(&tokio_serial::new(port_name, 921600)).unwrap();
        stream.write_data_terminal_ready(true).unwrap();
        stream.set_flow_control(serialport::FlowControl::None).unwrap();

        let mut sd2snes = SD2Snes::new(id, port_name, stream);

        /* The main communication channel for this specific device, all clients will have to go through this to use the device */
        /* The channel buffer is set to 1 since that's the maximum limit of commands the device can handle at a time. */
        let (device_tx, mut device_rx) = channel(1); 

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
                                let opcode = Opcode::from_request(&req);
                                let block_size = if opcode == Opcode::VGET || opcode == Opcode::VPUT { 64 } else { 512 };

                                match req.command {
                                    protocol::Command::Info => {
                                        let response = DeviceResponse::Strings(sd2snes.get_information().await);
                                        let _ = sender.send(response);
                                    },
                                    protocol::Command::Menu |
                                    protocol::Command::Reset => {
                                        let _ = sd2snes.send_command(opcode, req.space, req.flags.unwrap_or(Flags::NONE), CommandArg::None).await;
                                        let _ = sender.send(DeviceResponse::Empty);
                                    },
                                    protocol::Command::Remove(path) |
                                    protocol::Command::MakeDir(path) |
                                    protocol::Command::Boot(path) => {
                                        let _ = sd2snes.send_command(opcode, Space::FILE, req.flags.unwrap_or(Flags::NONE), CommandArg::Filename(path)).await;
                                        let _ = sender.send(DeviceResponse::Empty);
                                    },
                                    protocol::Command::Rename(paths) => {
                                        let _ = sd2snes.send_command(opcode, Space::FILE, req.flags.unwrap_or(Flags::NONE), CommandArg::Filenames(paths)).await;
                                        let _ = sender.send(DeviceResponse::Empty);
                                    },
                                    protocol::Command::List(path) => {
                                        let _ = sd2snes.send_command(opcode, Space::FILE, req.flags.unwrap_or(Flags::NONE) | Flags::NORESP, CommandArg::Filename(path)).await;
                                        let mut ls_type = 0u8;
                                        let mut filelist = Vec::new();
                                        while ls_type != 0xFF {
                                            let (mut ptr, data) = (0, sd2snes.read_data(512).await);
                                            while ptr < data.len() {
                                                ls_type = data[ptr];
                                                match ls_type {
                                                    0 | 1 => {
                                                        let mut f_idx = 0;
                                                        while data[ptr + 1 + f_idx] != 0 { f_idx += 1; }
                                                        let filename = String::from_utf8_lossy(&data[(ptr + 1)..(ptr + 1 + f_idx)]).to_string();
                                                        filelist.push(vec![ls_type.to_string(), filename]);
                                                        ptr += f_idx + 2;
                                                    },
                                                    _ => {
                                                        break;
                                                     }
                                                }
                                            }
                                        }
                                        let response = DeviceResponse::Strings(filelist.drain(..).flatten().collect());
                                        let _ = sender.send(response);                                        
                                    }
                                    protocol::Command::GetAddress(addr_info) => {
                                        let start = Instant::now();
                                        
                                        /* Send a response back directly with a binary channel that will be used to push data that 
                                           gets read from the sd2snes */                                           
                                        let (tx, rx) = tokio::sync::mpsc::channel(32);

                                        /* Calculate the total size and padded size of all requests */
                                        let (data_size, padded_size) = if opcode == Opcode::GET { 
                                            (addr_info[0].size as usize, sd2snes.pad_size(addr_info[0].size as usize, 512))
                                        } else {
                                            (addr_info.iter().map(|a| a.size as usize).sum(), (addr_info.iter().map(|a| sd2snes.pad_size(a.size as usize, 64)).sum()))
                                        };
                                        
                                        /* Create and send response */
                                        let response = DeviceResponse::BinaryReader((data_size, rx));
                                        sender.send(response).unwrap();

                                        /* Set flags depending on opcode and so on, NORESP is set to not have to care about a response since it's redundant */
                                        let flags = req.flags.unwrap_or(Flags::NONE) | Flags::NORESP | if opcode == Opcode::VGET { Flags::DATA64B } else { Flags::NONE };
                                        
                                        /* Send command to device to begin sending data */
                                        let _ = sd2snes.send_command(opcode, req.space, flags, CommandArg::AddressList(&addr_info)).await;

                                        /* Read data to the end */
                                        let _ = sd2snes.read_stream(tx, padded_size, data_size).await;
                                        
                                        let elapsed = start.elapsed();
                                        let kbps = ((data_size as f64) / 1024f64) / elapsed.as_secs_f64();
                                        println!("GetAddress Complete: Sent {:?}({:?} padded) bytes in {:?}ms ({:?}kB/s)", data_size, padded_size, elapsed.as_millis(), kbps);

                                    },
                                    protocol::Command::PutAddress(addr_info) => {
                                        /* Send a response back directly with a binary channel that will be used to write data to the sd2snes */
                                        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

                                         /* Calculate the total size and padded size of all requests */
                                        let (data_size, padded_size) = if opcode == Opcode::PUT { 
                                            (addr_info[0].size as usize, sd2snes.pad_size(addr_info[0].size as usize, block_size))
                                        } else {
                                            (addr_info.iter().map(|a| a.size as usize).sum(), sd2snes.pad_size(addr_info.iter().map(|a| a.size as usize).sum(), block_size))
                                        };                                       

                                        /* Create and send response */
                                        let response = DeviceResponse::BinaryWriter((data_size, tx));
                                        sender.send(response).unwrap();

                                        /* Set flags depending on opcode and so on, NORESP is set to not have to care about a response since it's redundant */
                                        let flags = req.flags.unwrap_or(Flags::NONE) | Flags::NORESP | if opcode == Opcode::VPUT { Flags::DATA64B } else { Flags::NONE };
                                        
                                        /* Send command to device to begin reading data */
                                        let _ = sd2snes.send_command(opcode, req.space, flags, CommandArg::AddressList(&addr_info)).await;

                                        /* Start reading from the input stream and write to the sd2snes */
                                        let result = sd2snes.write_stream(&mut rx, padded_size, data_size, block_size).await;

                                        println!("PutAddress Complete: Got {:?}({:?}) - Sent: {:?}", padded_size, data_size, result);
                                    },
                                    protocol::Command::GetFile(path) => {
                                        let (tx, rx) = tokio::sync::mpsc::channel(32);
                                        let response = sd2snes.send_command(opcode, Space::FILE, req.flags.unwrap_or(Flags::NONE), CommandArg::Filename(path)).await.unwrap();
                                        let data_size = (response[252] as usize) << 24 | (response[253] as usize) << 16 | (response[254] as usize) << 8 | response[255] as usize;
                                        let padded_size = sd2snes.pad_size(data_size, block_size);
                                        println!("GetFile response: Size: {:?}/{:?}", data_size, padded_size);

                                        /* Create and send response */
                                        let response = DeviceResponse::FileReader((data_size, rx));

                                        /* Don't fail if sender is broken, we now have to read the file from the sd2snes even though the receiver is disconnected */
                                        let _ = sender.send(response).unwrap();

                                        /* Read data to the end */
                                        let result = sd2snes.read_stream(tx, padded_size, data_size).await;                                        
                                        println!("GetFile Complete: Sent {:?}({:?}) - Received: {:?}", padded_size, data_size, result);
                                    },
                                    protocol::Command::PutFile((path, data_size)) => {
                                        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
                                        let padded_size = sd2snes.pad_size(data_size, block_size);
                                     
                                        /* Create and send response */
                                        let response = DeviceResponse::BinaryWriter((data_size, tx));
                                        sender.send(response).unwrap();

                                        /* Send command to device to begin reading data */
                                        let _ = sd2snes.send_command(opcode, Space::FILE, req.flags.unwrap_or(Flags::NONE) | Flags::NORESP, CommandArg::FilenameAndSize((path, data_size))).await;

                                        /* Start reading from the input stream and write to the sd2snes */
                                        let result = sd2snes.write_stream(&mut rx, padded_size, data_size, block_size).await;

                                        println!("PutFile Complete: Got {:?}({:?}) - Sent: {:?}", padded_size, data_size, result);                                        
                                    }

                                    _ => sender.send(DeviceResponse::Empty).unwrap()
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
                                for device in devices.values() {
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
                /* Check every 500ms for changed ports */
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                /* Get all serial ports and check if any port matches the USB2SNES vid/pid */
                let available_ports: Vec<SerialPortInfo> = tokio_serial::available_ports().unwrap().drain(..).filter(|p| match &p.port_type {
                    SerialPortType::UsbPort(usb_info) if usb_info.vid == 4617 && usb_info.pid == 23074 => {
                        if cfg!(target_os = "macos") {
                            /* 
                                MacOS is a bit dumb and will generate two serial devices for each USB-serial (one cu.XXX and one tty.XXX).
                                This makes sure we only get one of the devices.                            
                            */
                            if p.port_name.starts_with("/dev/cu.") { true } else { false }
                        } else {
                            true
                        }
                    },
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