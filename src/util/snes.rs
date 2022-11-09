#![allow(dead_code)]
use crate::protocol::{AddressInfo, MemoryDomain};

#[derive(Debug, Clone, Copy)]
pub enum RomMapping {
    LoROM,
    HiROM,
    SDD1,
    SA1,
    ExHiROM,
    SPC7110,
    Unknown
}

impl RomMapping {
    pub fn to_snes(&self, address: &AddressInfo) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        let mapped_address = match self {
            RomMapping::LoROM => {
                match address.domain {
                    MemoryDomain::WRAM => 0x7E0000 + address.address,
                    MemoryDomain::CARTRAM => 0x700000 + (((address.address & 0xFF8000) << 1) | (address.address & 0x7FFF)),
                    MemoryDomain::CARTROM => (if address.address < 0x400000 { 0x800000 } else { 0 }) + (((address.address & 0xFF8000) << 1) | (address.address & 0x7FFF) | 0x8000),
                }
            },
            RomMapping::HiROM => {
                match address.domain {
                    MemoryDomain::WRAM => 0x7E0000 + address.address,
                    MemoryDomain::CARTRAM => {
                        let bank = (address.address / 0x2000) as i64 + 0xA0;
                        let saddr = 0x6000 | (address.address % 0x2000);
                        (bank << 16) | saddr
                    },
                    MemoryDomain::CARTROM => 0xC00000 + address.address,
                }
            },
            RomMapping::ExHiROM => {
                match address.domain {
                    MemoryDomain::WRAM => 0x7E0000 + address.address,
                    MemoryDomain::CARTRAM => {
                        let bank = (address.address / 0x2000) as i64 + 0xA0;
                        let saddr = 0x6000 | (address.address % 0x2000);
                        (bank << 16) | saddr
                    },
                    MemoryDomain::CARTROM => (if address.address < 0x400000 { 0xC00000 } else { 0 }) + address.address,
                }
            },
            _ => return Err("Unsupported RomMapping".into())
        };

        Ok(mapped_address)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomChip {
    ROM,
    RAM,
    CoProcessor,
    Battery,
    DSP,
    GSU,
    OBC1,
    SA1,
    SDD1,
    SRTC,
    Other,
    Custom,
    Unknown
}

#[derive(Debug, Clone, Copy)]
pub enum RomSpeed {
    Slow,
    Fast
}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct RomHeader {
    game_title: [u8; 21],
    rom_mapping: u8,
    rom_chipset: u8,
    rom_size: u8,
    sram_size: u8,
    pub developer_id: u8,
    pub country: u8,
    pub version: u8,
    pub checksum_comp: u16,
    pub checksum: u16
}

impl RomHeader {
    pub fn from_le_bytes(data: &[u8]) -> Result<RomHeader, Box<dyn std::error::Error + Send + Sync>> {
        let rom_header = unsafe { std::ptr::read_unaligned::<RomHeader>(data.as_ptr() as * const RomHeader) };
        if rom_header.checksum_comp ^ 0xFFFF != rom_header.checksum {
            return Err("Invalid header".into())
        } else {
            Ok(rom_header)
        }
    }

    pub fn game_title(&self) -> String {
        String::from_utf8_lossy(&self.game_title).to_string()
    }

    pub fn rom_size(&self) -> i64 {
        1 << self.rom_size
    }

    pub fn sram_size(&self) -> i64 {
        1 << self.sram_size
    }

    pub fn rom_mapping(&self) -> RomMapping {
        match self.rom_mapping & 0b111 {
            0 => RomMapping::LoROM,
            1 => RomMapping::HiROM,
            2 => RomMapping::SDD1,
            3 => RomMapping::SA1, 
            5 => RomMapping::ExHiROM,
            10 => RomMapping::SPC7110,
            _ => RomMapping::Unknown
        }
    }

    pub fn rom_speed(&self) -> RomSpeed {
        if self.rom_mapping & 0x10 == 0 { RomSpeed::Slow } else { RomSpeed::Fast }
    }

    pub fn chipset(&self) -> Vec<RomChip> {
        let mut base = match self.rom_chipset & 0x0F {
            0 => vec![RomChip::ROM],
            1 => vec![RomChip::ROM, RomChip::RAM],
            2 => vec![RomChip::ROM, RomChip::RAM, RomChip::Battery],
            3 => vec![RomChip::CoProcessor, RomChip::ROM],
            4 => vec![RomChip::CoProcessor, RomChip::ROM, RomChip::RAM],
            5 => vec![RomChip::CoProcessor, RomChip::ROM, RomChip::RAM, RomChip::Battery],
            6 => vec![RomChip::CoProcessor, RomChip::Battery],
            _ => vec![RomChip::Unknown]
        };

        let mut cop = match self.rom_chipset & 0xF0 {
            0x00 => vec![RomChip::DSP],
            0x10 => vec![RomChip::GSU],
            0x20 => vec![RomChip::OBC1],
            0x30 => vec![RomChip::SA1],
            0x40 => vec![RomChip::SDD1],
            0x50 => vec![RomChip::SRTC],
            0xE0 => vec![RomChip::Other],
            0xF0 => vec![RomChip::Custom],
            _ => vec![RomChip::Unknown]
        };

        if base.contains(&RomChip::CoProcessor) {
            base.append(&mut cop);
        }
        
        base

    }

}

