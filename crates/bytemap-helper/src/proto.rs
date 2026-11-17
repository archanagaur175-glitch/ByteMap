//! BYM1 wire protocol, shared by the helper (server side) and the scan
//! client. Everything is little-endian.
//!
//! Session: client connects, sends the 6-byte handshake (magic + u16
//! version), server answers 7 bytes (magic + its version + status byte:
//! 0 ok, 1 bad magic, 2 bad version). Then both sides talk length-prefixed
//! frames: `[u32 LE payload length][payload]`, where the payload starts
//! with a one-byte opcode:
//!
//! | op | frame       | payload after the opcode                                        |
//! |----|-------------|-----------------------------------------------------------------|
//! | 1  | ScanRequest | u16 len + UTF-8 path (the scan root)                            |
//! | 2  | Record      | file_ref u64, parent_ref u64, logical u64, allocated u64, attrs u32, u16 len + UTF-8 name |
//! | 3  | Done        | entries u64, logical_bytes u64 (session summary)                |
//! | 4  | Error       | u16 len + UTF-8 message (terminal: the server closes after it)  |

use std::io::{self, Read, Write};

use crate::{PROTOCOL_MAGIC, PROTOCOL_VERSION};

pub const OP_SCAN_REQUEST: u8 = 1;
pub const OP_RECORD: u8 = 2;
pub const OP_DONE: u8 = 3;
pub const OP_ERROR: u8 = 4;

/// Hard cap on one frame's payload. Names and paths are small; anything
/// bigger is a protocol violation, not a scan.
pub const MAX_FRAME_PAYLOAD: u32 = 64 * 1024 * 1024;

/// One MFT record on the wire (P6.T3 streams these).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Full MFT reference of the file (record number + sequence number).
    pub file_ref: u64,
    /// Full MFT reference of the containing directory.
    pub parent_ref: u64,
    pub logical_size: u64,
    pub allocated_size: u64,
    /// Win32 attribute flags (`FILE_ATTRIBUTE_*`): directory bit, hidden,
    /// system, reparse point, ...
    pub attributes: u32,
    /// The file's name (longest Win32 name; UTF-8 on the wire).
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    ScanRequest { path: String },
    Record(Record),
    Done { entries: u64, logical_bytes: u64 },
    Error { message: String },
}

#[derive(Debug)]
pub enum HandshakeError {
    Io(io::Error),
    BadMagic,
    BadVersion(u16),
}

impl From<io::Error> for HandshakeError {
    fn from(err: io::Error) -> Self {
        HandshakeError::Io(err)
    }
}

impl core::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HandshakeError::Io(err) => write!(f, "handshake io error: {err}"),
            HandshakeError::BadMagic => write!(f, "handshake: bad magic"),
            HandshakeError::BadVersion(v) => {
                write!(f, "handshake: protocol version {v} != {PROTOCOL_VERSION}")
            }
        }
    }
}

impl std::error::Error for HandshakeError {}

/// Client side of the handshake: send magic + version, validate the reply.
pub fn handshake_client(stream: &mut (impl Read + Write)) -> Result<(), HandshakeError> {
    stream.write_all(&PROTOCOL_MAGIC)?;
    stream.write_all(&PROTOCOL_VERSION.to_le_bytes())?;
    let mut reply = [0u8; 7];
    stream.read_exact(&mut reply)?;
    if reply[0..4] != PROTOCOL_MAGIC {
        return Err(HandshakeError::BadMagic);
    }
    let version = u16::from_le_bytes([reply[4], reply[5]]);
    match reply[6] {
        0 => Ok(()),
        1 => Err(HandshakeError::BadMagic),
        2 => Err(HandshakeError::BadVersion(version)),
        status => Err(HandshakeError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown handshake status {status}"),
        ))),
    }
}

/// Server side of the handshake: validate the request, answer with status.
/// A rejected client always gets its reply before the error is returned.
pub fn handshake_server(stream: &mut (impl Read + Write)) -> Result<(), HandshakeError> {
    let mut request = [0u8; 6];
    stream.read_exact(&mut request)?;
    let version = u16::from_le_bytes([request[4], request[5]]);
    let (status, verdict) = if request[0..4] != PROTOCOL_MAGIC {
        (1u8, Some(HandshakeError::BadMagic))
    } else if version != PROTOCOL_VERSION {
        (2, Some(HandshakeError::BadVersion(version)))
    } else {
        (0, None)
    };
    let mut reply = Vec::with_capacity(7);
    reply.extend_from_slice(&PROTOCOL_MAGIC);
    reply.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    reply.push(status);
    stream.write_all(&reply)?;
    match verdict {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Write one length-prefixed frame.
pub fn write_frame(stream: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let payload = encode(frame);
    stream.write_all(&(payload.len() as u32).to_le_bytes())?;
    stream.write_all(&payload)
}

/// Read one length-prefixed frame.
pub fn read_frame(stream: &mut impl Read) -> io::Result<Frame> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header);
    if len > MAX_FRAME_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds the {MAX_FRAME_PAYLOAD}-byte cap"),
        ));
    }
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload)?;
    decode(&payload)
}

fn encode(frame: &Frame) -> Vec<u8> {
    match frame {
        Frame::ScanRequest { path } => {
            let mut v = Vec::with_capacity(3 + path.len());
            v.push(OP_SCAN_REQUEST);
            v.extend_from_slice(&(path.len() as u16).to_le_bytes());
            v.extend_from_slice(path.as_bytes());
            v
        }
        Frame::Record(rec) => {
            let mut v = Vec::with_capacity(1 + 8 * 4 + 4 + 2 + rec.name.len());
            v.push(OP_RECORD);
            v.extend_from_slice(&rec.file_ref.to_le_bytes());
            v.extend_from_slice(&rec.parent_ref.to_le_bytes());
            v.extend_from_slice(&rec.logical_size.to_le_bytes());
            v.extend_from_slice(&rec.allocated_size.to_le_bytes());
            v.extend_from_slice(&rec.attributes.to_le_bytes());
            v.extend_from_slice(&(rec.name.len() as u16).to_le_bytes());
            v.extend_from_slice(rec.name.as_bytes());
            v
        }
        Frame::Done {
            entries,
            logical_bytes,
        } => {
            let mut v = Vec::with_capacity(17);
            v.push(OP_DONE);
            v.extend_from_slice(&entries.to_le_bytes());
            v.extend_from_slice(&logical_bytes.to_le_bytes());
            v
        }
        Frame::Error { message } => {
            let mut v = Vec::with_capacity(3 + message.len());
            v.push(OP_ERROR);
            v.extend_from_slice(&(message.len() as u16).to_le_bytes());
            v.extend_from_slice(message.as_bytes());
            v
        }
    }
}

fn decode(payload: &[u8]) -> io::Result<Frame> {
    let (&op, rest) = payload
        .split_first()
        .ok_or_else(|| bad_frame("empty frame"))?;
    match op {
        OP_SCAN_REQUEST => {
            let (path, rest) = take_string(rest)?;
            trailing(rest)?;
            Ok(Frame::ScanRequest { path })
        }
        OP_RECORD => {
            let fixed = 8 * 4 + 4;
            if rest.len() < fixed {
                return Err(bad_frame("record shorter than its fixed fields"));
            }
            let file_ref = u64::from_le_bytes(rest[0..8].try_into().expect("8 bytes"));
            let parent_ref = u64::from_le_bytes(rest[8..16].try_into().expect("8 bytes"));
            let logical_size = u64::from_le_bytes(rest[16..24].try_into().expect("8 bytes"));
            let allocated_size = u64::from_le_bytes(rest[24..32].try_into().expect("8 bytes"));
            let attributes = u32::from_le_bytes(rest[32..36].try_into().expect("4 bytes"));
            let (name, rest) = take_string(&rest[fixed..])?;
            trailing(rest)?;
            Ok(Frame::Record(Record {
                file_ref,
                parent_ref,
                logical_size,
                allocated_size,
                attributes,
                name,
            }))
        }
        OP_DONE => {
            if rest.len() != 16 {
                return Err(bad_frame("done frame must carry exactly 16 bytes"));
            }
            let entries = u64::from_le_bytes(rest[0..8].try_into().expect("8 bytes"));
            let logical_bytes = u64::from_le_bytes(rest[8..16].try_into().expect("8 bytes"));
            Ok(Frame::Done {
                entries,
                logical_bytes,
            })
        }
        OP_ERROR => {
            let (message, rest) = take_string(rest)?;
            trailing(rest)?;
            Ok(Frame::Error { message })
        }
        _ => Err(bad_frame("unknown opcode {op}")),
    }
}

/// `u16 length + UTF-8 bytes`, bounds- and UTF-8-checked.
fn take_string(data: &[u8]) -> io::Result<(String, &[u8])> {
    if data.len() < 2 {
        return Err(bad_frame("string length prefix missing"));
    }
    let len = u16::from_le_bytes([data[0], data[1]]) as usize;
    let rest = &data[2..];
    if rest.len() < len {
        return Err(bad_frame("string shorter than its length prefix"));
    }
    let string = std::str::from_utf8(&rest[..len])
        .map_err(|_| bad_frame("string is not valid UTF-8"))?
        .to_owned();
    Ok((string, &rest[len..]))
}

fn trailing(rest: &[u8]) -> io::Result<()> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(bad_frame("trailing bytes after frame payload"))
    }
}

fn bad_frame(reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("protocol error: {reason}"),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn frames_round_trip() {
        let frames = [
            Frame::ScanRequest {
                path: r"C:\temp\späte-近".to_owned(),
            },
            Frame::Record(Record {
                file_ref: 0x1122_3344_5566_7788,
                parent_ref: 5,
                logical_size: 123_456_789,
                allocated_size: 123_457_536,
                attributes: 0x0020,
                name: "報告書.txt".to_owned(),
            }),
            Frame::Done {
                entries: 101_112,
                logical_bytes: 9_876_543_210,
            },
            Frame::Error {
                message: "boom".to_owned(),
            },
        ];
        for frame in frames {
            let mut wire = Vec::new();
            write_frame(&mut wire, &frame).unwrap();
            let decoded = read_frame(&mut Cursor::new(&wire)).unwrap();
            assert_eq!(decoded, frame);
        }
    }

    #[test]
    fn oversized_frame_is_rejected_before_reading_the_payload() {
        let mut wire = (MAX_FRAME_PAYLOAD + 1).to_le_bytes().to_vec();
        wire.push(OP_ERROR); // payload never reached
        let err = read_frame(&mut Cursor::new(wire)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn unknown_opcode_is_rejected() {
        let wire = [1u8, 0, 0, 0, 99];
        let err = read_frame(&mut Cursor::new(wire)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn truncated_string_is_rejected() {
        let wire = [4u8, 0, 0, 0, OP_ERROR, 99, 0, b'a'];
        let err = read_frame(&mut Cursor::new(wire)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// A one-shot duplex buffer: `Read` drains `incoming`, `Write` appends
    /// to `outgoing`. Stands in for a pipe in handshake tests.
    struct Duplex {
        incoming: Vec<u8>,
        pos: usize,
        outgoing: Vec<u8>,
    }

    impl Duplex {
        fn new(incoming: Vec<u8>) -> Self {
            Self {
                incoming,
                pos: 0,
                outgoing: Vec::new(),
            }
        }
    }

    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let remaining = &self.incoming[self.pos..];
            let n = remaining.len().min(buf.len());
            buf[..n].copy_from_slice(&remaining[..n]);
            self.pos += n;
            Ok(n)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.outgoing.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn handshake_request(magic: &[u8; 4], version: u16) -> Vec<u8> {
        let mut request = Vec::new();
        request.extend_from_slice(magic);
        request.extend_from_slice(&version.to_le_bytes());
        request
    }

    #[test]
    fn server_handshake_replies_ok_over_a_real_buffer() {
        let mut stream = Duplex::new(handshake_request(&PROTOCOL_MAGIC, PROTOCOL_VERSION));
        handshake_server(&mut stream).unwrap();
        assert_eq!(
            stream.outgoing,
            vec![b'B', b'Y', b'M', b'1', 1, 0, 0],
            "reply must be magic + version + status 0"
        );
    }

    #[test]
    fn server_handshake_rejects_bad_magic_after_replying() {
        let mut stream = Duplex::new(handshake_request(b"NOPE", PROTOCOL_VERSION));
        let err = handshake_server(&mut stream).unwrap_err();
        assert!(matches!(err, HandshakeError::BadMagic));
        assert_eq!(
            stream.outgoing,
            vec![b'B', b'Y', b'M', b'1', 1, 0, 1],
            "status 1 must be sent before the rejection"
        );
    }

    #[test]
    fn client_handshake_accepts_ok_and_rejects_statuses() {
        let mut ok = handshake_request(&PROTOCOL_MAGIC, PROTOCOL_VERSION);
        ok.push(0);
        let mut stream = Duplex::new(ok);
        handshake_client(&mut stream).unwrap();
        assert_eq!(
            stream.outgoing,
            handshake_request(&PROTOCOL_MAGIC, PROTOCOL_VERSION),
            "the client must send magic + version"
        );

        let mut bad = handshake_request(&PROTOCOL_MAGIC, PROTOCOL_VERSION);
        bad.push(2);
        let mut stream = Duplex::new(bad);
        let err = handshake_client(&mut stream).unwrap_err();
        assert!(matches!(err, HandshakeError::BadVersion(PROTOCOL_VERSION)));
    }
}
