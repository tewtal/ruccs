use futures::stream::ReadyChunks;
/* USB2SNES Protocol Definitions */
use serde::{Serialize, Deserialize};
use std::str::FromStr;
use strum_macros::EnumString;
use bitflags::bitflags;

#[allow(dead_code, non_camel_case_types)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Opcode {
    GET,
    PUT,
    VGET,
    VPUT,

    // file system operations
    LS,
    MKDIR,
    RM,
    MV,

    // special operations
    RESET,
    BOOT,
    POWER_CYCLE,
    INFO,
    MENU_RESET,
    STREAM,
    TIME,

    // response
    RESPONSE,

    //
    NONE
}

impl Opcode {
    pub fn from_request(req: &Request) -> Opcode {
        match &req.command {
            Command::Info => Opcode::INFO,
            Command::Boot(_) => Opcode::BOOT,
            Command::Menu => Opcode::MENU_RESET,
            Command::Reset => Opcode::RESET,
            Command::Stream => Opcode::STREAM,
            Command::GetAddress(a) => if a.len() == 1 { Opcode::GET } else { Opcode::VGET },
            Command::PutAddress(a) => if a.len() == 1 { Opcode::PUT } else { Opcode::VPUT },
            Command::GetFile(_) => Opcode::GET,
            Command::PutFile(_) => Opcode::PUT,
            Command::List(_) => Opcode::LS,
            Command::Remove(_) => Opcode::RM,
            Command::Rename(_) => Opcode::MV,
            Command::MakeDir(_) => Opcode::MKDIR,
            _ => Opcode::NONE
        }
    }
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Debug, PartialEq, EnumString, Clone, Copy)]
pub enum Space {
    FILE,
    SNES,
    MSU,
    CMD,
    CONFIG,   
}

bitflags! {
    pub struct Flags: u8 {
        const NONE = 0;
        const SKIPRESET = 1;
        const ONLYRESET = 2;
        const CLRX = 4;
        const SETX = 8;
        const STREAM_BURST = 16;
        const NORESP = 64;
        const DATA64B = 128;
    }    
}
impl Flags {
    pub fn from_str(str: &str) -> Result<Flags, Box<dyn std::error::Error + Send + Sync>>
    {
        Ok(match str {
            "NONE" => Flags::NONE,
            "SKIPRESET" => Flags::SKIPRESET,
            "ONLYRESET" => Flags::ONLYRESET,
            "CLRX" => Flags::CLRX,
            "SETX" => Flags::SETX,
            "STREAM_BURST" => Flags::STREAM_BURST,
            "NORESP" => Flags::NORESP,
            "DATA64B" => Flags::DATA64B,
            _ => Err("Invalid flag")?
        })
    }
}

bitflags! {
    pub struct InfoFlags: u8 {
        const FEAT_NONE = 0;
        const FEAT_DSPX = 1;
        const FEAT_ST0010 = 2;
        const FEAT_SRTC = 4;
        const FEAT_MUS1 = 8;
        const FEAT_213F = 16;
        const FEAT_CMD_UNLOCK = 32;
        const FEAT_USB1 = 64;
        const FEAT_DMA1 = 128;
    }
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Debug)]
pub enum FileType {
    DIRECTORY = 0,
    FILE = 1
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Debug)]
pub enum MemoryDomain {
    WRAM,
    CARTROM,
    CARTRAM
}

#[derive(Debug)]
pub struct AddressInfo {
    pub domain: MemoryDomain,
    pub address: i64,
    pub orig_address: i64,
    pub size: i64
}

impl AddressInfo {
    pub fn create(address: &str, size: &str) -> Result<AddressInfo, Box<dyn std::error::Error + Send + Sync>> {
        let address_int = i64::from_str_radix(address, 16)?;
        let size_int = i64::from_str_radix(size, 16)?;
        Ok(match address_int {
            a if a < 0xE00000 => AddressInfo { domain: MemoryDomain::CARTROM, orig_address: a, address: a, size: size_int },
            a if a >= 0xE00000 && a < 0xF50000 => AddressInfo { domain: MemoryDomain::CARTRAM, orig_address: a, address: a - 0xE00000, size: size_int },
            a => AddressInfo { domain: MemoryDomain::WRAM, orig_address: a, address: a - 0xF50000, size: size_int },
        })
    }

    pub fn from_operands(operands: &[String]) -> Result<Vec<AddressInfo>, Box<dyn std::error::Error + Send + Sync>> {
        operands.chunks(2).map(|o| AddressInfo::create(&o[0], &o[1])).collect()
    }
}

#[derive(Debug)]
pub enum Command {
    DeviceList,
    Attach(String),
    AppVersion,
    Name(String),
    Close,
    Info,
    Boot(String),
    Menu,
    Reset,
    Binary,
    Stream,
    Fence,
    GetAddress(Vec<AddressInfo>),
    PutAddress(Vec<AddressInfo>),
    PutIPS,
    GetFile(String),
    PutFile((String, usize)),
    List(String),
    Remove(String),
    Rename((String, String)),
    MakeDir(String)
}

#[derive(Debug)]
pub struct Request {
    pub command: Command,
    pub space: Space,
    pub flags: Option<Flags>
}

impl Request {
    pub fn from_wsrequest(wr: &WSRequest) -> Result<Request, Box<dyn std::error::Error + Send + Sync>> {
        let operands = wr.operands.as_deref().unwrap_or_default();
        let command = match wr.opcode.as_str() {
            "DeviceList" => Command::DeviceList,
            "Attach" if operands.len() == 1 => Command::Attach(operands[0].to_string()),
            "AppVersion" => Command::AppVersion,
            "Name" if operands.len() == 1 => Command::Name(operands[0].to_string()),
            "Close" => Command::Close,
            "Info" => Command::Info,
            "Boot" if operands.len() == 1 => Command::Boot(operands[0].to_string()),
            "Menu" => Command::Menu,
            "Reset" => Command::Reset,
            "Binary" => Command::Binary,
            "Stream" => Command::Stream,
            "Fence" => Command::Fence,
            "GetAddress" if operands.len() >= 2 => Command::GetAddress(AddressInfo::from_operands(operands)?),
            "PutAddress" if operands.len() >= 2 => Command::PutAddress(AddressInfo::from_operands(operands)?),
            "PutIPS" => Command::PutIPS,
            "GetFile" if operands.len() == 1 => Command::GetFile(operands[0].to_string()),
            "PutFile" if operands.len() == 2 => Command::PutFile((operands[0].to_string(), usize::from_str_radix(&operands[1], 16)?)),
            "List" if operands.len() == 1 => Command::List(operands[0].to_string()),
            "Remove" if operands.len() == 1 => Command::Remove(operands[0].to_string()),
            "Rename" if operands.len() == 2 => Command::Rename((operands[0].to_string(), operands[1].to_string())),
            "MakeDir" if operands.len() == 1 => Command::MakeDir(operands[0].to_string()),
            _ => Err("Invalid Opcode")?
        };

        let req = Request {
            command,
            space: Space::from_str(&wr.space)?,
            flags: match &wr.flags {
                Some(fl) => fl.iter().map(|f| Flags::from_str(f).unwrap_or(Flags::NONE)).reduce(|a, b| a | b),                
                None => None
            }
        };

        Ok(req)
    }
}

#[derive(Deserialize)]
pub struct WSRequest {
    #[serde(rename="Opcode")]
    opcode: String,
    #[serde(rename="Space")]
    space: String,
    #[serde(rename="Flags")]
    flags: Option<Vec<String>>,
    #[serde(rename="Operands")]
    operands: Option<Vec<String>>
}

#[derive(Serialize)]
pub struct WSResponse {
    #[serde(rename="Results")]
    pub results: Vec<String>
}