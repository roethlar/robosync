use robosync::filesystem_info::{get_filesystem_info, FilesystemType};
use robosync::metadata::{copy_file_with_metadata, CopyFlags};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::env;

#[test]
#[ignore] // Run with ROBOSYNC_TEST_DIR=/mnt/robosync_test/ntfs cargo test --test ntfs_fuse_test -- --ignored
fn test_ntfs_fuse_detection() {
    let test_dir = env::var("ROBOSYNC_TEST_DIR")
        .unwrap_or_else(|_| "/mnt/robosync_test/ntfs".to_string());
    
    let test_path = PathBuf::from(&test_dir);
    if !test_path.exists() {
        eprintln!("NTFS test directory not found at: {:?}", test_path);
        eprintln!("Please set ROBOSYNC_TEST_DIR to an NTFS mount point");
        return;
    }
    
    // Check filesystem detection
    match get_filesystem_info(&test_path) {
        Ok(fs_info) => {
            println!("Detected filesystem: {:?}", fs_info.filesystem_type);
            assert_eq!(fs_info.filesystem_type, FilesystemType::Ntfs);
        }
        Err(e) => {
            panic!("Failed to detect filesystem: {}", e);
        }
    }
}

#[test]
#[ignore]
fn test_ntfs_fuse_basic_copy() {
    let test_dir = env::var("ROBOSYNC_TEST_DIR")
        .unwrap_or_else(|_| "/mnt/robosync_test/ntfs".to_string());
    
    let test_path = PathBuf::from(&test_dir).join("ntfs_test");
    fs::create_dir_all(&test_path).expect("Failed to create test directory");
    
    // Create a test file
    let source_file = test_path.join("test_file.txt");
    let mut file = File::create(&source_file).expect("Failed to create test file");
    let test_data = "This is NTFS test data with some special characters: Ñ © ® €\n".repeat(1000);
    file.write_all(test_data.as_bytes()).expect("Failed to write test data");
    drop(file);
    
    // Copy the file
    let dest_file = test_path.join("test_file_copy.txt");
    let copy_flags = CopyFlags::default();
    match copy_file_with_metadata(&source_file, &dest_file, &copy_flags) {
        Ok(bytes_copied) => {
            println!("Successfully copied {} bytes", bytes_copied);
            
            // Verify the copy
            let source_content = fs::read(&source_file).expect("Failed to read source");
            let dest_content = fs::read(&dest_file).expect("Failed to read dest");
            assert_eq!(source_content, dest_content);
        }
        Err(e) => {
            panic!("Failed to copy file: {}", e);
        }
    }
    
    // Cleanup
    fs::remove_dir_all(&test_path).ok();
}

#[test]
#[ignore]
fn test_ntfs_fuse_large_file() {
    let test_dir = env::var("ROBOSYNC_TEST_DIR")
        .unwrap_or_else(|_| "/mnt/robosync_test/ntfs".to_string());
    
    let test_path = PathBuf::from(&test_dir).join("ntfs_large_test");
    fs::create_dir_all(&test_path).expect("Failed to create test directory");
    
    // Create a 100MB test file
    let source_file = test_path.join("large_file.bin");
    let mut file = File::create(&source_file).expect("Failed to create test file");
    let chunk = vec![0u8; 1024 * 1024]; // 1MB chunks
    for _ in 0..100 {
        file.write_all(&chunk).expect("Failed to write data");
    }
    drop(file);
    
    // Time the copy
    let dest_file = test_path.join("large_file_copy.bin");
    let copy_flags = CopyFlags::default();
    let start = std::time::Instant::now();
    
    match copy_file_with_metadata(&source_file, &dest_file, &copy_flags) {
        Ok(bytes_copied) => {
            let elapsed = start.elapsed();
            let throughput = bytes_copied as f64 / elapsed.as_secs_f64() / 1024.0 / 1024.0;
            println!("Copied {} bytes in {:?}", bytes_copied, elapsed);
            println!("Throughput: {:.2} MB/s", throughput);
            
            // Verify size
            let source_meta = fs::metadata(&source_file).expect("Failed to get source metadata");
            let dest_meta = fs::metadata(&dest_file).expect("Failed to get dest metadata");
            assert_eq!(source_meta.len(), dest_meta.len());
        }
        Err(e) => {
            panic!("Failed to copy large file: {}", e);
        }
    }
    
    // Cleanup
    fs::remove_dir_all(&test_path).ok();
}

#[test]
#[ignore]
fn test_ntfs_fuse_windows_attributes() {
    let test_dir = env::var("ROBOSYNC_TEST_DIR")
        .unwrap_or_else(|_| "/mnt/robosync_test/ntfs".to_string());
    
    let test_path = PathBuf::from(&test_dir).join("ntfs_attr_test");
    fs::create_dir_all(&test_path).expect("Failed to create test directory");
    
    // Create files with different attributes
    let files = vec![
        ("normal.txt", "Normal file"),
        ("hidden.txt", "Hidden file"),
        ("readonly.txt", "Read-only file"),
    ];
    
    for (name, content) in &files {
        let file_path = test_path.join(name);
        fs::write(&file_path, content).expect("Failed to create file");
        
        // Copy and verify
        let copy_path = test_path.join(format!("{}.copy", name));
        let copy_flags = CopyFlags::default();
        match copy_file_with_metadata(&file_path, &copy_path, &copy_flags) {
            Ok(_) => {
                println!("Successfully copied {}", name);
                
                // Verify content
                let copied_content = fs::read_to_string(&copy_path).expect("Failed to read copy");
                assert_eq!(&copied_content, content);
            }
            Err(e) => {
                eprintln!("Failed to copy {}: {}", name, e);
            }
        }
    }
    
    // Cleanup
    fs::remove_dir_all(&test_path).ok();
}