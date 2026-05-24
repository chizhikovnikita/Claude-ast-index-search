//! Read files into UTF-8 strings with auto-detection of legacy encodings.
//!
//! The project's source pipeline (tree-sitter and regex parsers) expects valid
//! UTF-8 `&str`. Many legacy codebases ship files in Windows-1251 or other
//! 8-bit encodings; `fs::read_to_string` rejects them outright. This module
//! provides a drop-in replacement: try UTF-8 first (zero overhead for the 99%
//! case), then auto-detect via `chardetng` and decode with `encoding_rs`.
//!
//! Decoding is lossy: unmappable bytes become U+FFFD rather than errors. This
//! follows the project requirement of "maximum fault-tolerance" — a partially
//! decoded file still yields useful symbols, whereas a skipped file yields none.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Files decoded via the non-UTF-8 fallback path in the current process.
/// `take_fallback_count` resets at the start of each rebuild/update.
pub static FALLBACK_DECODE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// When true, every fallback decode prints a one-line stderr trace.
/// Set by rebuild/update when `--verbose` is passed.
pub static VERBOSE_DECODE: AtomicBool = AtomicBool::new(false);

/// Reads a file from disk and returns its content as a UTF-8 `String`.
///
/// Drop-in replacement for `std::fs::read_to_string` that survives non-UTF-8
/// encodings (Windows-1251, KOI8-R, Shift-JIS, etc.) by auto-detection.
pub fn read_file_to_string<P: AsRef<Path>>(path: P) -> std::io::Result<String> {
    let p = path.as_ref();
    let bytes = std::fs::read(p)?;
    Ok(decode_bytes(&bytes, Some(p)))
}

/// Converts raw bytes to a UTF-8 `String`.
///
/// 1. If the input is already valid UTF-8 → returned as-is, no detection.
/// 2. Otherwise, `chardetng` picks an encoding and `encoding_rs` decodes it,
///    replacing unmappable bytes with U+FFFD.
///
/// Never errors. `origin` is used only for the per-file verbose log.
pub fn decode_bytes(bytes: &[u8], origin: Option<&Path>) -> String {
    if std::str::from_utf8(bytes).is_ok() {
        // Safety: validity just confirmed by `from_utf8`. Skipping the second
        // validation inside `String::from_utf8` keeps the UTF-8 fast path free.
        return unsafe { String::from_utf8_unchecked(bytes.to_vec()) };
    }

    let mut det = chardetng::EncodingDetector::new();
    det.feed(bytes, true);
    // `None` = no TLD hint; `true` = treat input as definitely-not-UTF-8.
    let encoding = det.guess(None, true);
    let (cow, _used, _had_errors) = encoding.decode(bytes);

    FALLBACK_DECODE_COUNT.fetch_add(1, Ordering::Relaxed);
    if VERBOSE_DECODE.load(Ordering::Relaxed) {
        let p = origin
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<bytes>".to_string());
        eprintln!("[encoding] decoded {} as {}", p, encoding.name());
    }
    cow.into_owned()
}

/// Atomically reads and resets `FALLBACK_DECODE_COUNT`.
/// Used by rebuild/update to print a summary at the end of indexing.
pub fn take_fallback_count() -> usize {
    FALLBACK_DECODE_COUNT.swap(0, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_passthrough_does_not_touch_counter() {
        let before = FALLBACK_DECODE_COUNT.load(Ordering::Relaxed);
        let s = "hello, мир";
        let out = decode_bytes(s.as_bytes(), None);
        let after = FALLBACK_DECODE_COUNT.load(Ordering::Relaxed);
        assert_eq!(out, s);
        assert_eq!(before, after, "UTF-8 fast path must not touch counter");
    }

    #[test]
    fn cp1251_decodes_correctly() {
        // "Привет" in Windows-1251: П=0xCF Р=0xF0 И=0xE8 В=0xE2 Е=0xE5 Т=0xF2
        let cp1251 = [0xCFu8, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2];
        let out = decode_bytes(&cp1251, None);
        assert_eq!(out, "Привет");
    }
}
