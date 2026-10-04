//! Four-byte unsigned big-endian length prefix followed by one payload.

use crate::error::{Error, Result};

const HEADER_LENGTH: usize = 4;
/// Payload buffers grow as bytes arrive; a declared length never drives allocation.
const INITIAL_BLOCK: usize = 64 * 1024;

pub const DEFAULT_MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

fn err(message: impl Into<String>) -> Error {
    Error::Frame(message.into())
}

/// Prefix a payload with its length.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>> {
    let length = u32::try_from(payload.len())
        .map_err(|_| err("Frame payload exceeds the unsigned 32-bit length limit"))?;
    let mut frame = Vec::with_capacity(HEADER_LENGTH + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Open,
    Ended,
    Failed,
}

/// Incrementally splits arbitrary byte chunks into payloads. Chunk boundaries never matter:
/// frames may be fragmented or coalesced arbitrarily.
#[derive(Debug)]
pub struct FrameDecoder {
    max_frame_length: usize,
    header: [u8; HEADER_LENGTH],
    header_length: usize,
    expected: Option<usize>,
    payload: Vec<u8>,
    state: State,
}

impl FrameDecoder {
    pub fn new(max_frame_length: usize) -> Self {
        FrameDecoder {
            max_frame_length,
            header: [0; HEADER_LENGTH],
            header_length: 0,
            expected: None,
            payload: Vec::new(),
            state: State::Open,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>> {
        match self.state {
            State::Ended => return Err(err("Frame decoder has ended")),
            State::Failed => return Err(err("Frame decoder has failed")),
            State::Open => {}
        }
        let mut frames = Vec::new();
        let mut offset = 0;
        while offset < chunk.len() {
            let expected = match self.expected {
                Some(expected) => expected,
                None => {
                    let take = (HEADER_LENGTH - self.header_length).min(chunk.len() - offset);
                    self.header[self.header_length..self.header_length + take]
                        .copy_from_slice(&chunk[offset..offset + take]);
                    self.header_length += take;
                    offset += take;
                    if self.header_length < HEADER_LENGTH {
                        continue;
                    }
                    self.header_length = 0;
                    let length = u32::from_be_bytes(self.header) as usize;
                    if length > self.max_frame_length {
                        return Err(self.fail(format!(
                            "Frame length {length} exceeds configured limit of {}",
                            self.max_frame_length
                        )));
                    }
                    if length == 0 {
                        frames.push(Vec::new());
                        continue;
                    }
                    self.expected = Some(length);
                    self.payload = Vec::with_capacity(length.min(INITIAL_BLOCK));
                    length
                }
            };
            let take = (expected - self.payload.len()).min(chunk.len() - offset);
            self.payload
                .extend_from_slice(&chunk[offset..offset + take]);
            offset += take;
            if self.payload.len() == expected {
                frames.push(std::mem::take(&mut self.payload));
                self.expected = None;
            }
        }
        Ok(frames)
    }

    /// Declare end of stream; a partial frame is an error.
    pub fn end(&mut self) -> Result<()> {
        match self.state {
            State::Ended => return Err(err("Frame decoder has ended")),
            State::Failed => return Err(err("Frame decoder has failed")),
            State::Open => {}
        }
        if self.header_length != 0 || self.expected.is_some() {
            return Err(self.fail("Truncated frame at end of stream".to_string()));
        }
        self.state = State::Ended;
        Ok(())
    }

    fn fail(&mut self, message: String) -> Error {
        self.state = State::Failed;
        self.header_length = 0;
        self.expected = None;
        self.payload = Vec::new();
        err(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoder() -> FrameDecoder {
        FrameDecoder::new(DEFAULT_MAX_FRAME_LENGTH)
    }

    #[test]
    fn prefixes_payloads_with_a_four_byte_big_endian_length() {
        assert_eq!(
            encode_frame(&[0xaa, 0xbb, 0xcc]).unwrap(),
            [0, 0, 0, 3, 0xaa, 0xbb, 0xcc]
        );
        assert_eq!(encode_frame(&[]).unwrap(), [0, 0, 0, 0]);
    }

    #[test]
    fn decodes_fragmented_coalesced_and_empty_frames_in_order() {
        let mut wire = Vec::new();
        wire.extend(encode_frame(&[1, 2, 3]).unwrap());
        wire.extend(encode_frame(&[]).unwrap());
        wire.extend(encode_frame(&[4]).unwrap());
        let expected = vec![vec![1, 2, 3], vec![], vec![4]];

        let mut one_byte = decoder();
        let mut frames = Vec::new();
        for byte in &wire {
            frames.extend(one_byte.push(&[*byte]).unwrap());
        }
        one_byte.end().unwrap();
        assert_eq!(frames, expected);

        let mut coalesced = decoder();
        assert_eq!(coalesced.push(&wire).unwrap(), expected);
        coalesced.end().unwrap();
    }

    #[test]
    fn handles_every_split_point() {
        let wire = encode_frame(&[10, 20, 30, 40]).unwrap();
        for split in 0..=wire.len() {
            let mut d = decoder();
            let mut frames = d.push(&wire[..split]).unwrap();
            frames.extend(d.push(&wire[split..]).unwrap());
            d.end().unwrap();
            assert_eq!(frames, vec![vec![10, 20, 30, 40]], "split {split}");
        }
    }

    #[test]
    fn assembles_payloads_larger_than_one_internal_block() {
        let payload: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let wire = encode_frame(&payload).unwrap();
        let mut d = decoder();
        let mut frames = d.push(&wire[..101]).unwrap();
        frames.extend(d.push(&wire[101..65_541]).unwrap());
        frames.extend(d.push(&wire[65_541..]).unwrap());
        d.end().unwrap();
        assert_eq!(frames, vec![payload]);
    }

    #[test]
    fn accepts_empty_chunks_and_a_clean_empty_stream() {
        let mut d = decoder();
        assert!(d.push(&[]).unwrap().is_empty());
        d.end().unwrap();
    }

    #[test]
    fn rejects_a_truncated_stream_at_end() {
        for wire in [&[0u8, 0, 0][..], &[0, 0, 0, 2, 1][..]] {
            let mut d = decoder();
            assert!(d.push(wire).unwrap().is_empty());
            assert!(matches!(d.end(), Err(Error::Frame(_))));
        }
    }

    #[test]
    fn rejects_an_oversized_declared_length_once_the_header_is_complete() {
        let mut d = FrameDecoder::new(3);
        assert!(
            d.push(&[0, 0, 0, 4])
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
        assert!(d.push(&[1]).unwrap_err().to_string().contains("failed"));
    }

    #[test]
    fn a_huge_declared_length_does_not_allocate() {
        let mut d = FrameDecoder::new(usize::MAX);
        assert!(d.push(&[0xff, 0xff, 0xff, 0xff, 1]).unwrap().is_empty());
        assert!(d.payload.capacity() <= INITIAL_BLOCK);
    }

    #[test]
    fn accepts_a_frame_exactly_at_the_maximum() {
        let mut d = FrameDecoder::new(3);
        assert_eq!(
            d.push(&encode_frame(&[1, 2, 3]).unwrap()).unwrap(),
            vec![vec![1, 2, 3]]
        );
        d.end().unwrap();
    }

    #[test]
    fn cannot_be_pushed_after_end() {
        let mut d = decoder();
        d.end().unwrap();
        assert!(d.push(&[]).unwrap_err().to_string().contains("ended"));
        assert!(d.end().unwrap_err().to_string().contains("ended"));
    }
}
