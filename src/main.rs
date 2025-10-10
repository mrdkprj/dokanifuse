#![allow(clippy::all)]
#![allow(warnings, unused)]
mod afc;
mod bindings;
use afc::*;
pub(crate) use bindings::*;
use once_cell::sync::Lazy;
use std::io::Write;
use std::{
    any::Any,
    collections::HashMap,
    ffi::{CStr, CString},
    io::Read,
    mem::{zeroed, MaybeUninit},
    net::{TcpStream, UdpSocket},
    os::{
        raw::{c_char, c_void},
        windows::io::FromRawSocket,
    },
    path::Path,
    str,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
    time::Duration,
};

const AFC_SERVICE_NAME: &str = "com.apple.afc";
const G_BLOCKSIZE: u64 = 4096;

#[derive(Clone)]
pub struct Device {
    pub(crate) pointer: usize,
}
impl From<idevice_t> for Device {
    fn from(device: idevice_t) -> Device {
        Device {
            pointer: device as _,
        }
    }
}
impl Device {
    pub fn pointer(&self) -> idevice_t {
        self.pointer as _
    }
}
impl Default for Device {
    fn default() -> Self {
        Self { pointer: 0 }
    }
}

static mut DEVICE: Lazy<Device> = Lazy::new(|| Device::default());
static mut CLIENT: Lazy<AfcClient> = Lazy::new(|| AfcClient::default());

fn main() {
    unsafe {
        let argv: Vec<String> = std::env::args().collect();
        let mut opt = Vec::new();
        argv.into_iter().for_each(|a| {
            let c = CString::new(a).unwrap();
            opt.push(c.into_raw());
        });

        if opt.len() <= 1 {
            println!("Invalid arguments");
            return;
        }

        println!("Finding device connected...");
        let mut device_info = MaybeUninit::<idevice_t>::zeroed();
        let device_info_ptr = device_info.as_mut_ptr();
        let res = idevice_new_with_options(
            device_info_ptr,
            std::ptr::null(),
            idevice_options_IDEVICE_LOOKUP_USBMUX | idevice_options_IDEVICE_LOOKUP_NETWORK,
        );

        if res != idevice_error_t_IDEVICE_E_SUCCESS {
            println!("No device found");
            return;
        }

        let device = device_info.assume_init();

        println!("Creating lockdown client...");
        let mut client = MaybeUninit::<lockdownd_client_t>::zeroed();
        let client_ptr = client.as_mut_ptr();
        let program_name = CString::new("myfuse").unwrap();
        let res = lockdownd_client_new_with_handshake(device, client_ptr, program_name.as_ptr());
        if res != lockdownd_error_t_LOCKDOWN_E_SUCCESS || client_ptr.is_null() {
            println!("lockdown failed:{:?}", res);
            idevice_free(device);
            return;
        }

        let client = client.assume_init();

        println!("Starting lockdown service...");
        let service_name = CString::new(AFC_SERVICE_NAME).unwrap();
        let mut descriptor = MaybeUninit::<lockdownd_service_descriptor_t>::zeroed();
        let descriptor_ptr = descriptor.as_mut_ptr();
        let res = lockdownd_start_service(client, service_name.as_ptr(), descriptor_ptr);
        if res != lockdownd_error_t_LOCKDOWN_E_SUCCESS || descriptor_ptr.is_null() {
            println!("lockdownd_start_service failed:{:?}", res);
            lockdownd_client_free(client);
            idevice_free(device);
            return;
        }

        let service_descriptor = descriptor.assume_init();

        if let Some(afc_client) = AfcClient::new(device, service_descriptor) {
            *CLIENT = afc_client;
        } else {
            println!("Cannot create AfcClient");
            lockdownd_client_free(client);
            idevice_free(device);
            return;
        }

        *DEVICE = device.into();
        lockdownd_client_free(client);

        let args = fuse_args {
            argc: opt.len() as _,
            argv: opt.as_mut_ptr(),
            allocated: 0,
        };
        let operations = get_fuse_operations();

        println!("Start fuse");

        let res = fuse_main_real(
            args.argc,
            args.argv,
            &operations as *const _,
            size_of::<fuse_operations>() as _,
            std::ptr::null_mut(),
        );

        /*.\fuse.exe C:/iphone -f */
        /*  .\dokanctl.exe /u C:/iphone */
    }
}

unsafe extern "C" fn ifuse_init(con: *mut fuse_conn_info) -> *mut c_void {
    (*con).async_read = 0;
    std::ptr::null_mut() as _
}

unsafe extern "C" fn ifuse_cleanup(data: *mut c_void) {
    if CLIENT.close() == 0 {
        println!("usbmuxd_disconnect");
    }

    if idevice_free(DEVICE.pointer()) == idevice_error_t_IDEVICE_E_SUCCESS {
        println!("idevice_free")
    }
}

unsafe extern "C" fn ifuse_getattr(path: *const i8, stbuf: *mut stat) -> i32 {
    let info = CLIENT.get_file_info(path);

    std::ptr::write_bytes(stbuf, 0, 1);
    if let Some(list) = extract_list(info) {
        if list.is_empty() {
            return -1;
        }
        let info = to_map(list);

        if let Some(st_size) = info.get("st_size") {
            (*stbuf).st_size = st_size.parse().unwrap();
        }

        // if let Some(st_blocks) = info.get("st_blocks") {
        //     (*stbuf).st_blocks = st_blocks.parse().unwrap();
        // }

        if let Some(st_ifmt) = info.get("st_ifmt") {
            let mode = match st_ifmt.as_str() {
                "S_IFREG" => S_IFREG,
                "S_IFDIR" => S_IFDIR,
                "S_IFLNK" => S_IFLNK,
                "S_IFBLK" => S_IFBLK,
                "S_IFCHR" => S_IFCHR,
                "S_IFIFO" => S_IFIFO,
                "S_IFSOCK" => S_IFSOCK,
                _ => 0,
            };
            (*stbuf).st_mode = mode as _;
        }

        if let Some(st_nlink) = info.get("st_nlink") {
            (*stbuf).st_nlink = st_nlink.parse().unwrap()
        }

        if let Some(st_mtim) = info.get("st_mtime") {
            let nanos: i64 = st_mtim.parse().unwrap();
            (*stbuf).st_mtim = timespec {
                tv_sec: (nanos / 1_000_000_000) as _,
                tv_nsec: (nanos % 1_000_000_000) as _,
            }
        }

        if (*stbuf).st_mode == S_IFDIR as _ {
            (*stbuf).st_mode |= 0o755;
        } else if (*stbuf).st_mode == S_IFLNK as _ {
            (*stbuf).st_mode |= 0o777;
        } else {
            (*stbuf).st_mode |= 0o644;
        }

        // and set some additional info
        (*stbuf).st_uid = 123;
        (*stbuf).st_gid = 456;

        // (*stbuf).st_blksize = G_BLOCKSIZE as _;

        return 0;
    }

    -1
}

unsafe extern "C" fn ifuse_readdir(
    path: *const i8,
    buf: *mut c_void,
    filter: Option<unsafe extern "C" fn(*mut c_void, *const i8, *const stat, u64) -> i32>,
    offset: u64,
    fi: *mut fuse_file_info,
) -> i32 {
    let info = CLIENT.read_directory(path);

    if let Some(dirs) = extract_list(info) {
        if let Some(filter) = filter {
            for dir in dirs {
                let dir = CString::new(dir).unwrap();
                filter(buf, dir.as_ptr(), std::ptr::null(), 1);
            }
        }

        return 0;
    }

    -1
}

unsafe extern "C" fn ifuse_statfs(path: *const i8, stats: *mut statvfs) -> i32 {
    let info = CLIENT.get_device_info();

    if info.is_none() {
        return -1;
    }

    let mut totalspace = 0u64;
    let mut freespace = 0u64;
    let mut blocksize = 0u64;

    if let Some(res) = extract_list(info) {
        let info = to_map(res);

        if let Some(n_str) = info.get("FSTotalBytes") {
            totalspace = n_str.parse().unwrap();
        }
        if let Some(n_str) = info.get("FSFreeBytes") {
            freespace = n_str.parse().unwrap();
        }
        if let Some(n_str) = info.get("FSBlockSize") {
            blocksize = n_str.parse().unwrap();
        }
    }

    (*stats).f_bsize = blocksize as _;
    (*stats).f_frsize = blocksize as _;
    (*stats).f_blocks = if blocksize > 0 {
        (totalspace / blocksize) as _
    } else {
        0
    };
    (*stats).f_bfree = if blocksize > 0 {
        (freespace / blocksize) as _
    } else {
        0
    };
    (*stats).f_bavail = if blocksize > 0 {
        (freespace / blocksize) as _
    } else {
        0
    };
    (*stats).f_namemax = 255;
    (*stats).f_files = 1000000000;
    (*stats).f_ffree = 1000000000;
    0
}

unsafe extern "C" fn ifuse_release(path: *const i8, fi: *mut fuse_file_info) -> i32 {
    CLIENT.file_close((*fi).fh);
    0
}

unsafe extern "C" fn ifuse_opendir(path: *const i8, fi: *mut fuse_file_info) -> i32 {
    0
}

unsafe extern "C" fn ifuse_releasedir(path: *const i8, fi: *mut fuse_file_info) -> i32 {
    0
}

unsafe extern "C" fn ifuse_open(path: *const i8, fi: *mut fuse_file_info) -> i32 {
    let mode = get_afc_file_mode((*fi).flags as _);

    if mode == 0 {
        return -EPERM;
    }

    let info = CLIENT.file_open(path, mode);

    if let Some(res) = extract_num(info) {
        (*fi).fh = res;
        return 0;
    }

    -EPERM
}

fn get_afc_file_mode(flags: u32) -> afc_file_mode_t {
    match flags & O_ACCMODE {
        O_RDONLY => afc_file_mode_t_AFC_FOPEN_RDONLY,
        O_WRONLY => {
            if (flags & O_TRUNC) == O_TRUNC {
                afc_file_mode_t_AFC_FOPEN_WRONLY
            } else if (flags & O_APPEND) == O_APPEND {
                afc_file_mode_t_AFC_FOPEN_APPEND
            } else {
                afc_file_mode_t_AFC_FOPEN_RW
            }
        }
        O_RDWR => {
            if (flags & O_TRUNC) == O_TRUNC {
                afc_file_mode_t_AFC_FOPEN_WR
            } else if (flags & O_APPEND) == O_APPEND {
                afc_file_mode_t_AFC_FOPEN_RDAPPEND
            } else {
                afc_file_mode_t_AFC_FOPEN_RW
            }
        }
        _ => 0,
    }
}

const MAXIMUM_READ_SIZE: u64 = 4 * 1024_u64.pow(2); // 4 MB
unsafe extern "C" fn ifuse_read(
    path: *const i8,
    buf: *mut i8,
    size: u32,
    offset: u64,
    fi: *mut fuse_file_info,
) -> i32 {
    if size == 0 {
        return 0;
    }

    let info = CLIENT.file_seek((*fi).fh, offset, SEEK_SET as _);

    let res = parse_status(info);
    if res != 0 {
        return -res;
    }

    let info = CLIENT.file_read((*fi).fh, size as _);

    let mut bytes = Vec::new();
    if let Some(byte) = extract_byte(info) {
        bytes.extend(byte);
    }

    if !bytes.is_empty() {
        let len = bytes.len().min(size as _);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, len);
        len as _
    } else {
        0
    }
}

unsafe extern "C" fn ifuse_create(path: *const i8, mode: u64, fi: *mut fuse_file_info) -> i32 {
    ifuse_open(path, fi)
}

unsafe extern "C" fn ifuse_write(
    path: *const i8,
    buf: *const i8,
    size: u32,
    offset: u64,
    fi: *mut fuse_file_info,
) -> i32 {
    println!("ifuse_write");
    // if size == 0 {
    //     return 0;
    // }

    // let info = CLIENT.file_seek((*fi).fh, offset, SEEK_SET as _);

    // let res = parse_status(info);
    // if res != 0 {
    //     return -res;
    // }

    // let mut bytes = Vec::new();
    // let info = CLIENT.file_write((*fi).fh, buf, size);
    // if let Some(byte) = extract_byte(info) {
    //     bytes.extend(byte);
    // }

    // bytes.len() as _
    0
}

unsafe extern "C" fn ifuse_truncate(path: *const i8, size: u64) -> i32 {
    // CLIENT.truncate(path, size);
    println!("ifuse_truncate");
    -1
}

unsafe extern "C" fn ifuse_ftruncate(path: *const i8, size: u64, fi: *mut fuse_file_info) -> i32 {
    println!("ifuse_ftruncate");
    // CLIENT.truncate(path, size);
    -1
}

unsafe extern "C" fn ifuse_unlink(path: *const i8) -> i32 {
    println!("ifuse_unlink");
    // let info = CLIENT.remove_path(path);
    // println!("{:?}", info);
    // let res = parse_status(info);
    // if res != 0 {
    //     return -res;
    // }

    // 0
    -1
}

unsafe extern "C" fn ifuse_fsync(path: *const i8, datasync: i32, fi: *mut fuse_file_info) -> i32 {
    println!("ifuse_fsync");
    0
}

unsafe extern "C" fn ifuse_chmod(path: *const i8, mode: u64) -> i32 {
    0
}

unsafe extern "C" fn ifuse_chown(file: *const i8, user: u32, group: u32) -> i32 {
    0
}

unsafe extern "C" fn ifuse_readlink(a: *const i8, b: *mut i8, c: u32) -> i32 {
    0
}

pub fn get_fuse_operations() -> fuse_operations {
    fuse_operations {
        getattr: Some(ifuse_getattr),
        statfs: Some(ifuse_statfs),
        readdir: Some(ifuse_readdir),
        mkdir: None,
        rmdir: None,
        create: Some(ifuse_create),
        open: Some(ifuse_open),
        read: Some(ifuse_read),
        write: Some(ifuse_write),
        truncate: Some(ifuse_truncate),
        readlink: Some(ifuse_readlink),
        symlink: None,
        link: None,
        unlink: Some(ifuse_unlink),
        rename: None,
        utimens: None,
        fsync: Some(ifuse_fsync),
        chmod: Some(ifuse_chmod),
        chown: Some(ifuse_chown),
        release: Some(ifuse_release),
        init: Some(ifuse_init),
        destroy: Some(ifuse_cleanup),
        //
        getdir: None,
        mknod: None,
        utime: None,
        flush: None,
        setxattr: None,
        getxattr: None,
        listxattr: None,
        removexattr: None,
        opendir: Some(ifuse_opendir),
        releasedir: Some(ifuse_releasedir),
        fsyncdir: None,
        access: None,
        ftruncate: None,
        fgetattr: None,
        lock: None,
        bmap: None,
    }
}
