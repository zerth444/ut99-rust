use std::io::{self, ErrorKind};
use std::ops::Range;

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message.into())
}

pub struct Reader<'a> {
    pub remaining: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn at(data: &'a [u8], offset: usize) -> io::Result<Self> {
        Ok(Self {
            remaining: data
                .get(offset..)
                .ok_or_else(|| invalid("Offset outside package"))?,
        })
    }

    pub fn take(&mut self, size: usize) -> io::Result<&'a [u8]> {
        let (value, rest) = self
            .remaining
            .split_at_checked(size)
            .ok_or_else(|| invalid("Unexpected end of package"))?;
        self.remaining = rest;
        Ok(value)
    }

    pub fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn f32(&mut self) -> io::Result<f32> {
        let value = f32::from_bits(self.u32()?);
        if !value.is_finite() {
            return Err(invalid("Non-finite coordinate"));
        }
        Ok(value)
    }

    pub fn compact(&mut self) -> io::Result<i32> {
        let first = self.byte()?;
        let mut value = u64::from(first & 0x3f);
        if first & 0x40 != 0 {
            for shift in [6, 13, 20, 27] {
                let byte = self.byte()?;
                if shift == 27 {
                    value |= u64::from(byte) << shift;
                    break;
                }
                value |= u64::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    break;
                }
            }
        }
        let signed = if first & 0x80 == 0 {
            value as i64
        } else {
            -(value as i64)
        };
        i32::try_from(signed).map_err(|_| invalid("Compact index exceeds i32"))
    }

    pub fn index(&mut self) -> io::Result<usize> {
        usize::try_from(self.compact()?).map_err(|_| invalid("Unexpected negative index"))
    }

    pub fn count(&mut self, minimum_size: usize) -> io::Result<usize> {
        let count = self.index()?;
        bounded_count(count, minimum_size, self.remaining.len())
    }

    pub fn string(&mut self, version: u16) -> io::Result<String> {
        if version < 64 {
            let end = self
                .remaining
                .iter()
                .position(|&value| value == 0)
                .ok_or_else(|| invalid("Unterminated name"))?;
            let bytes = self.take(end + 1)?;
            return Ok(decode_ansi(&bytes[..end]));
        }
        let length = self.compact()?;
        if length == 0 {
            return Ok(String::new());
        }
        if length > 0 {
            let bytes = self.take(length as usize)?;
            return bytes
                .strip_suffix(&[0])
                .map(decode_ansi)
                .ok_or_else(|| invalid("Unterminated ANSI name"));
        }
        let bytes = self.take(length.unsigned_abs() as usize * 2)?;
        let bytes = bytes
            .strip_suffix(&[0, 0])
            .ok_or_else(|| invalid("Unterminated UTF-16 name"))?;
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
        Ok(char::decode_utf16(units)
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect())
    }
}

fn decode_ansi(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}

fn bounded_count(count: usize, minimum_size: usize, available: usize) -> io::Result<usize> {
    if count > available / minimum_size {
        return Err(invalid("Table exceeds package bounds"));
    }
    Ok(count)
}

pub struct Import {
    pub name: usize,
    pub class: usize,
    pub outer: i32,
}

pub struct Export {
    pub name: usize,
    pub class: i32,
    pub superclass: i32,
    pub flags: u32,
    pub outer: i32,
    pub data: Range<usize>,
}

pub struct Package<'a> {
    pub data: &'a [u8],
    pub version: u16,
    pub licensee_version: u16,
    pub names: Vec<String>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
}

impl<'a> Package<'a> {
    pub fn read(data: &'a [u8]) -> io::Result<Self> {
        let mut header = Reader::at(data, 0)?;
        if header.u32()? != 0x9e2a83c1 {
            return Err(invalid(
                "Expected a UE1 package (.unr, .u, .utx, .uax, .umx)",
            ));
        }
        let version = header.u16()?;
        let licensee_version = header.u16()?;
        if !(61..=69).contains(&version) {
            return Err(invalid(format!(
                "Unsupported UT99 package version {version}"
            )));
        }
        header.u32()?;
        let (name_count, name_offset) = (header.u32()? as usize, header.u32()? as usize);
        let (export_count, export_offset) = (header.u32()? as usize, header.u32()? as usize);
        let (import_count, import_offset) = (header.u32()? as usize, header.u32()? as usize);

        let mut reader = Reader::at(data, name_offset)?;
        let mut names = Vec::with_capacity(bounded_count(name_count, 5, reader.remaining.len())?);
        for _ in 0..name_count {
            names.push(reader.string(version)?);
            reader.u32()?;
        }

        let mut reader = Reader::at(data, import_offset)?;
        let mut imports =
            Vec::with_capacity(bounded_count(import_count, 7, reader.remaining.len())?);
        for _ in 0..import_count {
            name_index(&mut reader, &names)?;
            let class = name_index(&mut reader, &names)?;
            let outer = reader.u32()? as i32;
            let name = name_index(&mut reader, &names)?;
            imports.push(Import { name, class, outer });
        }

        let mut reader = Reader::at(data, export_offset)?;
        let mut exports =
            Vec::with_capacity(bounded_count(export_count, 12, reader.remaining.len())?);
        for _ in 0..export_count {
            let class = reader.compact()?;
            let superclass = reader.compact()?;
            let outer = reader.u32()? as i32;
            let name = name_index(&mut reader, &names)?;
            let flags = reader.u32()?;
            let size = reader.index()?;
            let offset = if size == 0 { 0 } else { reader.index()? };
            let end = offset
                .checked_add(size)
                .filter(|&end| end <= data.len())
                .ok_or_else(|| invalid("Export exceeds package bounds"))?;
            exports.push(Export {
                name,
                class,
                superclass,
                flags,
                outer,
                data: offset..end,
            });
        }
        Ok(Self {
            data,
            version,
            licensee_version,
            names,
            imports,
            exports,
        })
    }

    pub fn name(&self, index: usize) -> io::Result<&str> {
        self.names
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| invalid("Unknown name index"))
    }

    pub fn object_name(&self, reference: i32) -> Option<&str> {
        match reference {
            0 => None,
            1.. => self
                .exports
                .get(reference as usize - 1)
                .map(|item| self.names[item.name].as_str()),
            _ => self
                .imports
                .get(reference.unsigned_abs() as usize - 1)
                .map(|item| self.names[item.name].as_str()),
        }
    }

    pub fn properties<'p>(&'p self, export: &Export) -> io::Result<Properties<'p, 'a>> {
        let mut reader = Reader::at(&self.data[export.data.clone()], 0)?;
        if export.flags & 0x0200_0000 != 0 {
            let node = reader.compact()?;
            reader.compact()?;
            reader.take(12)?;
            if node != 0 {
                reader.compact()?;
            }
        }
        Ok(self.properties_at(reader))
    }

    pub fn properties_at(&self, reader: Reader<'a>) -> Properties<'_, 'a> {
        Properties {
            package: self,
            reader,
        }
    }
}

pub struct Properties<'p, 'a> {
    package: &'p Package<'a>,
    pub reader: Reader<'a>,
}

pub struct Property<'p, 'a> {
    pub name: &'p str,
    pub array_index: usize,
    pub boolean: Option<bool>,
    pub value: Reader<'a>,
}

impl<'p, 'a> Properties<'p, 'a> {
    pub fn next(&mut self) -> io::Result<Option<(&'p str, Reader<'a>)>> {
        Ok(self.next_tag()?.map(|tag| (tag.name, tag.value)))
    }

    pub fn next_tag(&mut self) -> io::Result<Option<Property<'p, 'a>>> {
        let name = self.package.name(self.reader.index()?)?;
        if name.eq_ignore_ascii_case("None") {
            return Ok(None);
        }
        let info = self.reader.byte()?;
        let kind = info & 0x0f;
        if kind == 10 {
            self.package.name(self.reader.index()?)?;
        }
        let size = match (info >> 4) & 7 {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 12,
            4 => 16,
            5 => self.reader.byte()? as usize,
            6 => self.reader.u16()? as usize,
            _ => self.reader.u32()? as usize,
        };
        let mut array_index = 0;
        if info & 0x80 != 0 && kind != 3 {
            let first = self.reader.byte()?;
            array_index = match first {
                0..=0x7f => usize::from(first),
                0x80..=0xbf => (usize::from(first & 0x7f) << 8) | usize::from(self.reader.byte()?),
                _ => {
                    let extra = self.reader.take(3)?;
                    u32::from_be_bytes([first & 0x3f, extra[0], extra[1], extra[2]]) as usize
                }
            };
        }
        Ok(Some(Property {
            name,
            array_index,
            boolean: (kind == 3).then_some(info & 0x80 != 0),
            value: Reader {
                remaining: self.reader.take(if kind == 3 { 0 } else { size })?,
            },
        }))
    }
}

fn name_index(reader: &mut Reader<'_>, names: &[String]) -> io::Result<usize> {
    let index = reader.index()?;
    if index >= names.len() {
        return Err(invalid("Unknown name index in table"));
    }
    Ok(index)
}
