/* USB2SNES Protocol Definitions */
use serde::{Serialize, Deserialize};
use std::str::FromStr;
use strum_macros::EnumString;

#[allow(dead_code, non_camel_case_types)]
#[derive(Debug, Clone)]
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
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Debug, PartialEq, EnumString, Clone)]
pub enum Space {
    FILE,
    SNES,
    MSU,
    CMD,
    CONFIG,   
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Debug, PartialEq, EnumString, Clone)]
pub enum Flags {
    NONE = 0,
    SKIPRESET = 1,
    ONLYRESET = 2,
    CLRX = 4,
    SETX = 8,
    STREAM_BURST = 16,
    NORESP = 64,
    DATA64B = 128,    
}

#[allow(dead_code, non_camel_case_types)]
pub enum InfoFlag {
    FEAT_DSPX = 1,
    FEAT_ST0010 = 2,
    FEAT_SRTC = 4,
    FEAT_MUS1 = 8,
    FEAT_213F = 16,
    FEAT_CMD_UNLOCK = 32,
    FEAT_USB1 = 64,
    FEAT_DMA1 = 128
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
    pub size: i64
}

impl AddressInfo {
    pub fn create(address: &str, size: &str) -> Result<AddressInfo, Box<dyn std::error::Error + Send + Sync>> {
        let address_int = i64::from_str_radix(address, 16)?;
        let size_int = i64::from_str_radix(size, 16)?;
        Ok(match address_int {
            a if a < 0xE00000 => AddressInfo { domain: MemoryDomain::CARTROM, address: a, size: size_int },
            a if a >= 0xE00000 && a < 0xF50000 => AddressInfo { domain: MemoryDomain::CARTRAM, address: a - 0xE00000, size: size_int },
            a => AddressInfo { domain: MemoryDomain::WRAM, address: a - 0xF50000, size: size_int },
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
    PutFile((String, i64)),
    List(String),
    Remove(String),
    Rename((String, String)),
    MakeDir(String)
}

#[derive(Debug)]
pub struct Request {
    pub command: Command,
    pub space: Space,
    pub flags: Option<Vec<Flags>>
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
            "PutFile" if operands.len() == 2 => Command::PutFile((operands[0].to_string(), i64::from_str(&operands[1])?)),
            "List" if operands.len() == 1 => Command::List(operands[0].to_string()),
            "Remove" if operands.len() == 1 => Command::Remove(operands[0].to_string()),
            "Rename" if operands.len() == 2 => Command::Rename((operands[0].to_string(), operands[1].to_string())),
            "MakeDir" if operands.len() == 1 => Command::MakeDir(operands[0].to_string()),
            _ => Err("Invalid Opcode".to_string())?
        };

        let req = Request {
            command,
            space: Space::from_str(&wr.space)?,
            flags: match &wr.flags {
                Some(fl) => Some(fl.iter().map(|f| Flags::from_str(f)).collect::<Result<Vec<_>,_>>()?),
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