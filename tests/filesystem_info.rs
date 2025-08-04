use robosync::filesystem_info::*;
use std::fs::File;
use tempfile::tempdir;

#[test]
fn test_are_on_same_filesystem() {
    let dir = tempdir().unwrap();
    let path1 = dir.path().join("file1");
    let path2 = dir.path().join("file2");
    File::create(&path1).unwrap();
    File::create(&path2).unwrap();

    assert!(are_on_same_filesystem(&path1, &path2).unwrap());
}

#[test]
fn test_get_filesystem_info() {
    let dir = tempdir().unwrap();
    let info = get_filesystem_info(dir.path()).unwrap();
    
    // Basic checks
    assert!(info.device_id > 0);
    assert!(info.optimal_block_size > 0);
    
    // The filesystem type should be one of the known types
    match info.filesystem_type {
        FilesystemType::Btrfs | FilesystemType::Xfs | FilesystemType::Zfs |
        FilesystemType::Apfs | FilesystemType::ReFS | FilesystemType::Ext4 |
        FilesystemType::Ntfs | FilesystemType::Nfs | FilesystemType::Smb |
        FilesystemType::Other(_) => {
            // Valid filesystem type
        }
    }
}

#[test]
fn test_cache_functionality() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    
    // First call should hit the filesystem
    let info1 = get_filesystem_info(path).unwrap();
    
    // Second call should use cache (within TTL)
    let info2 = get_filesystem_info(path).unwrap();
    
    // Both should return the same data
    assert_eq!(info1.device_id, info2.device_id);
    assert_eq!(info1.filesystem_type, info2.filesystem_type);
    assert_eq!(info1.supports_reflinks, info2.supports_reflinks);
    assert_eq!(info1.optimal_block_size, info2.optimal_block_size);
}
