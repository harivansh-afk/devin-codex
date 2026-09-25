use super::{GatewayError, protocol};
use bytes::{Buf, BytesMut};
use std::io::Read;

#[cfg(test)]
#[path = "../../tests/unit/connect.rs"]
mod tests;

pub(super) fn frame(payload: &[u8], flags: u8) -> Vec<u8> {
    let mut output = Vec::with_capacity(payload.len() + 5);
    output.push(flags);
    output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    output.extend_from_slice(payload);
    output
}

pub(super) struct ConnectParser {
    buffer: BytesMut,
    limit: usize,
    gzip: bool,
    ended: bool,
}

impl ConnectParser {
    pub fn new(limit: usize, gzip: bool) -> Self {
        Self {
            buffer: BytesMut::new(),
            limit,
            gzip,
            ended: false,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, GatewayError> {
        if self.ended && !bytes.is_empty() {
            return Err(protocol());
        }
        self.buffer.extend_from_slice(bytes);
        let mut output = Vec::new();
        while self.buffer.len() >= 5 {
            let flags = self.buffer[0];
            let length = u32::from_be_bytes(self.buffer[1..5].try_into().unwrap()) as usize;
            if flags & !3 != 0 || length > self.limit {
                return Err(protocol());
            }
            if self.buffer.len() < length + 5 {
                break;
            }
            self.buffer.advance(5);
            let mut payload = self.buffer.split_to(length).to_vec();
            if flags & 1 != 0 {
                if !self.gzip {
                    return Err(protocol());
                }
                let mut decoder = flate2::bufread::GzDecoder::new(payload.as_slice());
                let mut inflated = Vec::new();
                decoder
                    .by_ref()
                    .take(self.limit as u64 + 1)
                    .read_to_end(&mut inflated)
                    .map_err(|_| protocol())?;
                if inflated.len() > self.limit || !decoder.get_ref().is_empty() {
                    return Err(protocol());
                }
                payload = inflated;
            }
            if flags & 2 != 0 {
                validate_trailer(&payload)?;
                self.ended = true;
                if !self.buffer.is_empty() {
                    return Err(protocol());
                }
            } else {
                output.push(payload);
            }
        }
        Ok(output)
    }

    pub fn finish(&self) -> Result<(), GatewayError> {
        if !self.ended {
            Err(GatewayError::new("upstream_truncated_stream"))
        } else if !self.buffer.is_empty() {
            Err(protocol())
        } else {
            Ok(())
        }
    }
}

fn validate_trailer(payload: &[u8]) -> Result<(), GatewayError> {
    let trailer: serde_json::Value = serde_json::from_slice(payload).map_err(|_| protocol())?;
    let object = trailer.as_object().ok_or_else(protocol)?;
    if let Some(error) = object.get("error") {
        let code = error
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(protocol)?;
        if !matches!(
            code,
            "canceled"
                | "unknown"
                | "invalid_argument"
                | "deadline_exceeded"
                | "not_found"
                | "already_exists"
                | "permission_denied"
                | "resource_exhausted"
                | "failed_precondition"
                | "aborted"
                | "out_of_range"
                | "unimplemented"
                | "internal"
                | "unavailable"
                | "data_loss"
                | "unauthenticated"
        ) {
            return Err(protocol());
        }
        return Err(GatewayError::trailer(
            code,
            error.get("message").and_then(|v| v.as_str()),
        ));
    }
    Ok(())
}
