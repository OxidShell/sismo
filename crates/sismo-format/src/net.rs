use crate::{DecodeError, Decoder, Encoder, SessionInfo};

/// UDP port used for discovery probes.
pub const DISCOVERY_PORT: u16 = 7701;
/// Default port a recorder listens on for viewers.
pub const DEFAULT_PORT: u16 = 7720;
/// Application id used to scope discovery to sismo peers.
pub const DISCOVERY_APP_ID: &str = "sismo";

/// zudp stream ids.
pub mod stream {
    pub const CONTROL: u16 = 0;
    pub const DATA: u16 = 1;
    pub const BULK: u16 = 2;
}

/// How a recorder accepts viewers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// Any viewer on the network may connect.
    Open = 0,
    /// The viewer must present the recorder's pairing code once.
    Pairing = 1,
}

impl TryFrom<u8> for AuthMode {
    type Error = DecodeError;

    fn try_from(value: u8) -> Result<Self, DecodeError> {
        match value {
            0 => Ok(AuthMode::Open),
            1 => Ok(AuthMode::Pairing),
            other => Err(DecodeError::InvalidTag {
                what: "auth mode",
                tag: other,
            }),
        }
    }
}

/// Discovery metadata advertised by a recorder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beacon {
    pub format_version: u8,
    pub sismo_version: String,
    pub session_id: u64,
    pub app: String,
    pub app_version: String,
    pub platform: String,
    pub device: String,
    pub port: u16,
    pub auth: AuthMode,
    /// Hex fingerprint of the recorder's static key.
    pub fingerprint: String,
}

impl Beacon {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut enc = Encoder::default();
        enc.u8(self.format_version)
            .str(&self.sismo_version)
            .uvar(self.session_id)
            .str(&self.app)
            .str(&self.app_version)
            .str(&self.platform)
            .str(&self.device)
            .uvar(u64::from(self.port))
            .u8(self.auth as u8)
            .str(&self.fingerprint);
        enc.into_inner()
    }

    /// # Errors
    /// Any [`DecodeError`] for malformed input.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut dec = Decoder::new(bytes);
        Ok(Self {
            format_version: dec.u8()?,
            sismo_version: dec.string()?,
            session_id: dec.uvar()?,
            app: dec.string()?,
            app_version: dec.string()?,
            platform: dec.string()?,
            device: dec.string()?,
            port: u16::try_from(dec.uvar()?).map_err(|_| DecodeError::OutOfRange)?,
            auth: AuthMode::try_from(dec.u8()?)?,
            fingerprint: dec.string()?,
        })
    }
}

/// Messages a viewer sends to a recorder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToRecorder {
    Hello {
        viewer: String,
    },
    Pair {
        code: String,
    },
    /// Start streaming; `history` also replays the recorder's ring buffer,
    /// only the chunks after `since` (a [`ToViewer::Chunk`] seq the viewer
    /// already has, `0` for none), so a reconnect never repeats data.
    Subscribe {
        history: bool,
        since: u64,
    },
    SetFilter {
        filter: String,
    },
    Marker {
        note: String,
    },
    StartDeviceRecording,
    StopDeviceRecording,
    Ping {
        nonce: u64,
    },
    Bye,
}

/// Messages a recorder sends to a viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToViewer {
    Welcome {
        session: SessionInfo,
        pairing_required: bool,
        filter: String,
        history_bytes: u64,
    },
    Paired,
    PairRejected {
        reason: String,
    },
    /// A sealed chunk (see [`crate::Chunk`]) and its position in the
    /// recorder's stream; seqs only grow within one session.
    Chunk {
        seq: u64,
        bytes: Vec<u8>,
    },
    HistoryEnd,
    Filter {
        filter: String,
    },
    /// Piece of the on-device recording, uploaded after it stops.
    DeviceRecording {
        offset: u64,
        total: u64,
        bytes: Vec<u8>,
    },
    Pong {
        nonce: u64,
    },
}

mod tag {
    pub const HELLO: u8 = 1;
    pub const PAIR: u8 = 2;
    pub const SUBSCRIBE: u8 = 3;
    pub const SET_FILTER: u8 = 4;
    pub const MARKER: u8 = 5;
    pub const START_DEVICE: u8 = 6;
    pub const STOP_DEVICE: u8 = 7;
    pub const PING: u8 = 8;
    pub const BYE: u8 = 9;

    pub const WELCOME: u8 = 1;
    pub const PAIRED: u8 = 2;
    pub const PAIR_REJECTED: u8 = 3;
    pub const CHUNK: u8 = 4;
    pub const HISTORY_END: u8 = 5;
    pub const FILTER: u8 = 6;
    pub const DEVICE_RECORDING: u8 = 7;
    pub const PONG: u8 = 8;
}

impl ToRecorder {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut enc = Encoder::default();
        match self {
            ToRecorder::Hello { viewer } => enc.u8(tag::HELLO).str(viewer),
            ToRecorder::Pair { code } => enc.u8(tag::PAIR).str(code),
            ToRecorder::Subscribe { history, since } => {
                enc.u8(tag::SUBSCRIBE).bool(*history).uvar(*since)
            }
            ToRecorder::SetFilter { filter } => enc.u8(tag::SET_FILTER).str(filter),
            ToRecorder::Marker { note } => enc.u8(tag::MARKER).str(note),
            ToRecorder::StartDeviceRecording => enc.u8(tag::START_DEVICE),
            ToRecorder::StopDeviceRecording => enc.u8(tag::STOP_DEVICE),
            ToRecorder::Ping { nonce } => enc.u8(tag::PING).uvar(*nonce),
            ToRecorder::Bye => enc.u8(tag::BYE),
        };
        enc.into_inner()
    }

    /// # Errors
    /// Any [`DecodeError`] for malformed input.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut dec = Decoder::new(bytes);
        Ok(match dec.u8()? {
            tag::HELLO => ToRecorder::Hello {
                viewer: dec.string()?,
            },
            tag::PAIR => ToRecorder::Pair {
                code: dec.string()?,
            },
            tag::SUBSCRIBE => ToRecorder::Subscribe {
                history: dec.bool()?,
                since: dec.uvar()?,
            },
            tag::SET_FILTER => ToRecorder::SetFilter {
                filter: dec.string()?,
            },
            tag::MARKER => ToRecorder::Marker {
                note: dec.string()?,
            },
            tag::START_DEVICE => ToRecorder::StartDeviceRecording,
            tag::STOP_DEVICE => ToRecorder::StopDeviceRecording,
            tag::PING => ToRecorder::Ping { nonce: dec.uvar()? },
            tag::BYE => ToRecorder::Bye,
            other => {
                return Err(DecodeError::InvalidTag {
                    what: "recorder message",
                    tag: other,
                });
            }
        })
    }
}

impl ToViewer {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut enc = Encoder::default();
        match self {
            ToViewer::Welcome {
                session,
                pairing_required,
                filter,
                history_bytes,
            } => {
                enc.u8(tag::WELCOME);
                crate::Record::Session(session.clone()).encode(&mut enc);
                enc.bool(*pairing_required).str(filter).uvar(*history_bytes);
            }
            ToViewer::Paired => {
                enc.u8(tag::PAIRED);
            }
            ToViewer::PairRejected { reason } => {
                enc.u8(tag::PAIR_REJECTED).str(reason);
            }
            ToViewer::Chunk { seq, bytes } => {
                enc.u8(tag::CHUNK).uvar(*seq).bytes(bytes);
            }
            ToViewer::HistoryEnd => {
                enc.u8(tag::HISTORY_END);
            }
            ToViewer::Filter { filter } => {
                enc.u8(tag::FILTER).str(filter);
            }
            ToViewer::DeviceRecording {
                offset,
                total,
                bytes,
            } => {
                enc.u8(tag::DEVICE_RECORDING)
                    .uvar(*offset)
                    .uvar(*total)
                    .bytes(bytes);
            }
            ToViewer::Pong { nonce } => {
                enc.u8(tag::PONG).uvar(*nonce);
            }
        }
        enc.into_inner()
    }

    /// # Errors
    /// Any [`DecodeError`] for malformed input.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut dec = Decoder::new(bytes);
        Ok(match dec.u8()? {
            tag::WELCOME => {
                let crate::Record::Session(session) = crate::Record::decode(&mut dec)? else {
                    return Err(DecodeError::InvalidTag {
                        what: "welcome session",
                        tag: 0,
                    });
                };
                ToViewer::Welcome {
                    session,
                    pairing_required: dec.bool()?,
                    filter: dec.string()?,
                    history_bytes: dec.uvar()?,
                }
            }
            tag::PAIRED => ToViewer::Paired,
            tag::PAIR_REJECTED => ToViewer::PairRejected {
                reason: dec.string()?,
            },
            tag::CHUNK => ToViewer::Chunk {
                seq: dec.uvar()?,
                bytes: dec.bytes()?.to_vec(),
            },
            tag::HISTORY_END => ToViewer::HistoryEnd,
            tag::FILTER => ToViewer::Filter {
                filter: dec.string()?,
            },
            tag::DEVICE_RECORDING => ToViewer::DeviceRecording {
                offset: dec.uvar()?,
                total: dec.uvar()?,
                bytes: dec.bytes()?.to_vec(),
            },
            tag::PONG => ToViewer::Pong { nonce: dec.uvar()? },
            other => {
                return Err(DecodeError::InvalidTag {
                    what: "viewer message",
                    tag: other,
                });
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let beacon = Beacon {
            format_version: 1,
            sismo_version: "0.1.0".into(),
            session_id: 5,
            app: "demo".into(),
            app_version: "1.0".into(),
            platform: "android".into(),
            device: "Pixel 7".into(),
            port: 7720,
            auth: AuthMode::Pairing,
            fingerprint: "ab12".into(),
        };
        assert_eq!(Beacon::decode(&beacon.encode()).unwrap(), beacon);

        for msg in [
            ToRecorder::Hello {
                viewer: "pc".into(),
            },
            ToRecorder::Pair {
                code: "123456".into(),
            },
            ToRecorder::Subscribe {
                history: true,
                since: 41,
            },
            ToRecorder::SetFilter {
                filter: "info".into(),
            },
            ToRecorder::Ping { nonce: 3 },
            ToRecorder::Bye,
        ] {
            assert_eq!(ToRecorder::decode(&msg.encode()).unwrap(), msg);
        }
        for msg in [
            ToViewer::Welcome {
                session: SessionInfo::default(),
                pairing_required: true,
                filter: "info".into(),
                history_bytes: 10,
            },
            ToViewer::Chunk {
                seq: 7,
                bytes: vec![1, 2, 3],
            },
            ToViewer::DeviceRecording {
                offset: 1,
                total: 2,
                bytes: vec![4],
            },
            ToViewer::Pong { nonce: 9 },
        ] {
            assert_eq!(ToViewer::decode(&msg.encode()).unwrap(), msg);
        }
    }
}
