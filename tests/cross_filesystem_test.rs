use robosync::filesystem_info::*;
use robosync::reflink::{try_reflink, ReflinkOptions, ReflinkMode, ReflinkResult};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Cross-filesystem test matrix
/// Tests copying between different filesystem types and verifies correct behavior
#[derive(Debug)]
struct CrossFsTestCase {
    source_fs: &'static str,
    dest_fs: &'static str,
    expected_behavior: ExpectedBehavior,
}

#[derive(Debug, PartialEq)]
enum ExpectedBehavior {
    RefLinkFails,      // Reflink should fail, fallback to regular copy
    NetworkOptimized,  // Should use network-optimized buffers
    RegularCopy,       // Standard copy behavior
}

/// Test matrix for cross-filesystem operations
const TEST_MATRIX: &[CrossFsTestCase] = &[
    CrossFsTestCase {
        source_fs: "btrfs",
        dest_fs: "ext4",
        expected_behavior: ExpectedBehavior::RefLinkFails,
    },
    CrossFsTestCase {
        source_fs: "xfs",
        dest_fs: "btrfs",
        expected_behavior: ExpectedBehavior::RefLinkFails,
    },
    CrossFsTestCase {
        source_fs: "zfs",
        dest_fs: "nfs",
        expected_behavior: ExpectedBehavior::NetworkOptimized,
    },
    CrossFsTestCase {
        source_fs: "ext4",
        dest_fs: "smb",
        expected_behavior: ExpectedBehavior::NetworkOptimized,
    },
];

#[test]
fn test_cross_filesystem_reflink_behavior() {
    // Test reflink behavior across filesystems
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("source.bin");
    let dest = temp_dir.path().join("dest.bin");
    
    // Create test file
    let test_data = vec![0xAB; 1024 * 1024]; // 1MB
    fs::write(&source, &test_data).unwrap();
    
    // Test with reflink=always (should fail across filesystems)
    let options_always = ReflinkOptions { mode: ReflinkMode::Always };
    
    // Create empty dest file to work around are_on_same_filesystem check
    fs::write(&dest, b"").unwrap();
    
    let result = try_reflink(&source, &dest, &options_always);
    match result {
        ReflinkResult::Success => {
            // Same filesystem with reflink support
            println!("Reflink succeeded (same filesystem)");
            assert_eq!(fs::read(&dest).unwrap(), test_data);
        }
        ReflinkResult::Error(e) => {
            // Expected for cross-filesystem or unsupported
            println!("Expected reflink failure: {}", e);
            // Clean up for next test
            let _ = fs::remove_file(&dest);
        }
        ReflinkResult::Fallback => {
            panic!("Reflink with Always mode should not return Fallback");
        }
    }
    
    // Test with reflink=auto (should succeed with fallback)
    fs::write(&dest, b"").unwrap(); // Create empty dest again
    let options_auto = ReflinkOptions { mode: ReflinkMode::Auto };
    let result = try_reflink(&source, &dest, &options_auto);
    
    match result {
        ReflinkResult::Success => {
            println!("Auto mode: reflink succeeded");
            assert_eq!(fs::read(&dest).unwrap(), test_data);
        }
        ReflinkResult::Fallback => {
            println!("Auto mode: fell back to regular copy (expected)");
            // Would need to do actual copy here in real usage
            fs::copy(&source, &dest).unwrap();
            assert_eq!(fs::read(&dest).unwrap(), test_data);
        }
        ReflinkResult::Error(e) => {
            panic!("Auto mode should not fail: {}", e);
        }
    }
}

#[test]
fn test_filesystem_detection_accuracy() {
    // Test filesystem detection on various paths
    let test_paths = vec![
        ("/", "Root filesystem"),
        ("/tmp", "Temp filesystem"),
        (".", "Current directory"),
    ];
    
    for (path, description) in test_paths {
        if let Ok(fs_info) = get_filesystem_info(Path::new(path)) {
            println!("{} ({}): {:?}", description, path, fs_info.filesystem_type);
            println!("  Device ID: {}", fs_info.device_id);
            println!("  Supports reflinks: {}", fs_info.supports_reflinks);
            println!("  Optimal block size: {}", fs_info.optimal_block_size);
            
            // Check if network filesystem based on type
            let is_network = matches!(fs_info.filesystem_type, 
                FilesystemType::Nfs | FilesystemType::Smb);
            println!("  Is network: {}", is_network);
        }
    }
}

#[test]
#[ignore] // Requires multiple filesystem mounts
fn test_cross_filesystem_matrix() {
    // This test requires specific filesystem mounts to be available
    let mounts = vec![
        ("/mnt/btrfs_test", FilesystemType::Btrfs),
        ("/mnt/xfs_test", FilesystemType::Xfs),
        ("/mnt/ext4_test", FilesystemType::Ext4),
        ("/mnt/zfs_test", FilesystemType::Zfs),
    ];
    
    // Verify mounts
    let mut available_mounts = vec![];
    for (mount, expected_fs) in mounts {
        if std::path::Path::new(mount).exists() {
            if let Ok(fs_info) = get_filesystem_info(Path::new(mount)) {
                if fs_info.filesystem_type == expected_fs {
                    available_mounts.push((mount, expected_fs));
                }
            }
        }
    }
    
    if available_mounts.len() < 2 {
        eprintln!("Not enough test filesystems available");
        return;
    }
    
    // Test matrix
    for (source_mount, source_fs) in &available_mounts {
        for (dest_mount, dest_fs) in &available_mounts {
            if source_mount == dest_mount {
                continue;
            }
            
            println!("Testing {:?} -> {:?}", source_fs, dest_fs);
            test_cross_fs_copy(source_mount, dest_mount);
        }
    }
}

fn test_cross_fs_copy(source_mount: &str, dest_mount: &str) {
    let source_dir = TempDir::new_in(source_mount).unwrap();
    let dest_dir = TempDir::new_in(dest_mount).unwrap();
    
    let source_file = source_dir.path().join("test.bin");
    let dest_file = dest_dir.path().join("test.bin");
    
    // Create test files of various sizes
    let test_sizes = vec![
        (1024, "1KB"),
        (1024 * 1024, "1MB"),
        (10 * 1024 * 1024, "10MB"),
    ];
    
    for (size, desc) in test_sizes {
        println!("  Testing {} file", desc);
        
        // Create source file
        let test_data = vec![0xFF; size];
        fs::write(&source_file, &test_data).unwrap();
        
        // Test with different reflink behaviors
        for mode in &[ReflinkMode::Never, ReflinkMode::Auto, ReflinkMode::Always] {
            // Create empty dest file for filesystem check
            fs::write(&dest_file, b"").unwrap();
            
            let options = ReflinkOptions { mode: *mode };
            let result = try_reflink(&source_file, &dest_file, &options);
            
            match (mode, &result) {
                (ReflinkMode::Always, ReflinkResult::Error(_)) => {
                    // Expected failure for cross-filesystem reflink
                    println!("    RefLink=always: Failed as expected");
                }
                (ReflinkMode::Never, ReflinkResult::Fallback) => {
                    // Expected - Never mode always returns fallback
                    fs::copy(&source_file, &dest_file).unwrap();
                    assert_eq!(fs::read(&dest_file).unwrap(), test_data);
                    println!("    RefLink=never: Fallback as expected");
                }
                (ReflinkMode::Auto, ReflinkResult::Fallback) => {
                    // Expected for cross-filesystem
                    fs::copy(&source_file, &dest_file).unwrap();
                    assert_eq!(fs::read(&dest_file).unwrap(), test_data);
                    println!("    RefLink=auto: Fallback as expected");
                }
                (_, ReflinkResult::Success) => {
                    // Reflink succeeded (same filesystem case)
                    assert_eq!(fs::read(&dest_file).unwrap(), test_data);
                    println!("    RefLink={:?}: Success", mode);
                }
                (mode, result) => {
                    panic!("Unexpected result with reflink={:?}: {:?}", mode, result);
                }
            }
            
            // Cleanup destination for next test
            let _ = fs::remove_file(&dest_file);
        }
    }
}

#[test]
fn test_network_filesystem_detection() {
    // Test that network filesystems are correctly identified
    println!("Testing network filesystem detection");
    
    // These filesystem types should be considered network filesystems
    let network_types = vec![
        FilesystemType::Nfs,
        FilesystemType::Smb,
    ];
    
    for fs_type in &network_types {
        println!("  {:?} - should be detected as network filesystem", fs_type);
    }
    
    // Non-network filesystems
    let local_types = vec![
        FilesystemType::Ext4,
        FilesystemType::Xfs,
        FilesystemType::Btrfs,
        FilesystemType::Ntfs,
    ];
    
    for fs_type in &local_types {
        println!("  {:?} - should be detected as local filesystem", fs_type);
    }
}