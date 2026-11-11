use std::io;

use crate::package::{Package, Reader, invalid};

pub fn skip<'a>(package: &Package<'a>, reader: &mut Reader<'a>, size: u32) -> io::Result<()> {
    let mut script = Script {
        package,
        reader,
        left: size as usize,
    };
    while script.left > 0 {
        script.token(0)?;
    }
    Ok(())
}

struct Script<'p, 'r, 'a> {
    package: &'p Package<'a>,
    reader: &'r mut Reader<'a>,
    left: usize,
}

impl Script<'_, '_, '_> {
    fn consume(&mut self, size: usize) -> io::Result<()> {
        self.left = self
            .left
            .checked_sub(size)
            .ok_or_else(|| invalid("Bytecode exceeds declared size"))?;
        Ok(())
    }

    fn bytes(&mut self, size: usize) -> io::Result<()> {
        self.consume(size)?;
        self.reader.take(size)?;
        Ok(())
    }

    fn index(&mut self) -> io::Result<i32> {
        // The archive uses a compact index. The VM address space uses four bytes.
        self.consume(4)?;
        self.reader.compact()
    }

    fn token(&mut self, depth: u8) -> io::Result<u8> {
        if depth >= 64 {
            return Err(invalid("Bytecode nesting exceeds 64 levels"));
        }
        self.consume(1)?;
        let op = self.reader.byte()?;
        let next = depth + 1;
        match op {
            0x00..=0x02 | 0x20 | 0x21 | 0x29 => {
                self.index()?;
            }
            0x04 if self.package.version <= 61 => {}
            0x04 | 0x0d | 0x0e | 0x2d | 0x39..=0x60 => {
                self.token(next)?;
            }
            0x05 | 0x2b => {
                self.bytes(1)?;
                self.token(next)?;
            }
            0x06 => self.bytes(2)?,
            0x07 | 0x09 | 0x18 => {
                self.bytes(2)?;
                self.token(next)?;
            }
            0x08 | 0x0b | 0x15..=0x17 | 0x25..=0x28 | 0x2a | 0x30 | 0x31 => {}
            0x0a => {
                self.consume(2)?;
                if self.reader.u16()? != 0xffff {
                    self.token(next)?;
                }
            }
            0x0c => loop {
                let name = self.index()?;
                self.bytes(4)?;
                if self
                    .package
                    .name(usize::try_from(name).map_err(|_| invalid("Invalid script label"))?)?
                    == "None"
                {
                    break;
                }
            },
            0x14 if self.package.version <= 63 => loop {
                self.consume(1)?;
                if self.reader.byte()? == 0 {
                    break;
                }
                self.bytes(1)?;
            },
            0x0f | 0x10 | 0x14 | 0x1a => {
                self.token(next)?;
                self.token(next)?;
            }
            0x11 => {
                for _ in 0..4 {
                    self.token(next)?;
                }
            }
            0x12 | 0x19 => {
                self.token(next)?;
                self.bytes(3)?;
                self.token(next)?;
            }
            0x13 | 0x2e | 0x36 => {
                self.index()?;
                self.token(next)?;
            }
            0x1b | 0x1c | 0x38 => {
                self.index()?;
                self.arguments(next)?;
            }
            0x1d | 0x1e => self.bytes(4)?,
            0x1f => loop {
                self.consume(1)?;
                if self.reader.byte()? == 0 {
                    break;
                }
            },
            0x22 | 0x23 => self.bytes(12)?,
            0x24 | 0x2c => self.bytes(1)?,
            0x2f => {
                self.token(next)?;
                self.bytes(2)?;
            }
            0x32 | 0x33 => {
                self.index()?;
                self.token(next)?;
                self.token(next)?;
            }
            0x34 => loop {
                self.consume(2)?;
                if self.reader.u16()? == 0 {
                    break;
                }
            },
            0x61..=0x6f => {
                self.bytes(1)?;
                self.arguments(next)?;
            }
            0x70..=0xff => self.arguments(next)?,
            _ => return Err(invalid(format!("Unknown UE1 opcode 0x{op:02x}"))),
        }
        Ok(op)
    }

    fn arguments(&mut self, depth: u8) -> io::Result<()> {
        while self.token(depth)? != 0x16 {}
        Ok(())
    }
}
