use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use crate::safe_ops::SafeMutex;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
use nix::sys::statfs;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
use std::os::unix::fs::MetadataExt;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use winapi::um::fileapi::{GetVolumeInformationW, OPEN_EXISTING};
#[cfg(windows)]
use winapi::um::winnt::FILE_ATTRIBUTE_NORMAL;

lazy_static::lazy_static! {
    static ref FILESYSTEM_INFO_CACHE: Mutex<HashMap<PathBuf, (FilesystemInfo, Instant)>> = Mutex::new(HashMap::new());
}

const CACHE_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesystemType {
    Btrfs,
    Xfs,
    Zfs,
    Apfs,
    ReFS,
    Ext4,
    Ntfs,
    Nfs,
    Smb,
    Other(String),
}

#[derive(Debug, Clone)]
pub struct FilesystemInfo {
    pub device_id: u64,
    pub filesystem_type: FilesystemType,
    pub supports_reflinks: bool,
    pub optimal_block_size: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
fn get_filesystem_info_unix(path: &Path) -> Result<FilesystemInfo, io::Error> {
    let metadata = std::fs::metadata(path)?;
    let stat = statfs::statfs(path)?;
    
    // Get filesystem type
    #[cfg(target_os = "linux")]
    let fs_type = {
        use nix::sys::statfs::FsType;
        match stat.filesystem_type() {
            FsType(0x9123683e) => FilesystemType::Btrfs,  // BTRFS_SUPER_MAGIC
            FsType(0xef53) => FilesystemType::Ext4,       // EXT4_SUPER_MAGIC  
            FsType(0x58465342) => FilesystemType::Xfs,     // XFS_SUPER_MAGIC
            FsType(0x2fc12fc1) => FilesystemType::Zfs,     // ZFS magic number
            FsType(0x6969) => FilesystemType::Nfs,        // NFS_SUPER_MAGIC
            FsType(0x517b) => FilesystemType::Smb,        // SMB_SUPER_MAGIC
            FsType(0x65735546) => FilesystemType::Ntfs,   // NTFS on Linux (FUSE)
            _ => FilesystemType::Other(format!("0x{:x}", stat.filesystem_type().0)),
        }
    };
    
    #[cfg(target_os = "macos")]
    let fs_type = {
        // macOS uses string-based filesystem type names
        let fs_type_name = stat.filesystem_type_name()
            .trim_end_matches('\0');
        match fs_type_name {
            "apfs" => FilesystemType::Apfs,
            "hfs" => FilesystemType::Other("HFS+".to_string()),
            "nfs" => FilesystemType::Nfs,
            "smbfs" => FilesystemType::Smb,
            _ => FilesystemType::Other(fs_type_name.to_string()),
        }
    };
    
    #[cfg(target_os = "freebsd")]
    let fs_type = {
        // FreeBSD also uses string-based names
        let fs_type_name = stat.filesystem_type_name()
            .trim_end_matches('\0');
        match fs_type_name {
            "zfs" => FilesystemType::Zfs,
            "ufs" => FilesystemType::Other("UFS".to_string()),
            "nfs" => FilesystemType::Nfs,
            _ => FilesystemType::Other(fs_type_name.to_string()),
        }
    };
    
    // On Linux, only BTRFS and XFS support FICLONE ioctl
    // ZFS on Linux doesn't support FICLONE (it has its own copy-on-write mechanism)
    #[cfg(target_os = "linux")]
    let supports_reflinks = matches!(
        fs_type, 
        FilesystemType::Btrfs | FilesystemType::Xfs
    );
    
    // On macOS, APFS supports clonefile()
    #[cfg(target_os = "macos")]
    let supports_reflinks = matches!(
        fs_type, 
        FilesystemType::Apfs
    );
    
    // On FreeBSD, ZFS supports copy-on-write
    #[cfg(target_os = "freebsd")]
    let supports_reflinks = matches!(
        fs_type, 
        FilesystemType::Zfs
    );
    
    // On other platforms, no reflink support
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
    let supports_reflinks = false;

    Ok(FilesystemInfo {
        device_id: metadata.dev(),
        filesystem_type: fs_type,
        supports_reflinks,
        optimal_block_size: stat.optimal_transfer_size() as usize,
    })
}

#[cfg(windows)]
fn get_filesystem_info_windows(path: &Path) -> Result<FilesystemInfo, io::Error> {
    use winapi::um::fileapi::GetVolumePathNameW;
    
    let mut volume_name = [0u16; 261];
    let mut fs_name = [0u16; 261];
    let mut volume_serial_number = 0;
    let mut max_component_length = 0;
    let mut fs_flags = 0;

    // First, get the volume root path
    let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut volume_root = [0u16; 261];
    
    let result = unsafe {
        GetVolumePathNameW(
            path_wide.as_ptr(),
            volume_root.as_mut_ptr(),
            volume_root.len() as u32,
        )
    };
    
    if result == 0 {
        return Err(io::Error::last_os_error());
    }

    // Now get volume information using the root path
    let result = unsafe {
        GetVolumeInformationW(
            volume_root.as_ptr(),
            volume_name.as_mut_ptr(),
            volume_name.len() as u32,
            &mut volume_serial_number,
            &mut max_component_length,
            &mut fs_flags,
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };

    if result == 0 {
        return Err(io::Error::last_os_error());
    }

    let fs_name_str = String::from_utf16_lossy(&fs_name).trim_end_matches('\0').to_string();
    let fs_type = match fs_name_str.as_str() {
        "NTFS" => FilesystemType::Ntfs,
        "ReFS" => FilesystemType::ReFS,
        _ => FilesystemType::Other(fs_name_str),
    };

    let supports_reflinks = matches!(fs_type, FilesystemType::ReFS);

    Ok(FilesystemInfo {
        device_id: volume_serial_number as u64,
        filesystem_type: fs_type,
        supports_reflinks,
        optimal_block_size: 4096, // A reasonable default for Windows
    })
}

pub fn get_filesystem_info(path: &Path) -> Result<FilesystemInfo, io::Error> {
    let mut cache = FILESYSTEM_INFO_CACHE.safe_lock()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Failed to lock filesystem cache: {}", e)))?;
    if let Some((info, timestamp)) = cache.get(path) {
        if timestamp.elapsed() < CACHE_TTL {
            return Ok(info.clone());
        }
    }

    let info = {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
        {
            get_filesystem_info_unix(path)
        }

        #[cfg(windows)]
        {
            get_filesystem_info_windows(path)
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd", windows)))]
        {
            unimplemented!("Filesystem detection not implemented for this platform yet.")
        }
    }?; 

    cache.insert(path.to_path_buf(), (info.clone(), Instant::now()));
    Ok(info)
}

pub fn are_on_same_filesystem(path1: &Path, path2: &Path) -> Result<bool, io::Error> {
    let info1 = get_filesystem_info(path1)?;
    let info2 = get_filesystem_info(path2)?;
    Ok(info1.device_id == info2.device_id)
}
