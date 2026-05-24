use crate::checksum::{raw_part_name, SwiftyError, RAW_PART_SIZE};
use crate::model::{Md5Digest, SrfPart};
use crate::pbo::{swifty_pbo_part_plan_from_prefix, SwiftyPboPartPlan};

const MAX_PBO_HEADER_PREFIX: usize = 1024 * 1024;

pub struct SwiftyStreamingPartScanner {
    file_path: String,
    file_len: u64,
    consumed: u64,
    mode: ScannerMode,
}

pub struct SwiftyStreamingPartValidator {
    expected: Md5Digest,
    expected_len: u64,
    seen: u64,
    ctx: md5::Context,
}

impl SwiftyStreamingPartValidator {
    pub fn new(expected: Md5Digest, expected_len: u64) -> Self {
        Self {
            expected,
            expected_len,
            seen: 0,
            ctx: md5::Context::new(),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<(), SwiftyError> {
        self.seen = self.seen.checked_add(bytes.len() as u64).ok_or({
            SwiftyError::InvalidPartLength {
                expected: self.expected_len,
                actual: u64::MAX,
            }
        })?;
        if self.seen > self.expected_len {
            return Err(SwiftyError::InvalidPartLength {
                expected: self.expected_len,
                actual: self.seen,
            });
        }
        self.ctx.consume(bytes);
        Ok(())
    }

    pub fn finish(self) -> Result<u64, SwiftyError> {
        if self.seen != self.expected_len {
            return Err(SwiftyError::InvalidPartLength {
                expected: self.expected_len,
                actual: self.seen,
            });
        }
        let digest = self.ctx.finalize();
        if Md5Digest::from_bytes(digest.0) != self.expected {
            return Err(SwiftyError::PartChecksumMismatch);
        }
        Ok(self.seen)
    }
}

enum ScannerMode {
    Detect { prefix: Vec<u8> },
    Raw(RawScanner),
    Planned(PlannedScanner),
}

impl SwiftyStreamingPartScanner {
    pub fn new(file_path: &str, file_len: u64) -> Self {
        let extension = std::path::Path::new(file_path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mode = if extension == "pbo" {
            ScannerMode::Detect { prefix: Vec::new() }
        } else {
            ScannerMode::Raw(RawScanner::new(file_path))
        };
        Self {
            file_path: file_path.to_owned(),
            file_len,
            consumed: 0,
            mode,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SrfPart>, SwiftyError> {
        let next_consumed = self
            .consumed
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| SwiftyError::InvalidPbo {
                file: "stream.pbo".to_string(),
                reason: "stream length overflow".to_string(),
            })?;
        if next_consumed > self.file_len {
            return Err(SwiftyError::InvalidPbo {
                file: "stream.pbo".to_string(),
                reason: "stream exceeded declared file length".to_string(),
            });
        }
        self.consumed = next_consumed;

        match &mut self.mode {
            ScannerMode::Detect { prefix } => {
                prefix.extend_from_slice(bytes);
                match swifty_pbo_part_plan_from_prefix(&self.file_path, prefix, self.file_len) {
                    Ok(Some(plan)) => {
                        let mut scanner = PlannedScanner::new(plan);
                        let out = scanner.push(prefix)?;
                        self.mode = ScannerMode::Planned(scanner);
                        Ok(out)
                    }
                    Ok(None) if prefix.len() <= MAX_PBO_HEADER_PREFIX => Ok(Vec::new()),
                    Ok(None) => Err(SwiftyError::InvalidPbo {
                        file: self.file_path.clone(),
                        reason: "PBO header exceeded streaming prefix limit".to_string(),
                    }),
                    Err(SwiftyError::InvalidPbo { .. }) => {
                        let mut scanner = RawScanner::new(&self.file_path);
                        let out = scanner.push(prefix);
                        self.mode = ScannerMode::Raw(scanner);
                        Ok(out)
                    }
                    Err(error) => Err(error),
                }
            }
            ScannerMode::Raw(scanner) => Ok(scanner.push(bytes)),
            ScannerMode::Planned(scanner) => scanner.push(bytes),
        }
    }

    pub fn finish(mut self) -> Result<Vec<SrfPart>, SwiftyError> {
        if self.consumed != self.file_len {
            return Err(SwiftyError::InvalidPbo {
                file: "stream.pbo".to_string(),
                reason: "stream ended before declared file length".to_string(),
            });
        }
        match &mut self.mode {
            ScannerMode::Detect { prefix } => {
                match swifty_pbo_part_plan_from_prefix(&self.file_path, prefix, self.file_len)? {
                    Some(plan) => {
                        let mut scanner = PlannedScanner::new(plan);
                        let mut out = scanner.push(prefix)?;
                        out.extend(scanner.finish()?);
                        Ok(out)
                    }
                    None => {
                        let mut scanner = RawScanner::new(&self.file_path);
                        Ok(scanner.finish(prefix))
                    }
                }
            }
            ScannerMode::Raw(scanner) => Ok(scanner.finish(&[])),
            ScannerMode::Planned(scanner) => scanner.finish(),
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.consumed
    }
}

struct RawScanner {
    file_name: String,
    offset: u64,
    pending: Vec<u8>,
}

impl RawScanner {
    fn new(file_path: &str) -> Self {
        let file_name = std::path::Path::new(file_path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("file")
            .to_owned();
        Self {
            file_name,
            offset: 0,
            pending: Vec::with_capacity(RAW_PART_SIZE as usize),
        }
    }

    fn push(&mut self, mut bytes: &[u8]) -> Vec<SrfPart> {
        let mut out = Vec::new();
        while !bytes.is_empty() {
            let need = RAW_PART_SIZE as usize - self.pending.len();
            let take = need.min(bytes.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.pending.len() == RAW_PART_SIZE as usize {
                out.push(self.emit_pending());
            }
        }
        out
    }

    fn finish(&mut self, bytes: &[u8]) -> Vec<SrfPart> {
        let mut out = self.push(bytes);
        if !self.pending.is_empty() {
            out.push(self.emit_pending());
        }
        out
    }

    fn emit_pending(&mut self) -> SrfPart {
        let length = self.pending.len() as u64;
        let start = self.offset;
        self.offset += length;
        let end = self.offset;
        let digest = Md5Digest::from_bytes(md5::compute(&self.pending).0);
        self.pending.clear();
        SrfPart {
            path: raw_part_name(&self.file_name, end),
            start,
            length,
            checksum: digest,
        }
    }
}

struct PlannedScanner {
    plan: Vec<SwiftyPboPartPlan>,
    index: usize,
    offset: u64,
    ctx: md5::Context,
}

impl PlannedScanner {
    fn new(plan: Vec<SwiftyPboPartPlan>) -> Self {
        Self {
            plan,
            index: 0,
            offset: 0,
            ctx: md5::Context::new(),
        }
    }

    fn push(&mut self, mut bytes: &[u8]) -> Result<Vec<SrfPart>, SwiftyError> {
        let mut out = Vec::new();
        while !bytes.is_empty() {
            while self
                .plan
                .get(self.index)
                .is_some_and(|part| part.length == 0)
            {
                out.push(self.emit_current());
            }
            let Some(part) = self.plan.get(self.index) else {
                return Err(SwiftyError::InvalidPbo {
                    file: "stream.pbo".to_string(),
                    reason: "more bytes than planned PBO parts".to_string(),
                });
            };
            let part_end = part.start + part.length;
            let remaining = (part_end - self.offset) as usize;
            let take = remaining.min(bytes.len());
            self.ctx.consume(&bytes[..take]);
            self.offset += take as u64;
            bytes = &bytes[take..];
            if self.offset == part_end {
                out.push(self.emit_current());
            }
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<SrfPart>, SwiftyError> {
        let mut out = Vec::new();
        while self.index < self.plan.len() {
            let part = &self.plan[self.index];
            if part.start + part.length != self.offset {
                return Err(SwiftyError::InvalidPbo {
                    file: "stream.pbo".to_string(),
                    reason: "stream ended before planned PBO part completed".to_string(),
                });
            }
            out.push(self.emit_current());
        }
        Ok(out)
    }

    fn emit_current(&mut self) -> SrfPart {
        let part = &self.plan[self.index];
        let digest = std::mem::replace(&mut self.ctx, md5::Context::new()).finalize();
        self.index += 1;
        SrfPart {
            path: part.path.clone(),
            start: part.start,
            length: part.length,
            checksum: Md5Digest::from_bytes(digest.0),
        }
    }
}
