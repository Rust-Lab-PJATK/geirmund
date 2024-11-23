use std::sync::LazyLock;

use thiserror::Error;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const STARTING_PAYLOAD: LazyLock<String> = LazyLock::new(|| format!("RLPG/{VERSION}\n"));

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RLPGParserState {
    ParsingHeaderStart,                        // "RLPG/0.1.0\n"
    ParsingContentLengthInHeader(Vec<u8>),     // length in u32 + \n
    WaitingForNewlineBetweenHeaderAndBody(u8), // \n
    ParsingBody(Vec<u8>),                      // bytes
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RLPGParser {
    state: RLPGParserState,
    buffer: Vec<u8>,
    current_index: usize,
    body_length: Option<usize>,
}

#[derive(Debug, Error)]
pub enum ParsingError {
    #[error("invalid byte ({0}) occured at packet buffer position: {1}")]
    InvalidByte(u8, usize),

    #[error("invalid utf8 string occured while parsing: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),

    #[error("Invalid U32 value: {0:?}")]
    InvalidU32(Vec<u8>),
}

impl RLPGParser {
    pub fn new() -> Self {
        Self {
            state: RLPGParserState::ParsingHeaderStart,
            buffer: Vec::new(),
            current_index: 0,
            body_length: None,
        }
    }

    pub fn add_to_buffer(&mut self, payload: &[u8]) {
        for ele in payload {
            self.buffer.push(*ele);
        }
    }

    fn parse_header_start(&mut self) -> Result<bool, ParsingError> {
        let starting_payload_str = STARTING_PAYLOAD.clone();
        let starting_payload = starting_payload_str.as_bytes();

        if self.current_index > starting_payload.len() {
            return Ok(false);
        }

        while self.current_index < self.buffer.len() && self.current_index < STARTING_PAYLOAD.len()
        {
            if starting_payload[self.current_index] != self.buffer[self.current_index] {
                return Err(ParsingError::InvalidByte(
                    self.buffer[self.current_index],
                    self.current_index,
                ));
            }

            self.current_index += 1;
        }

        assert!(
            self.current_index <= STARTING_PAYLOAD.len(),
            "Current index must not be greater than length of starting payload."
        );

        Ok(STARTING_PAYLOAD.len() == self.current_index)
    }

    fn parse_content_length_in_header(&mut self) -> Result<bool, ParsingError> {
        while self.current_index < self.buffer.len() {
            if self.buffer[self.current_index] == b'\n' {
                let bytes = match self.state {
                    RLPGParserState::ParsingContentLengthInHeader(ref vec) => vec,
                    _ => panic!(
                        "Expected ParsingContentLengthInHeader state, received: {:?}",
                        self.state
                    ),
                };

                let content_length: usize = String::from_utf8(bytes.clone())?
                    .parse()
                    .map_err(|_| ParsingError::InvalidU32(bytes.clone()))?;

                self.body_length = Some(content_length);

                return Ok(true);
            } else if !(self.buffer[self.current_index] >= b'0'
                && self.buffer[self.current_index] <= b'9')
            {
                return Err(ParsingError::InvalidByte(
                    self.buffer[self.current_index],
                    self.current_index,
                ));
            }

            match self.state {
                RLPGParserState::ParsingContentLengthInHeader(ref mut vec) => {
                    vec.push(self.buffer[self.current_index]);
                }
                _ => panic!(
                    "Invalid state: {:?}, expected ParsingContentLengthInHeader",
                    self.state
                ),
            };

            self.current_index += 1;
        }

        Ok(false)
    }

    fn parse_newline_between_header_and_body(&mut self) -> Result<bool, ParsingError> {
        let occured_times = match self.state {
            RLPGParserState::WaitingForNewlineBetweenHeaderAndBody(ref mut occured_times) => {
                occured_times
            }
            _ => panic!(
                "Expected WaitingForNewlineBetweenHeaderAndBody, received: {:?}",
                self.state
            ),
        };

        while *occured_times != 2 && self.buffer.len() > self.current_index {
            if self.buffer[self.current_index] != b'\n' {
                dbg!(self.buffer[self.current_index] as char, *occured_times);
                return Err(ParsingError::InvalidByte(
                    self.buffer[self.current_index],
                    self.current_index,
                ));
            }

            self.current_index += 1;
            *occured_times += 1;
        }

        assert!(
            *occured_times <= 2,
            "Occured times must not be greater than 2."
        );

        if *occured_times == 2 {
            return Ok(true);
        }

        Ok(false)
    }

    fn parse_body(&mut self) -> Result<bool, ParsingError> {
        if let Some(body_length) = self.body_length {
            let remaining_bytes = self.buffer.len() - self.current_index;

            if remaining_bytes >= body_length {
                let body =
                    self.buffer[self.current_index..self.current_index + body_length].to_vec();
                self.current_index += body_length;

                match self.state {
                    RLPGParserState::ParsingBody(ref mut vec) => {
                        vec.extend_from_slice(&body);
                    }
                    _ => panic!("Expected ParsingBody state, received: {:?}", self.state),
                };

                self.buffer = self.buffer.split_off(self.current_index);

                return Ok(true);
            }
        }

        return Ok(false);
    }

    pub fn parse(&mut self) -> Result<Option<Vec<u8>>, ParsingError> {
        loop {
            match self.state {
                RLPGParserState::ParsingHeaderStart => {
                    if self.parse_header_start()? {
                        self.state = RLPGParserState::ParsingContentLengthInHeader(Vec::new());
                        continue;
                    }
                }
                RLPGParserState::ParsingContentLengthInHeader(_) => {
                    if self.parse_content_length_in_header()? {
                        self.state = RLPGParserState::WaitingForNewlineBetweenHeaderAndBody(0);
                        continue;
                    }
                }
                RLPGParserState::WaitingForNewlineBetweenHeaderAndBody(_) => {
                    if self.parse_newline_between_header_and_body()? {
                        self.state = RLPGParserState::ParsingBody(Vec::new());
                        continue;
                    }
                }
                RLPGParserState::ParsingBody(_) => {
                    if self.parse_body()? {
                        return Ok(Some(match self.state {
                            RLPGParserState::ParsingBody(ref body) => body.clone(),
                            _ => panic!("No way, no fucking way"),
                        }));
                    }
                }
            };

            return Ok(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{RLPGParser, RLPGParserState, VERSION};

    #[test]
    pub fn test_if_parse_header_start_works_on_two_batches() {
        let mut parser = RLPGParser::new();

        const VERSION: &str = env!("CARGO_PKG_VERSION");

        parser.add_to_buffer(b"RLPG/");

        assert!(matches!(parser.parse_header_start(), Ok(false)));

        parser.add_to_buffer(format!("{VERSION}\n200").as_bytes());

        assert!(matches!(parser.parse_header_start(), Ok(true)));
        assert!(parser.current_index == format!("RLPG/{VERSION}\n").len());
    }

    #[test]
    pub fn test_if_parse_header_start_does_not_do_anything_when_it_has_already_completed_its_job() {
        let mut parser = RLPGParser::new();

        const VERSION: &str = env!("CARGO_PKG_VERSION");

        parser.add_to_buffer(format!("RLPG/{VERSION}\n").as_bytes());

        assert!(matches!(parser.parse_header_start(), Ok(true)));
        assert!(parser.current_index == format!("RLPG/{VERSION}\n").len());

        assert!(matches!(parser.parse_header_start(), Ok(true)));
        assert!(parser.current_index == format!("RLPG/{VERSION}\n").len());
    }

    #[test]
    pub fn test_if_parse_content_length_in_header_works() {
        let mut parser = RLPGParser::new();

        const VERSION: &str = env!("CARGO_PKG_VERSION");

        parser.add_to_buffer(format!("RLPG/{VERSION}\n").as_bytes());

        assert!(matches!(parser.parse_header_start(), Ok(true)));
        parser.state = RLPGParserState::ParsingContentLengthInHeader(Vec::new());
        assert!(parser.current_index == format!("RLPG/{VERSION}\n").len());

        parser.add_to_buffer(b"200\n");

        assert!(matches!(parser.parse_content_length_in_header(), Ok(true)));
        assert!(parser.body_length == Some(200));
        assert!(parser.current_index == format!("RLPG/{VERSION}\n200\n").len() - 1);
    }

    #[test]
    pub fn test_if_parse_content_length_header_returns_invalid_byte() {
        let mut parser = RLPGParser::new();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n").as_bytes());

        assert!(matches!(parser.parse_header_start(), Ok(true)));
        parser.state = RLPGParserState::ParsingContentLengthInHeader(Vec::new());
        assert!(parser.current_index == format!("RLPG/{VERSION}\n").len());

        parser.add_to_buffer(b"20s0\n");

        assert!(matches!(
            parser.parse_content_length_in_header(),
            Err(crate::ParsingError::InvalidByte(_, _))
        ));
    }

    #[test]
    pub fn test_if_parse_content_length_header_works_correctly_in_two_batches() {
        let mut parser = RLPGParser::new();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n").as_bytes());

        assert!(matches!(parser.parse_header_start(), Ok(true)));
        parser.state = RLPGParserState::ParsingContentLengthInHeader(Vec::new());
        assert!(parser.current_index == format!("RLPG/{VERSION}\n").len());

        parser.add_to_buffer(b"20");

        assert!(matches!(parser.parse_content_length_in_header(), Ok(false)));
        assert!(matches!(parser.body_length, None));
        parser.add_to_buffer(b"25\n");

        assert!(matches!(parser.parse_content_length_in_header(), Ok(true)));
        assert!(matches!(parser.body_length, Some(2025)));
    }

    #[test]
    pub fn test_if_parse_newline_between_header_and_body_works_correctly() {
        let mut parser = RLPGParser::new();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n200").as_bytes());
        parser.state = RLPGParserState::WaitingForNewlineBetweenHeaderAndBody(0);
        parser.current_index = parser.buffer.len();
        parser.add_to_buffer("\n\n".as_bytes());

        assert!(matches!(
            parser.parse_newline_between_header_and_body(),
            Ok(true)
        ));
    }

    #[test]
    pub fn test_if_parse_newline_between_header_and_body_works_correctly_on_two_batches() {
        let mut parser = RLPGParser::new();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n200").as_bytes());
        parser.state = RLPGParserState::WaitingForNewlineBetweenHeaderAndBody(0);
        parser.current_index = parser.buffer.len();
        parser.add_to_buffer("\n".as_bytes());

        assert!(matches!(
            parser.parse_newline_between_header_and_body(),
            Ok(false)
        ));

        parser.add_to_buffer("\n".as_bytes());

        assert!(matches!(
            parser.parse_newline_between_header_and_body(),
            Ok(true)
        ));
    }

    #[test]
    pub fn test_if_parse_newline_returns_false_and_does_not_do_anything_when_passed_nothing_new() {
        let mut parser = RLPGParser::new();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n200").as_bytes());
        parser.state = RLPGParserState::WaitingForNewlineBetweenHeaderAndBody(0);
        parser.current_index = parser.buffer.len();

        let old_parser = parser.clone();

        assert!(matches!(
            parser.parse_newline_between_header_and_body(),
            Ok(false)
        ));

        assert!(old_parser == parser)
    }

    #[test]
    pub fn test_if_parse_body_works_correctly() {
        let mut parser = RLPGParser::new();

        let body = b"Hello, World!";
        let body_len = body.len();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n{body_len}\n\n").as_bytes());
        parser.state = RLPGParserState::ParsingBody(Vec::new());
        parser.current_index = parser.buffer.len();
        parser.body_length = Some(body_len as usize);

        parser.add_to_buffer(body);

        assert!(matches!(parser.parse_body(), Ok(true)));
        assert!(
            matches!(parser.state, RLPGParserState::ParsingBody(vec) if vec == b"Hello, World!".to_vec())
        );
    }

    #[test]
    pub fn test_if_parse_body_works_correctly_in_two_batches() {
        let mut parser = RLPGParser::new();

        let body = b"Hello, World!";
        let body_len = body.len();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n{body_len}\n\n").as_bytes());
        parser.state = RLPGParserState::ParsingBody(Vec::new());
        parser.current_index = parser.buffer.len();
        parser.body_length = Some(body_len as usize);

        parser.add_to_buffer(&body[..5]);

        assert!(matches!(parser.parse_body(), Ok(false)));
        assert!(matches!(parser.state, RLPGParserState::ParsingBody(ref vec) if vec.len() == 0));

        parser.add_to_buffer(&body[5..]);

        assert!(matches!(parser.parse_body(), Ok(true)));
        assert!(
            matches!(parser.state, RLPGParserState::ParsingBody(ref vec) if *vec == b"Hello, World!".to_vec())
        );
    }

    #[test]
    pub fn test_if_parse_body_works_correctly_when_passed_too_many_bytes() {
        let mut parser = RLPGParser::new();

        let body = b"Hello, World!";
        let body_len = body.len();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n{body_len}\n\n").as_bytes());
        parser.state = RLPGParserState::ParsingBody(Vec::new());
        parser.current_index = parser.buffer.len();
        parser.body_length = Some(body_len as usize);

        parser.add_to_buffer(body);
        parser.add_to_buffer(b"Extra bytes");

        assert!(matches!(parser.parse_body(), Ok(true)));
        assert!(
            matches!(parser.state, RLPGParserState::ParsingBody(ref vec) if *vec == b"Hello, World!".to_vec())
        );
    }

    #[test]
    pub fn test_if_parse_body_cuts_already_parsed_data_from_buffer() {
        let mut parser = RLPGParser::new();

        let body = b"Hello, World!";
        let body_len = body.len();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n{body_len}\n\n").as_bytes());
        parser.state = RLPGParserState::ParsingBody(Vec::new());
        parser.current_index = parser.buffer.len();
        parser.body_length = Some(body_len as usize);

        parser.add_to_buffer(body);
        parser.add_to_buffer(b"Extra bytes");

        assert!(matches!(parser.parse_body(), Ok(true)));
        assert!(
            matches!(parser.state, RLPGParserState::ParsingBody(ref vec) if *vec == b"Hello, World!".to_vec())
        );

        assert!(parser.buffer == b"Extra bytes".to_vec());
    }

    #[test]
    pub fn test_if_parse_works_correctly() {
        let mut parser = RLPGParser::new();

        let body = b"Hello, World!";
        let body_len = body.len();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n{body_len}\n\n").as_bytes());
        parser.add_to_buffer(body);

        let result = parser.parse();

        assert!(matches!(result, Ok(Some(vec)) if vec == b"Hello, World!"));
    }

    #[test]
    pub fn test_if_parse_works_correctly_in_batches() {
        let mut parser = RLPGParser::new();

        let body = b"Hello, World!";
        let body_len = body.len();

        parser.add_to_buffer(format!("RLPG/{VERSION}\n{body_len}\n\n").as_bytes());
        assert!(matches!(parser.parse(), Ok(None)));

        parser.add_to_buffer(&body[..5]);
        assert!(matches!(parser.parse(), Ok(None)));

        parser.add_to_buffer(&body[5..]);
        assert!(matches!(parser.parse(), Ok(Some(ref vec)) if vec == b"Hello, World!"));
    }
}
