#![allow(unused_variables)]
mod afc;
mod bindings;
mod housearrest;
mod instproxy;
use crate::instproxy::print_app;
use afc::*;
pub(crate) use bindings::*;
use clap::{arg, Args, Parser};
use std::sync::OnceLock;
use std::{
    ffi::{CStr, CString},
    mem::MaybeUninit,
    os::raw::c_void,
    str,
};

const AFC_SERVICE_NAME: &str = "com.apple.afc";
const HOUSE_ARREST_SERVICE_NAME: &str = "com.apple.mobile.house_arrest";
const INST_PROXY: &str = "com.apple.mobile.installation_proxy";

#[derive(Parser, Debug)]
#[clap(disable_help_flag = true)]
struct Cli {
    /// Mount point(drive or path)
    #[arg(group = "mount")]
    mount_point: Option<String>,

    /// Use house_arrest service.
    #[arg(short, long, requires = "mount", value_name = "appid")]
    documents: Option<String>,

    #[command(flatten)]
    vers: Option<ListApps>,

    #[arg(short, long)]
    verbose: bool,

    #[arg(long, action = clap::ArgAction::Help)]
    help: Option<bool>,
}

#[derive(Args, Debug)]
#[group(required = false, multiple = false)]
struct ListApps {
    /// Print applications
    #[arg(
        short,
        long,
        hide_possible_values = true,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    apps: Option<bool>,

    /// Print applications with UIFileSharingEnabled
    #[arg(
        short,
        long,
        hide_possible_values = true,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    sharing_apps: Option<bool>,
}

#[derive(Clone, Default)]
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

static DEVICE: OnceLock<Device> = OnceLock::new();
static CLIENT: OnceLock<Client> = OnceLock::new();
static IN_HOUSE_ARREST: OnceLock<bool> = OnceLock::new();
static VERBOSE: OnceLock<bool> = OnceLock::new();

macro_rules! debug {
    ($($arg:tt)*) => {{
        if *crate::VERBOSE.get().unwrap() {
            println!($($arg)*);
        }
    }};
}
pub(crate) use debug;

fn main() {
    let args = Cli::parse();

    let argv: Vec<String> = std::env::args().collect();
    if argv.len() <= 1 {
        return;
    }

    let mount_point = args.mount_point.unwrap_or_default();
    let mp = mount_point.clone();
    ctrlc::set_handler(move || {
        if !mp.is_empty() {
            println!("Unmounting...:{:?}", mp);
            let c = CString::new(mp.clone()).unwrap();
            unsafe { fuse_unmount(c.as_ptr(), std::ptr::null_mut()) };
        }
    })
    .expect("Error setting Ctrl-C handler");

    VERBOSE.get_or_init(|| args.verbose);
    let list_apps = args.vers.is_some();
    let app_id = args.documents.unwrap_or_default();
    let mut opt = Vec::new();
    if !list_apps {
        // exe name
        let exe_name = argv.first().unwrap();
        let c = CString::new(exe_name.clone()).unwrap();
        opt.push(c.into_raw());

        // mount point
        let c = CString::new(mount_point.clone()).unwrap();
        opt.push(c.into_raw());

        // must run in foreground
        let c = CString::new("-f").unwrap();
        opt.push(c.into_raw());
    }

    debug!("Finding device connected...");
    let mut device_info = MaybeUninit::<idevice_t>::zeroed();
    let device_info_ptr = device_info.as_mut_ptr();
    let res = unsafe {
        idevice_new_with_options(
            device_info_ptr,
            std::ptr::null(),
            idevice_options_IDEVICE_LOOKUP_USBMUX | idevice_options_IDEVICE_LOOKUP_NETWORK,
        )
    };
    if res != idevice_error_t_IDEVICE_E_SUCCESS {
        eprintln!("No device found:{:?}", res);
        return;
    }

    let device = unsafe { device_info.assume_init() };

    debug!("Creating lockdown client...");
    let mut client = MaybeUninit::<lockdownd_client_t>::zeroed();
    let client_ptr = client.as_mut_ptr();
    let program_name = CString::new("dokanifuse").unwrap();
    let res =
        unsafe { lockdownd_client_new_with_handshake(device, client_ptr, program_name.as_ptr()) };
    if res != lockdownd_error_t_LOCKDOWN_E_SUCCESS || client_ptr.is_null() {
        eprintln!("lockdown failed:{:?}", res);
        unsafe { idevice_free(device) };
        return;
    }

    let client = unsafe { client.assume_init() };

    let use_house_arrest = *IN_HOUSE_ARREST.get_or_init(|| !app_id.is_empty());
    debug!(
        "Starting {}lockdown service...",
        if use_house_arrest {
            "house_arrest "
        } else if list_apps {
            "instproxy "
        } else {
            ""
        }
    );
    let service_name = if use_house_arrest {
        CString::new(HOUSE_ARREST_SERVICE_NAME).unwrap()
    } else if list_apps {
        CString::new(INST_PROXY).unwrap()
    } else {
        CString::new(AFC_SERVICE_NAME).unwrap()
    };
    let mut descriptor = MaybeUninit::<lockdownd_service_descriptor_t>::zeroed();
    let descriptor_ptr = descriptor.as_mut_ptr();
    let res = unsafe { lockdownd_start_service(client, service_name.as_ptr(), descriptor_ptr) };
    if res != lockdownd_error_t_LOCKDOWN_E_SUCCESS || descriptor_ptr.is_null() {
        eprintln!("lockdownd_start_service failed:{:?}", res);
        unsafe { lockdownd_client_free(client) };
        unsafe { idevice_free(device) };
        return;
    }

    let service_descriptor = unsafe { descriptor.assume_init() };

    if let Some(afc_client) = Client::new(device, service_descriptor) {
        if list_apps {
            let sharing_only = args.vers.unwrap().sharing_apps.is_some();
            debug!("Start listing apps...");
            if let Some(apps) = afc_client.list_apps() {
                print_app(sharing_only, apps);
            }
            afc_client.close();
            unsafe { lockdownd_client_free(client) };
            unsafe { idevice_free(device) };
            return;
        }

        if use_house_arrest && afc_client.start_house_arrest(app_id) < 0 {
            eprintln!("Cannot start_house_arrest");
            unsafe { lockdownd_client_free(client) };
            unsafe { idevice_free(device) };
            return;
        }

        CLIENT.get_or_init(|| afc_client);
    } else {
        eprintln!("Cannot create AfcClient");
        unsafe { lockdownd_client_free(client) };
        unsafe { idevice_free(device) };
        return;
    }

    DEVICE.get_or_init(|| device.into());
    unsafe { lockdownd_client_free(client) };

    let args = fuse_args {
        argc: opt.len() as _,
        argv: opt.as_mut_ptr(),
        allocated: 0,
    };

    let operations = get_fuse_operations();

    debug!("Starting ifuse");

    unsafe {
        fuse_main_real(
            args.argc,
            args.argv,
            &operations as *const _,
            size_of::<fuse_operations>() as _,
            std::ptr::null_mut(),
        )
    };
}

fn real_path(path: *const i8) -> String {
    let raw = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .to_string();

    if !IN_HOUSE_ARREST.get().unwrap() {
        return raw;
    }

    if &raw == "." || &raw == ".." {
        raw
    } else if &raw == "/" {
        "Documents".to_string()
    } else if raw.starts_with('/') {
        format!("Documents{}", raw)
    } else {
        format!("Documents/{}", raw)
    }
}

unsafe extern "C" fn ifuse_init(con: *mut fuse_conn_info) -> *mut c_void {
    (*con).async_read = 0;
    std::ptr::null_mut() as _
}

unsafe extern "C" fn ifuse_cleanup(_data: *mut c_void) {
    if CLIENT.get().unwrap().close() == 0 {
        debug!("Connection closed...");
    }

    if idevice_free(DEVICE.get().unwrap().pointer()) == idevice_error_t_IDEVICE_E_SUCCESS {
        debug!("iDevice freed...")
    }
}

unsafe extern "C" fn ifuse_getattr(path: *const i8, stbuf: *mut stat) -> i32 {
    let info = CLIENT
        .get()
        .unwrap()
        .get_file_info(CString::new(real_path(path)).unwrap().as_ptr());

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
    let info = CLIENT
        .get()
        .unwrap()
        .read_directory(CString::new(real_path(path)).unwrap().as_ptr());

    if info.status == afc_error_t_AFC_E_SUCCESS {
        if let Some(dirs) = extract_list(info) {
            if let Some(filter) = filter {
                for dir in dirs {
                    let dir = CString::new(dir).unwrap();
                    filter(buf, dir.as_ptr(), std::ptr::null(), 1);
                }
            }
            return 0;
        } else {
            return -1;
        }
    }
    -1
}

unsafe extern "C" fn ifuse_statfs(path: *const i8, stats: *mut statvfs) -> i32 {
    let info = CLIENT.get().unwrap().get_device_info();

    if info.status != afc_error_t_AFC_E_SUCCESS {
        return -info.status;
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
    CLIENT.get().unwrap().file_close((*fi).fh);
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

    let info = CLIENT
        .get()
        .unwrap()
        .file_open(CString::new(real_path(path)).unwrap().as_ptr(), mode);

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

    let info = CLIENT
        .get()
        .unwrap()
        .file_seek((*fi).fh, offset as i64, SEEK_SET as _);

    if info.status != afc_error_t_AFC_E_SUCCESS {
        return -info.status;
    }

    let info = CLIENT.get().unwrap().file_read((*fi).fh, size as _);

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
    debug!("ifuse_create");
    ifuse_open(path, fi)
}

unsafe extern "C" fn ifuse_write(
    path: *const i8,
    buf: *const i8,
    size: u32,
    offset: u64,
    fi: *mut fuse_file_info,
) -> i32 {
    debug!("ifuse_write");
    if size == 0 {
        return 0;
    }

    let info = CLIENT
        .get()
        .unwrap()
        .file_seek((*fi).fh, offset as _, SEEK_SET as _);
    if info.status != afc_error_t_AFC_E_SUCCESS {
        return -info.status;
    }

    let info = CLIENT.get().unwrap().file_write((*fi).fh, buf, size);
    if info.status != afc_error_t_AFC_E_SUCCESS {
        return -info.status;
    }
    size as _
}

unsafe extern "C" fn ifuse_truncate(path: *const i8, size: u64) -> i32 {
    debug!("ifuse_truncate");
    let info = CLIENT
        .get()
        .unwrap()
        .truncate(CString::new(real_path(path)).unwrap().as_ptr(), size);
    if info.status != afc_error_t_AFC_E_SUCCESS {
        return -info.status;
    }
    0
}

unsafe extern "C" fn ifuse_unlink(path: *const i8) -> i32 {
    debug!("ifuse_unlink");
    let info = CLIENT
        .get()
        .unwrap()
        .remove_path(CString::new(real_path(path)).unwrap().as_ptr());
    if info.status != afc_error_t_AFC_E_SUCCESS {
        return -info.status;
    }
    0
}

unsafe extern "C" fn ifuse_mkdir(path: *const i8, ignored: u64) -> i32 {
    debug!("ifuse_mkdir");

    let info = CLIENT
        .get()
        .unwrap()
        .make_directory(CString::new(real_path(path)).unwrap().as_ptr());
    if info.status != afc_error_t_AFC_E_SUCCESS {
        return -info.status;
    }
    0
}

unsafe extern "C" fn ifuse_fsync(path: *const i8, datasync: i32, fi: *mut fuse_file_info) -> i32 {
    debug!("ifuse_fsync");
    0
}

unsafe extern "C" fn ifuse_chmod(path: *const i8, mode: u64) -> i32 {
    debug!("ifuse_fsync");
    0
}

unsafe extern "C" fn ifuse_chown(file: *const i8, user: u32, group: u32) -> i32 {
    debug!("ifuse_fsync");
    0
}

unsafe extern "C" fn ifuse_readlink(a: *const i8, b: *mut i8, c: u32) -> i32 {
    debug!("ifuse_readlink");
    0
}

unsafe extern "C" fn ifuse_symlink(a: *const i8, b: *const i8) -> i32 {
    debug!("ifuse_symlink");
    0
}

unsafe extern "C" fn ifuse_link(a: *const i8, b: *const i8) -> i32 {
    debug!("ifuse_link");
    0
}

unsafe extern "C" fn ifuse_rename(a: *const i8, b: *const i8) -> i32 {
    debug!("ifuse_rename");
    0
}

unsafe extern "C" fn ifuse_utimens(a: *const i8, b: *const timespec) -> i32 {
    debug!("ifuse_utimens");
    0
}

pub fn get_fuse_operations() -> fuse_operations {
    fuse_operations {
        getattr: Some(ifuse_getattr),
        statfs: Some(ifuse_statfs),
        readdir: Some(ifuse_readdir),
        mkdir: Some(ifuse_mkdir),
        rmdir: Some(ifuse_unlink),
        create: Some(ifuse_create),
        open: Some(ifuse_open),
        read: Some(ifuse_read),
        write: Some(ifuse_write),
        truncate: Some(ifuse_truncate),
        readlink: Some(ifuse_readlink),
        symlink: Some(ifuse_symlink),
        link: Some(ifuse_link),
        unlink: Some(ifuse_unlink),
        rename: Some(ifuse_rename),
        utimens: Some(ifuse_utimens),
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
