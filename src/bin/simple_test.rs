use std::fs;
use std::path::Path;
use std::time::Instant;

fn main() {
    // Test copying a single large file
    let test_file = "C:\\Program Files (x86)\\Steam\\steamapps\\common\\Counter-Strike Global Offensive\\csgo\\pak01_dir.vpk";
    let dest_file = "H:\\stuff\\backup\\test_single_file.vpk";
    
    println!("Testing single file copy speed...");
    println!("Source: {}", test_file);
    println!("Destination: {}", dest_file);
    
    // Get file size
    let metadata = fs::metadata(test_file).expect("Failed to get metadata");
    let file_size = metadata.len();
    println!("File size: {} MB", file_size / 1_000_000);
    
    // Time the copy
    let start = Instant::now();
    let bytes = fs::copy(test_file, dest_file).expect("Failed to copy");
    let elapsed = start.elapsed();
    
    let mb_per_sec = (bytes as f64 / 1_000_000.0) / elapsed.as_secs_f64();
    println!("\nCopied {} bytes in {:?}", bytes, elapsed);
    println!("Speed: {:.2} MB/s", mb_per_sec);
    
    // Clean up
    fs::remove_file(dest_file).ok();
}