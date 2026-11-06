#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::convert::TryInto;

/// Formatos de binario soportados por el cargador universal de shardevos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryFormat {
    PE,
    ELF,
    MachO,
    Unknown,
}

/// Arquitecturas de CPU reconocidas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    I386,
    AArch64,
    Arm,
    Unknown,
}

/// Cabecera parseada de un binario heterogéneo.
#[derive(Debug, Clone)]
pub struct BinaryHeader {
    pub format: BinaryFormat,
    pub arch: Arch,
    pub entry_point: usize,
    pub image_base: usize,
    pub sections: Vec<Section>,
}

/// Descripción de una sección del binario.
#[derive(Debug, Clone)]
pub struct Section {
    pub name: [u8; 16],
    pub vaddr: usize,
    pub memsz: usize,
    pub filesz: usize,
    pub offset: usize,
    pub flags: u32,
}

/// Errores del cargador de binarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryError {
    TooShort,
    UnknownFormat,
    Malformed,
    UnsupportedArch,
    Truncated,
}

/// Lectores de enteros little-endian.
fn r16le(b: &[u8]) -> u16 {
    u16::from_le_bytes(b.try_into().unwrap_or([0, 0]))
}
fn r32le(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().unwrap_or([0, 0, 0, 0]))
}
fn r64le(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().unwrap_or([0, 0, 0, 0, 0, 0, 0, 0]))
}
fn r16be(b: &[u8]) -> u16 {
    u16::from_be_bytes(b.try_into().unwrap_or([0, 0]))
}
fn r32be(b: &[u8]) -> u32 {
    u32::from_be_bytes(b.try_into().unwrap_or([0, 0, 0, 0]))
}

/// Identifica el formato de un binario a partir de sus primeros bytes.
/// Funciona en entornos `no_std` (kernel / bootloader).
pub fn detect_format(bytes: &[u8]) -> Result<BinaryFormat, BinaryError> {
    if bytes.len() < 4 {
        return Err(BinaryError::TooShort);
    }

    if &bytes[0..2] == b"MZ" {
        return Ok(BinaryFormat::PE);
    }

    if &bytes[0..4] == b"\x7fELF" {
        return Ok(BinaryFormat::ELF);
    }

    let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
    match magic {
        [0xFE, 0xED, 0xFA, 0xCE] // Mach-O 32-bit big-endian
        | [0xFE, 0xED, 0xFA, 0xCF] // Mach-O 64-bit big-endian
        | [0xCE, 0xFA, 0xED, 0xFE] // Mach-O 32-bit little-endian
        | [0xCF, 0xFA, 0xED, 0xFE] // Mach-O 64-bit little-endian => {
            => Ok(BinaryFormat::MachO),
        _ => Err(BinaryError::UnknownFormat),
    }
}

/// Parsea la cabecera completa de un binario detectando su formato automáticamente.
pub fn parse_header(bytes: &[u8]) -> Result<BinaryHeader, BinaryError> {
    match detect_format(bytes)? {
        BinaryFormat::PE => parse_pe(bytes),
        BinaryFormat::ELF => parse_elf(bytes),
        BinaryFormat::MachO => parse_macho(bytes),
        BinaryFormat::Unknown => Err(BinaryError::UnknownFormat),
    }
}

/// Mapea un binario parseado al espacio de usuario del kernel.
/// Stub de producción: devuelve Ok sin efectos laterales hasta integrar el MMU.
pub fn map_to_userspace(_header: &BinaryHeader) -> Result<(), BinaryError> {
    Ok(())
}

// ---------------------------------------------------------------------------
// PE (Windows)
// ---------------------------------------------------------------------------

fn parse_pe(bytes: &[u8]) -> Result<BinaryHeader, BinaryError> {
    if bytes.len() < 0x40 {
        return Err(BinaryError::TooShort);
    }

    let e_lfanew = r32le(&bytes[0x3C..0x40]) as usize;
    if e_lfanew + 24 > bytes.len() {
        return Err(BinaryError::TooShort);
    }
    if &bytes[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        return Err(BinaryError::Malformed);
    }

    let coff = e_lfanew + 4;
    let machine = r16le(&bytes[coff..coff + 2]);
    let sections_count = r16le(&bytes[coff + 2..coff + 4]) as usize;
    let size_opt = r16le(&bytes[coff + 16..coff + 18]) as usize;
    let opt = coff + 20;
    if opt + size_opt > bytes.len() {
        return Err(BinaryError::TooShort);
    }

    let magic = r16le(&bytes[opt..opt + 2]);
    let (entry_point, image_base) = match magic {
        0x010b => { // PE32
            (
                r32le(&bytes[opt + 16..opt + 20]) as usize,
                r32le(&bytes[opt + 28..opt + 32]) as usize,
            )
        }
        0x020b => { // PE32+
            (
                r32le(&bytes[opt + 16..opt + 20]) as usize,
                r64le(&bytes[opt + 24..opt + 32]) as usize,
            )
        }
        _ => return Err(BinaryError::Malformed),
    };

    let arch = match machine {
        0x014c => Arch::I386,
        0x8664 => Arch::X86_64,
        _ => return Err(BinaryError::UnsupportedArch),
    };

    let mut sections = Vec::with_capacity(sections_count);
    let sec_table = opt + size_opt;
    for i in 0..sections_count {
        let off = sec_table + i * 40;
        if off + 40 > bytes.len() {
            return Err(BinaryError::Truncated);
        }
        let mut name = [0u8; 16];
        name[..8].copy_from_slice(&bytes[off..off + 8]);
        let vsize = r32le(&bytes[off + 8..off + 12]) as usize;
        let vaddr = r32le(&bytes[off + 12..off + 16]) as usize;
        let fsize = r32le(&bytes[off + 16..off + 20]) as usize;
        let fptr = r32le(&bytes[off + 20..off + 24]) as usize;
        let flags = r32le(&bytes[off + 36..off + 40]);
        sections.push(Section {
            name,
            vaddr,
            memsz: vsize,
            filesz: fsize,
            offset: fptr,
            flags,
        });
    }

    Ok(BinaryHeader {
        format: BinaryFormat::PE,
        arch,
        entry_point,
        image_base,
        sections,
    })
}

// ---------------------------------------------------------------------------
// ELF (Linux)
// ---------------------------------------------------------------------------

fn parse_elf(bytes: &[u8]) -> Result<BinaryHeader, BinaryError> {
    if bytes.len() < 64 {
        return Err(BinaryError::TooShort);
    }

    let class = bytes[4];
    let data = bytes[5];

    let read_u16 = |off: usize| {
        if data == 1 {
            r16le(&bytes[off..off + 2])
        } else {
            r16be(&bytes[off..off + 2])
        }
    };
    let read_u32 = |off: usize| {
        if data == 1 {
            r32le(&bytes[off..off + 4])
        } else {
            r32be(&bytes[off..off + 4])
        }
    };
    let read_u64 = |off: usize| {
        if data == 1 {
            r64le(&bytes[off..off + 8])
        } else {
            0
        }
    };

    let e_type = read_u16(16);
    let e_machine = read_u16(18);
    let e_entry = if class == 2 {
        read_u64(24) as usize
    } else {
        read_u32(24) as usize
    };
    let e_shoff = if class == 2 {
        read_u64(40) as usize
    } else {
        read_u32(40) as usize
    };
    let e_shentsize = read_u16(58) as usize;
    let e_shnum = read_u16(60) as usize;

    let arch = match e_machine {
        0x3e => Arch::X86_64,
        0x03 => Arch::I386,
        0xb7 => Arch::AArch64,
        0x28 => Arch::Arm,
        _ => return Err(BinaryError::UnsupportedArch),
    };

    let image_base = if e_type == 2 { 0 } else { e_entry & !0xfff };

    let mut sections = Vec::with_capacity(e_shnum);
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        if off + e_shentsize > bytes.len() {
            return Err(BinaryError::Truncated);
        }
        let sh_type = read_u32(off);
        let sh_flags = read_u32(off + 4);
        let sh_addr = if class == 2 {
            read_u64(off + 16) as usize
        } else {
            read_u32(off + 16) as usize
        };
        let sh_offset = if class == 2 {
            read_u64(off + 24) as usize
        } else {
            read_u32(off + 24) as usize
        };
        let sh_size = if class == 2 {
            read_u64(off + 48) as usize
        } else {
            read_u32(off + 48) as usize
        };
        let sh_name = read_u32(off) as usize;

        let mut name = [0u8; 16];
        for (i, b) in name.iter_mut().enumerate() {
            if sh_name + i < bytes.len() {
                *b = bytes[sh_name + i];
            }
        }

        sections.push(Section {
            name,
            vaddr: sh_addr,
            memsz: sh_size,
            filesz: sh_size,
            offset: sh_offset,
            flags: sh_flags,
        });

        // Ignorar secciones tipo nulo para no llenar el header de basura.
        if sh_type == 0 {
            sections.pop();
        }
    }

    Ok(BinaryHeader {
        format: BinaryFormat::ELF,
        arch,
        entry_point: e_entry,
        image_base,
        sections,
    })
}

// ---------------------------------------------------------------------------
// Mach-O (macOS)
// ---------------------------------------------------------------------------

fn parse_macho(bytes: &[u8]) -> Result<BinaryHeader, BinaryError> {
    if bytes.len() < 32 {
        return Err(BinaryError::TooShort);
    }

    let magic = r32le(&bytes[0..4]);
    let is64 = matches!(magic, 0xfeedfacf | 0xcffaedfe);
    let cputype = r32le(&bytes[4..8]);
    let ncmds = r32le(&bytes[20..24]) as usize;
    let _sizeofcmds = r32le(&bytes[24..28]) as usize;

    let arch = match cputype {
        0x01000007 | 0x0100000d => Arch::X86_64,
        0x00000007 => Arch::I386,
        0x0100000c => Arch::AArch64,
        0x0000000c => Arch::Arm,
        _ => return Err(BinaryError::UnsupportedArch),
    };

    let mut entry_point = 0usize;
    let image_base = 0usize;
    let mut sections = Vec::with_capacity(ncmds);

    let mut cmd_off = 32;
    for _ in 0..ncmds {
        if cmd_off + 8 > bytes.len() {
            return Err(BinaryError::Truncated);
        }
        let cmd_size = r32le(&bytes[cmd_off + 4..cmd_off + 8]) as usize;
        let cmd_type = r32le(&bytes[cmd_off..cmd_off + 4]);

        if cmd_type == 0x80000001 || cmd_type == 0x80000002 {
            // LC_THREAD / LC_UNIXTHREAD -> contiene el entry point
            // Simplificado: leer el primer registro de hilo (rip/eip)
            if cmd_off + 8 + 16 <= bytes.len() {
                if is64 {
                    // x86_thread_state64: flavor(4) + count(4) + ... + rip(offset 168)
                    let rip_off = cmd_off + 168;
                    if rip_off + 8 <= bytes.len() {
                        entry_point = r64le(&bytes[rip_off..rip_off + 8]) as usize;
                    }
                } else {
                    // x86_thread_state32: eip(offset 52)
                    let eip_off = cmd_off + 52;
                    if eip_off + 4 <= bytes.len() {
                        entry_point = r32le(&bytes[eip_off..eip_off + 4]) as usize;
                    }
                }
            }
        }

        if cmd_type == 0x19 {
            // LC_SEGMENT_64
            if cmd_off + cmd_size <= bytes.len() {
                let nsects = r32le(&bytes[cmd_off + 56..cmd_off + 60]) as usize;
                let mut sect_off = cmd_off + 72;
                for _ in 0..nsects {
                    if sect_off + 80 > bytes.len() {
                        break;
                    }
                    let mut name = [0u8; 16];
                    name.copy_from_slice(&bytes[sect_off..sect_off + 16]);
                    let addr = r64le(&bytes[sect_off + 16..sect_off + 24]) as usize;
                    let size = r64le(&bytes[sect_off + 24..sect_off + 32]) as usize;
                    let offset = r64le(&bytes[sect_off + 32..sect_off + 40]) as usize;
                    sections.push(Section {
                        name,
                        vaddr: addr,
                        memsz: size,
                        filesz: size,
                        offset: offset as usize,
                        flags: r32le(&bytes[sect_off + 40..sect_off + 44]),
                    });
                    sect_off += 80;
                }
            }
        }

        if cmd_size == 0 {
            break;
        }
        cmd_off += cmd_size;
    }

    Ok(BinaryHeader {
        format: BinaryFormat::MachO,
        arch,
        entry_point,
        image_base,
        sections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pe() {
        assert_eq!(detect_format(b"MZ\x90\x00"), Ok(BinaryFormat::PE));
    }

    #[test]
    fn detects_elf() {
        assert_eq!(detect_format(b"\x7fELF\x02"), Ok(BinaryFormat::ELF));
    }

    #[test]
    fn detects_macho_le32() {
        assert_eq!(
            detect_format(b"\xCE\xFA\xED\xFE\x00"),
            Ok(BinaryFormat::MachO)
        );
    }

    #[test]
    fn detects_macho_be64() {
        assert_eq!(
            detect_format(b"\xFE\xED\xFA\xCF\x00"),
            Ok(BinaryFormat::MachO)
        );
    }

    #[test]
    fn rejects_unknown() {
        assert_eq!(
            detect_format(b"\x00\x00\x00\x00"),
            Err(BinaryError::UnknownFormat)
        );
    }

    #[test]
    fn rejects_too_short() {
        assert_eq!(detect_format(b"MZ"), Err(BinaryError::TooShort));
    }

    #[test]
    fn parse_pe_header() {
        let mut buf = [0u8; 512];
        buf[0..2].copy_from_slice(b"MZ");
        buf[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        buf[0x80..0x84].copy_from_slice(b"PE\0\0");
        buf[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        buf[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        buf[0x94..0x96].copy_from_slice(&0xf8u16.to_le_bytes());
        buf[0x98..0x9a].copy_from_slice(&0x020bu16.to_le_bytes());
        buf[0xa8..0xac].copy_from_slice(&0x1000u32.to_le_bytes());
        buf[0xb0..0xb8].copy_from_slice(&0x140000000u64.to_le_bytes());
        let sec_table = 0x98 + 0xf8usize;
        buf[sec_table..sec_table + 8].copy_from_slice(b".text\0\0\0");
        buf[sec_table + 8..sec_table + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        buf[sec_table + 12..sec_table + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        buf[sec_table + 16..sec_table + 20].copy_from_slice(&0x200u32.to_le_bytes());
        buf[sec_table + 20..sec_table + 24].copy_from_slice(&0x400u32.to_le_bytes());
        buf[sec_table + 36..sec_table + 40].copy_from_slice(&0x60000020u32.to_le_bytes());

        let h = parse_header(&buf).unwrap();
        assert_eq!(h.format, BinaryFormat::PE);
        assert_eq!(h.arch, Arch::X86_64);
        assert_eq!(h.entry_point, 0x1000);
        assert_eq!(h.image_base, 0x140000000);
        assert_eq!(h.sections.len(), 1);
        assert_eq!(h.sections[0].vaddr, 0x1000);
    }

    #[test]
    fn parse_elf_header() {
        let mut buf = [0u8; 64];
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2;
        buf[5] = 1;
        buf[16..18].copy_from_slice(&0x3eu16.to_le_bytes());
        buf[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
        buf[24..32].copy_from_slice(&0x400000u64.to_le_bytes());
        buf[40..48].copy_from_slice(&0u64.to_le_bytes());
        buf[58..60].copy_from_slice(&64u16.to_le_bytes());
        buf[60..62].copy_from_slice(&1u16.to_le_bytes());

        let h = parse_header(&buf).unwrap();
        assert_eq!(h.format, BinaryFormat::ELF);
        assert_eq!(h.arch, Arch::X86_64);
        assert_eq!(h.entry_point, 0x400000);
    }

    #[test]
    fn parse_macho_header() {
        let mut buf = [0u8; 48];
        buf[0..4].copy_from_slice(&0xcefaedfeu32.to_le_bytes());
        buf[4..8].copy_from_slice(&0x00000007u32.to_le_bytes());
        buf[20..24].copy_from_slice(&0u32.to_le_bytes());
        buf[24..28].copy_from_slice(&0u32.to_le_bytes());

        let h = parse_header(&buf).unwrap();
        assert_eq!(h.format, BinaryFormat::MachO);
        assert_eq!(h.arch, Arch::I386);
        assert_eq!(h.entry_point, 0);
        assert_eq!(h.sections.len(), 0);
    }

    #[test]
    fn map_to_userspace_ok() {
        let h = BinaryHeader {
            format: BinaryFormat::ELF,
            arch: Arch::X86_64,
            entry_point: 0x400000,
            image_base: 0,
            sections: Vec::new(),
        };
        assert!(map_to_userspace(&h).is_ok());
    }
}
