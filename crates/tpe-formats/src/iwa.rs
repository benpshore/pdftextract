//! The IWA container used by Apple iWork: a file is a sequence of chunks
//! (`0x00`, 3-byte little-endian length, raw Snappy data); the concatenated
//! decompressed bytes hold `ArchiveInfo` protobuf headers, each followed by
//! the payloads its `MessageInfo` entries announce. This module decodes that
//! framing and the protobuf wire format; it knows no message schema beyond
//! the few field numbers `iwork` asks for.

/// Decompress raw Snappy (no stream framing, no CRC).
pub fn snappy_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    let (expected, mut pos) = read_varint(input, 0)?;
    let expected = expected as usize;
    if expected > 256 * 1024 * 1024 {
        return Err(format!("snappy: declared size {expected} too large"));
    }
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    while pos < input.len() {
        let tag = input[pos];
        pos += 1;
        match tag & 0x03 {
            0 => {
                let mut len = (tag >> 2) as usize;
                if len >= 60 {
                    let extra = len - 59;
                    len = read_le(input, pos, extra)? as usize;
                    pos += extra;
                }
                len += 1;
                let end = pos + len;
                if end > input.len() {
                    return Err("snappy: literal runs past the input".into());
                }
                out.extend_from_slice(&input[pos..end]);
                pos = end;
            }
            1 => {
                let len = ((tag >> 2) & 0x07) as usize + 4;
                let offset = ((tag as usize >> 5) << 8)
                    | *input.get(pos).ok_or("snappy: truncated copy")? as usize;
                pos += 1;
                copy_back(&mut out, offset, len)?;
            }
            2 => {
                let len = (tag >> 2) as usize + 1;
                let offset = read_le(input, pos, 2)? as usize;
                pos += 2;
                copy_back(&mut out, offset, len)?;
            }
            _ => {
                let len = (tag >> 2) as usize + 1;
                let offset = read_le(input, pos, 4)? as usize;
                pos += 4;
                copy_back(&mut out, offset, len)?;
            }
        }
    }
    if out.len() != expected {
        return Err(format!(
            "snappy: declared {expected} bytes, produced {}",
            out.len()
        ));
    }
    Ok(out)
}

fn copy_back(out: &mut Vec<u8>, offset: usize, len: usize) -> Result<(), String> {
    if offset == 0 || offset > out.len() {
        return Err("snappy: copy offset outside the output".into());
    }
    let start = out.len() - offset;
    for i in 0..len {
        let byte = out[start + i];
        out.push(byte);
    }
    Ok(())
}

fn read_le(input: &[u8], pos: usize, n: usize) -> Result<u64, String> {
    let slice = input.get(pos..pos + n).ok_or("snappy: truncated length")?;
    Ok(slice
        .iter()
        .rev()
        .fold(0u64, |acc, b| (acc << 8) | u64::from(*b)))
}

/// Read a base-128 varint at `pos`; returns the value and the next position.
pub fn read_varint(input: &[u8], mut pos: usize) -> Result<(u64, usize), String> {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let byte = *input.get(pos).ok_or("truncated varint")?;
        pos += 1;
        if shift >= 64 {
            return Err("varint too long".into());
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, pos));
        }
        shift += 7;
    }
}

/// Decompress every chunk of an `.iwa` file and concatenate the results.
pub fn iwa_decompress(file: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < file.len() {
        let header = file
            .get(pos..pos + 4)
            .ok_or("iwa: truncated chunk header")?;
        if header[0] != 0 {
            return Err(format!("iwa: chunk type {:#04x} is not 0x00", header[0]));
        }
        let len =
            usize::from(header[1]) | (usize::from(header[2]) << 8) | (usize::from(header[3]) << 16);
        let body = file
            .get(pos + 4..pos + 4 + len)
            .ok_or("iwa: chunk runs past the file")?;
        out.extend(snappy_decompress(body)?);
        pos += 4 + len;
    }
    Ok(out)
}

/// One protobuf field as the wire format presents it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

/// Split a message into `(field number, value)` pairs in order.
pub fn fields(payload: &[u8]) -> Result<Vec<(u32, Field<'_>)>, String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < payload.len() {
        let (key, next) = read_varint(payload, pos)?;
        pos = next;
        let number = (key >> 3) as u32;
        match key & 0x07 {
            0 => {
                let (value, next) = read_varint(payload, pos)?;
                pos = next;
                out.push((number, Field::Varint(value)));
            }
            1 => {
                let slice = payload
                    .get(pos..pos + 8)
                    .ok_or("protobuf: truncated fixed64")?;
                pos += 8;
                out.push((
                    number,
                    Field::Fixed64(u64::from_le_bytes(slice.try_into().map_err(|_| "fixed64")?)),
                ));
            }
            2 => {
                let (len, next) = read_varint(payload, pos)?;
                let end = next + len as usize;
                let slice = payload
                    .get(next..end)
                    .ok_or("protobuf: truncated bytes field")?;
                pos = end;
                out.push((number, Field::Bytes(slice)));
            }
            5 => {
                let slice = payload
                    .get(pos..pos + 4)
                    .ok_or("protobuf: truncated fixed32")?;
                pos += 4;
                out.push((
                    number,
                    Field::Fixed32(u32::from_le_bytes(slice.try_into().map_err(|_| "fixed32")?)),
                ));
            }
            other => return Err(format!("protobuf: wire type {other} not supported")),
        }
    }
    Ok(out)
}

/// A message payload with the type its `MessageInfo` declared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// `ArchiveInfo.identifier`.
    pub identifier: u64,
    /// `MessageInfo.type`.
    pub type_id: u32,
    pub payload: Vec<u8>,
}

/// Walk a decompressed archive stream: `ArchiveInfo` headers and the
/// payloads they announce (`MessageInfo.length`, field 3).
pub fn parse_archives(stream: &[u8]) -> Result<Vec<Message>, String> {
    let mut messages = Vec::new();
    let mut pos = 0;
    while pos < stream.len() {
        let (len, next) = read_varint(stream, pos)?;
        let end = next + len as usize;
        let info = stream.get(next..end).ok_or("iwa: truncated ArchiveInfo")?;
        pos = end;
        let mut identifier = 0;
        let mut announced: Vec<(u32, usize)> = Vec::new();
        for (number, value) in fields(info)? {
            match (number, value) {
                (1, Field::Varint(id)) => identifier = id,
                (2, Field::Bytes(message_info)) => {
                    let mut type_id = 0;
                    let mut length = 0usize;
                    for (n, v) in fields(message_info)? {
                        match (n, v) {
                            (1, Field::Varint(t)) => type_id = t as u32,
                            (3, Field::Varint(l)) => length = l as usize,
                            _ => {}
                        }
                    }
                    announced.push((type_id, length));
                }
                _ => {}
            }
        }
        for (type_id, length) in announced {
            let payload = stream
                .get(pos..pos + length)
                .ok_or("iwa: payload runs past the stream")?;
            pos += length;
            messages.push(Message {
                identifier,
                type_id,
                payload: payload.to_vec(),
            });
        }
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snappy_literals_and_copies() {
        // "abc" literal, then copy offset 3 length 6, then literal "!".
        let input = [10, 0x08, b'a', b'b', b'c', 0b0000_1001, 3, 0x00, b'!'];
        assert_eq!(snappy_decompress(&input).unwrap(), b"abcabcabc!");
        assert!(snappy_decompress(&[5, 0x00, b'a']).is_err());
        assert!(snappy_decompress(&[2, 0b0000_1001, 9]).is_err());
    }

    #[test]
    fn varints_and_fields() {
        assert_eq!(read_varint(&[0x96, 0x01], 0).unwrap(), (150, 2));
        let payload = [0x08, 0x96, 0x01, 0x1a, 0x02, b'h', b'i'];
        assert_eq!(
            fields(&payload).unwrap(),
            vec![(1, Field::Varint(150)), (3, Field::Bytes(b"hi"))]
        );
    }
}
