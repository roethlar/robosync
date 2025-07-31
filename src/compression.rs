//! Compression support for delta transfer optimization

use anyhow::{Context, Result};
use std::io::{Read, Write};

// Constants for compression configuration
const MIN_COMPRESS_SIZE: usize = 64; // Minimum size to attempt compression
const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks for streaming
const MAX_DECOMPRESSED_SIZE: usize = 256 * 1024 * 1024; // 256MB max for safety
const DEFAULT_ZSTD_LEVEL: i32 = 3; // Balanced speed/compression
const FAST_LZ4_LEVEL: i32 = 1; // Fast compression for LZ4
const BEST_ZSTD_LEVEL: i32 = 19; // Maximum compression for zstd

/// Compression algorithms supported
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CompressionType {
    None,
    #[default]
    Zstd,
    Lz4,
}

/// Compression level settings
#[derive(Debug, Clone, Copy)]
pub struct CompressionConfig {
    pub algorithm: CompressionType,
    pub level: i32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            algorithm: CompressionType::Zstd,
            level: DEFAULT_ZSTD_LEVEL, // Balanced speed/compression for zstd
        }
    }
}

impl CompressionConfig {
    /// Create config optimized for speed
    pub fn fast() -> Self {
        Self {
            algorithm: CompressionType::Lz4,
            level: FAST_LZ4_LEVEL,
        }
    }

    /// Create config optimized for compression ratio
    pub fn best() -> Self {
        Self {
            algorithm: CompressionType::Zstd,
            level: BEST_ZSTD_LEVEL,
        }
    }

    /// Create config balanced for speed and ratio
    pub fn balanced() -> Self {
        Self {
            algorithm: CompressionType::Zstd,
            level: DEFAULT_ZSTD_LEVEL,
        }
    }
}

/// Compress data using the specified algorithm
pub fn compress_data(data: &[u8], config: CompressionConfig) -> Result<Vec<u8>> {
    match config.algorithm {
        CompressionType::None => Ok(data.to_vec()),
        CompressionType::Zstd => {
            zstd::bulk::compress(data, config.level).context("Failed to compress data with zstd")
        }
        CompressionType::Lz4 => Ok(lz4_flex::compress_prepend_size(data)),
    }
}

/// Decompress data using the specified algorithm with dynamic sizing
pub fn decompress_data(data: &[u8], algorithm: CompressionType) -> Result<Vec<u8>> {
    match algorithm {
        CompressionType::None => Ok(data.to_vec()),
        CompressionType::Zstd => {
            // Try to get the decompressed size from the frame header
            match zstd::bulk::Decompressor::upper_bound(data) {
                Some(size) if size > 0 && size <= MAX_DECOMPRESSED_SIZE => {
                    // Use the known size if reasonable
                    zstd::bulk::decompress(data, size)
                        .context("Failed to decompress data with zstd")
                }
                _ => {
                    // Fall back to streaming decompression for unknown or large sizes
                    let mut decoder = zstd::stream::Decoder::new(data)
                        .context("Failed to create zstd decoder")?;
                    let mut decompressed = Vec::new();
                    decoder
                        .read_to_end(&mut decompressed)
                        .context("Failed to decompress data with zstd")?;
                    Ok(decompressed)
                }
            }
        }
        CompressionType::Lz4 => {
            lz4_flex::decompress_size_prepended(data).context("Failed to decompress data with lz4")
        }
    }
}

/// Estimate if compression would be beneficial for given data size
pub fn should_compress(data_size: usize) -> bool {
    data_size >= MIN_COMPRESS_SIZE
}

/// Get optimal buffer size for decompression based on compressed data
pub fn get_decompression_buffer_size(
    compressed_data: &[u8],
    algorithm: CompressionType,
) -> Option<usize> {
    match algorithm {
        CompressionType::None => Some(compressed_data.len()),
        CompressionType::Zstd => zstd::bulk::Decompressor::upper_bound(compressed_data),
        CompressionType::Lz4 => {
            // LZ4 with prepended size stores the size in the first 4 bytes
            if compressed_data.len() >= 4 {
                let size = u32::from_le_bytes([
                    compressed_data[0],
                    compressed_data[1],
                    compressed_data[2],
                    compressed_data[3],
                ]) as usize;
                if size <= MAX_DECOMPRESSED_SIZE {
                    Some(size)
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

/// Compress delta transfer data efficiently
/// This is optimized for rsync-style delta transfers where we have
/// literal data chunks that can benefit from compression
pub fn compress_literal_data(literal_data: &[u8], config: CompressionConfig) -> Result<Vec<u8>> {
    // Only compress if the data is large enough to benefit
    if !should_compress(literal_data.len()) {
        return Ok(literal_data.to_vec());
    }

    let compressed = compress_data(literal_data, config)?;

    // Only use compressed version if it's actually smaller
    // and achieves at least 10% compression ratio
    let compression_ratio = compression_ratio(literal_data.len() as u64, compressed.len() as u64);
    if compressed.len() < literal_data.len() && compression_ratio >= 10.0 {
        Ok(compressed)
    } else {
        Ok(literal_data.to_vec())
    }
}

/// Streaming compressor for large files
pub struct StreamingCompressor {
    config: CompressionConfig,
}

impl StreamingCompressor {
    pub fn new(config: CompressionConfig) -> Self {
        Self { config }
    }

    /// Compress a stream of data
    pub fn compress_stream<R: Read, W: Write>(&self, mut reader: R, mut writer: W) -> Result<u64> {
        match self.config.algorithm {
            CompressionType::None => std::io::copy(&mut reader, &mut writer)
                .context("Failed to copy data without compression"),
            CompressionType::Zstd => {
                let mut encoder = zstd::Encoder::new(&mut writer, self.config.level)
                    .context("Failed to create zstd encoder")?;
                let bytes_written = std::io::copy(&mut reader, &mut encoder)
                    .context("Failed to compress stream with zstd")?;
                encoder
                    .finish()
                    .context("Failed to finalize zstd compression")?;
                Ok(bytes_written)
            }
            CompressionType::Lz4 => {
                // LZ4 doesn't have a streaming encoder in lz4_flex, so we read chunks
                let mut buffer = vec![0u8; CHUNK_SIZE];
                let mut total_read = 0u64;

                loop {
                    let bytes_read = reader
                        .read(&mut buffer)
                        .context("Failed to read data for lz4 compression")?;

                    if bytes_read == 0 {
                        break;
                    }

                    let compressed_chunk = lz4_flex::compress(&buffer[..bytes_read]);

                    // Write chunk size first, then compressed data
                    let chunk_size = compressed_chunk.len() as u32;
                    writer
                        .write_all(&chunk_size.to_le_bytes())
                        .context("Failed to write chunk size")?;
                    writer
                        .write_all(&compressed_chunk)
                        .context("Failed to write compressed chunk")?;

                    total_read += bytes_read as u64;
                }

                // Write end marker (chunk size 0)
                writer
                    .write_all(&0u32.to_le_bytes())
                    .context("Failed to write end marker")?;

                Ok(total_read)
            }
        }
    }
}

/// Streaming decompressor for large files
pub struct StreamingDecompressor {
    algorithm: CompressionType,
}

impl StreamingDecompressor {
    pub fn new(algorithm: CompressionType) -> Self {
        Self { algorithm }
    }

    /// Decompress a stream of data
    pub fn decompress_stream<R: Read, W: Write>(
        &self,
        mut reader: R,
        mut writer: W,
    ) -> Result<u64> {
        match self.algorithm {
            CompressionType::None => std::io::copy(&mut reader, &mut writer)
                .context("Failed to copy data without decompression"),
            CompressionType::Zstd => {
                let mut decoder =
                    zstd::Decoder::new(&mut reader).context("Failed to create zstd decoder")?;
                std::io::copy(&mut decoder, &mut writer)
                    .context("Failed to decompress stream with zstd")
            }
            CompressionType::Lz4 => {
                let mut total_written = 0u64;

                loop {
                    // Read chunk size
                    let mut size_buf = [0u8; 4];
                    if reader.read_exact(&mut size_buf).is_err() {
                        break; // End of stream
                    }

                    let chunk_size = u32::from_le_bytes(size_buf);
                    if chunk_size == 0 {
                        break; // End marker
                    }

                    // Read compressed chunk
                    let mut compressed_chunk = vec![0u8; chunk_size as usize];
                    reader
                        .read_exact(&mut compressed_chunk)
                        .context("Failed to read compressed chunk")?;

                    // Decompress chunk with size limit for safety
                    let decompressed =
                        lz4_flex::decompress(&compressed_chunk, MAX_DECOMPRESSED_SIZE)
                            .context("Failed to decompress lz4 chunk")?;

                    // Write decompressed data
                    writer
                        .write_all(&decompressed)
                        .context("Failed to write decompressed data")?;

                    total_written += decompressed.len() as u64;
                }

                Ok(total_written)
            }
        }
    }
}

/// Calculate compression ratio as a percentage
pub fn compression_ratio(original_size: u64, compressed_size: u64) -> f64 {
    if original_size == 0 {
        return 0.0;
    }
    ((original_size - compressed_size) as f64 / original_size as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zstd_compression() -> Result<()> {
        let data = b"Hello, world! This is a test string for compression.".repeat(100);
        let config = CompressionConfig::default();

        let compressed = compress_data(&data, config)?;
        let decompressed = decompress_data(&compressed, CompressionType::Zstd)?;

        assert_eq!(data, decompressed);
        assert!(compressed.len() < data.len());
        Ok(())
    }

    #[test]
    fn test_lz4_compression() -> Result<()> {
        let data = b"Hello, world! This is a test string for compression.".repeat(100);
        let config = CompressionConfig::fast();

        let compressed = compress_data(&data, config)?;
        let decompressed = decompress_data(&compressed, CompressionType::Lz4)?;

        assert_eq!(data, decompressed);
        Ok(())
    }

    #[test]
    fn test_compression_ratio() {
        assert_eq!(compression_ratio(100, 50), 50.0);
        assert_eq!(compression_ratio(100, 0), 100.0);
        assert_eq!(compression_ratio(0, 50), 0.0);
    }

    #[test]
    fn test_small_data_not_compressed() -> Result<()> {
        let small_data = b"small";
        let config = CompressionConfig::default();

        let result = compress_literal_data(small_data, config)?;
        assert_eq!(result, small_data);
        Ok(())
    }
}
