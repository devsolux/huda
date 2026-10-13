//! Bounded newline-delimited UTF-8 JSON. No unbounded read_line allocation.
use crate::error::{require, Error, Result};
use serde_json::Value;
use std::io::{BufRead, Write};
pub fn read_frame(reader: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>> {
    let mut output = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if output.is_empty() {
                Ok(None)
            } else {
                Err(Error::invalid("Incomplete JSON frame at EOF"))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |at| at + 1);
        require(
            output.len() + take <= limit,
            "LIMIT_EXCEEDED",
            "JSON frame exceeds the byte limit",
        )?;
        output.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline.is_some() {
            output.pop();
            if output.last() == Some(&b'\r') {
                output.pop();
            }
            require(
                !output.is_empty(),
                "INVALID_INPUT",
                "Empty JSON frames are not allowed",
            )?;
            std::str::from_utf8(&output).map_err(|_| Error::invalid("JSON frame is not UTF-8"))?;
            return Ok(Some(output));
        }
    }
}
pub fn write_frame(writer: &mut impl Write, value: &Value, limit: usize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    require(
        bytes.len() < limit,
        "LIMIT_EXCEEDED",
        "JSON response exceeds the byte limit",
    )?;
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn bounded_frames_and_eof() {
        assert_eq!(
            read_frame(&mut Cursor::new(b"{}\n"), 4).unwrap(),
            Some(b"{}".to_vec())
        );
        assert!(read_frame(&mut Cursor::new(b"12345\n"), 4).is_err());
        assert!(read_frame(&mut Cursor::new(b"{}"), 4).is_err());
        assert_eq!(read_frame(&mut Cursor::new(b""), 4).unwrap(), None);
    }
}
