//! One request and one response per connection, followed by a one-byte receipt.
//! u32 LE payload length, u16 LE version, u8 kind, length-prefixed request ID,
//! then an enumerated operation or outcome. Paths are lossless Windows UTF-16LE.
use super::super::security::invalid;
use crate::app::{ExternalOperation, ExternalRequestId, ScrollDirection};
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

pub(super) const MAX_FRAME: usize = 65536;
const VERSION: u16 = 2;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: ExternalRequestId,
    pub operation: ExternalOperation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub id: ExternalRequestId,
    pub outcome: Result<(), String>,
}
fn field(out: &mut Vec<u8>, value: &[u8]) -> io::Result<()> {
    if value.len() > 60000 {
        return Err(invalid("field too long"));
    }
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value);
    Ok(())
}
fn prefix(id: &ExternalRequestId, kind: u8) -> io::Result<Vec<u8>> {
    validate_id(&id.0)?;
    let mut out = VERSION.to_le_bytes().to_vec();
    out.push(kind);
    field(&mut out, id.0.as_bytes())?;
    Ok(out)
}
fn validate_id(id: &str) -> io::Result<()> {
    if id.is_empty() || id.len() > 256 {
        Err(invalid("invalid request ID"))
    } else {
        Ok(())
    }
}
fn frame(payload: Vec<u8>) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_FRAME {
        return Err(invalid("frame too long"));
    }
    let mut out = (payload.len() as u32).to_le_bytes().to_vec();
    out.extend(payload);
    Ok(out)
}
impl Request {
    pub(super) fn encode(&self) -> io::Result<Vec<u8>> {
        let mut out = prefix(&self.id, 1)?;
        let opcode = match &self.operation {
            ExternalOperation::Open { .. } => 0,
            ExternalOperation::Scroll(direction) => match direction {
                ScrollDirection::Down => 1,
                ScrollDirection::Up => 2,
                ScrollDirection::HalfDown => 3,
                ScrollDirection::HalfUp => 4,
                ScrollDirection::PageDown => 5,
                ScrollDirection::PageUp => 6,
            },
            ExternalOperation::Top => 7,
            ExternalOperation::Bottom => 8,
            ExternalOperation::Back => 9,
            ExternalOperation::Forward => 10,
        };
        out.push(opcode);
        if let ExternalOperation::Open { path, line } = &self.operation {
            if !path.is_absolute() {
                return Err(invalid("open requires an absolute CLI path"));
            }
            let units: Vec<u16> = path.as_os_str().encode_wide().collect();
            if units.contains(&0) {
                return Err(invalid("NUL in path"));
            }
            let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
            field(&mut out, &bytes)?;
            out.push(u8::from(line.is_some()));
            if let Some(line) = line {
                out.extend_from_slice(&line.get().to_le_bytes());
            }
        }
        frame(out)
    }
    pub(super) fn decode(payload: &[u8]) -> io::Result<Self> {
        let (mut input, id) = Decoder::new(payload, 1)?;
        let operation = match input.byte()? {
            0 => {
                let raw = input.field()?;
                if raw.len() % 2 != 0 {
                    return Err(invalid("odd UTF-16 path length"));
                }
                let units: Vec<u16> = raw
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let path = PathBuf::from(OsString::from_wide(&units));
                if !path.is_absolute() || units.contains(&0) {
                    return Err(invalid("invalid absolute path"));
                }
                let line = match input.byte()? {
                    0 => None,
                    1 => Some(
                        std::num::NonZeroU64::new(u64::from_le_bytes(
                            input.take(8)?.try_into().unwrap(),
                        ))
                        .ok_or_else(|| invalid("source line must be positive"))?,
                    ),
                    _ => return Err(invalid("invalid source line presence flag")),
                };
                ExternalOperation::Open { path, line }
            }
            1 => ExternalOperation::Scroll(ScrollDirection::Down),
            2 => ExternalOperation::Scroll(ScrollDirection::Up),
            3 => ExternalOperation::Scroll(ScrollDirection::HalfDown),
            4 => ExternalOperation::Scroll(ScrollDirection::HalfUp),
            5 => ExternalOperation::Scroll(ScrollDirection::PageDown),
            6 => ExternalOperation::Scroll(ScrollDirection::PageUp),
            7 => ExternalOperation::Top,
            8 => ExternalOperation::Bottom,
            9 => ExternalOperation::Back,
            10 => ExternalOperation::Forward,
            _ => return Err(invalid("unknown operation")),
        };
        input.end()?;
        Ok(Self { id, operation })
    }
}
impl Response {
    pub(super) fn encode(&self) -> io::Result<Vec<u8>> {
        let mut out = prefix(&self.id, 2)?;
        out.push(u8::from(self.outcome.is_err()));
        if let Err(reason) = &self.outcome {
            field(&mut out, reason.as_bytes())?;
        }
        frame(out)
    }
    pub(super) fn decode(payload: &[u8]) -> io::Result<Self> {
        let (mut input, id) = Decoder::new(payload, 2)?;
        let outcome = match input.byte()? {
            0 => Ok(()),
            1 => Err(input.string()?),
            _ => return Err(invalid("invalid outcome")),
        };
        input.end()?;
        Ok(Self { id, outcome })
    }
}
struct Decoder<'a>(&'a [u8]);
impl<'a> Decoder<'a> {
    fn new(payload: &'a [u8], kind: u8) -> io::Result<(Self, ExternalRequestId)> {
        if payload.len() > MAX_FRAME || payload.get(..3) != Some(&[VERSION as u8, 0, kind]) {
            return Err(invalid("unsupported protocol version or message kind"));
        }
        let mut input = Self(&payload[3..]);
        let id = input.string()?;
        validate_id(&id)?;
        Ok((input, ExternalRequestId(id)))
    }
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < count {
            return Err(invalid("truncated message"));
        }
        let (head, tail) = self.0.split_at(count);
        self.0 = tail;
        Ok(head)
    }
    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn field(&mut self) -> io::Result<&'a [u8]> {
        let length = u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize;
        if length > 60000 {
            return Err(invalid("field too long"));
        }
        self.take(length)
    }
    fn string(&mut self) -> io::Result<String> {
        String::from_utf8(self.field()?.to_vec()).map_err(|_| invalid("invalid UTF-8"))
    }
    fn end(self) -> io::Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(invalid("trailing message bytes"))
        }
    }
}
