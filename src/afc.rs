use crate::bindings::*;
use byteorder::{ByteOrder, LittleEndian, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use std::{
    collections::HashMap,
    ffi::{CStr, CString},
    io::{ErrorKind, Read, Write},
    mem::MaybeUninit,
    net::TcpStream,
    os::{raw::c_char, windows::io::FromRawSocket},
    slice::from_raw_parts,
    sync::{mpsc, Mutex},
};
//pub type __mode_t = u64; //::std::os::raw::c_uint;
pub const AFCMAGIC: &[u8; 8] = b"CFA6LPAA";

#[derive(Copy, Clone, Debug, FromPrimitive, ToPrimitive, PartialEq, PartialOrd)]
pub enum afc_opcode_t {
    STATUS = 0x00000001,
    DATA = 0x00000002,                     // Data */
    READ_DIR = 0x00000003,                 // ReadDir */
    READ_FILE = 0x00000004,                // ReadFile */
    WRITE_FILE = 0x00000005,               // WriteFile */
    WRITE_PART = 0x00000006,               // WritePart */
    TRUNCATE = 0x00000007,                 // TruncateFile */
    REMOVE_PATH = 0x00000008,              // RemovePath */
    MAKE_DIR = 0x00000009,                 // MakeDir */
    GET_FILE_INFO = 0x0000000a,            // GetFileInfo */
    GET_DEVINFO = 0x0000000b,              // GetDeviceInfo */
    WRITE_FILE_ATOM = 0x0000000c,          // WriteFileAtomic (tmp file+rename) */
    FILE_OPEN = 0x0000000d,                // FileRefOpen */
    FILE_OPEN_RES = 0x0000000e,            // FileRefOpenResult */
    READ = 0x0000000f,                     // FileRefRead */
    WRITE = 0x00000010,                    // FileRefWrite */
    FILE_SEEK = 0x00000011,                // FileRefSeek */
    FILE_TELL = 0x00000012,                // FileRefTell */
    FILE_TELL_RES = 0x00000013,            // FileRefTellResult */
    FILE_CLOSE = 0x00000014,               // FileRefClose */
    FILE_SET_SIZE = 0x00000015,            // FileRefSetFileSize (ftruncate) */
    GET_CON_INFO = 0x00000016,             // GetConnectionInfo */
    SET_CON_OPTIONS = 0x00000017,          // SetConnectionOptions */
    RENAME_PATH = 0x00000018,              // RenamePath */
    SET_FS_BS = 0x00000019,                // SetFSBlockSize (0x800000) */
    SET_SOCKET_BS = 0x0000001A,            // SetSocketBlockSize (0x800000) */
    FILE_LOCK = 0x0000001B,                // FileRefLock */
    MAKE_LINK = 0x0000001C,                // MakeLink */
    SET_FILE_TIME = 0x0000001E,            // set st_mtime */
    REMOVE_PATH_AND_CONTENTS = 0x00000022, /* RemovePathAndContents */
}

pub struct AfcClient {
    pub(crate) socket: Mutex<TcpStream>,
    pub(crate) packet_num: Mutex<u64>,
    pub(crate) sfd: i32,
    track: bool,
}

impl AfcClient {
    pub fn default() -> Self {
        Self {
            socket: unsafe { Mutex::new(TcpStream::from_raw_socket(0)) },
            packet_num: Mutex::new(0),
            sfd: 0,
            track: false,
        }
    }

    pub fn new(
        device: *mut idevice_private,
        service: *mut lockdownd_service_descriptor,
    ) -> Option<Self> {
        let mut udid: *mut ::std::os::raw::c_char = std::ptr::null_mut();
        unsafe { idevice_get_udid(device, &mut udid) };

        let mut usbdev = MaybeUninit::<usbmuxd_device_info_t>::zeroed();
        let usbdev_ptr = usbdev.as_mut_ptr();

        let res =
            unsafe { usbmuxd_get_device(udid, usbdev_ptr, idevice_options_IDEVICE_LOOKUP_USBMUX) };

        if res != 0 {
            let handle = unsafe { usbdev.assume_init() };
            let sfd = unsafe { usbmuxd_connect(handle.handle, (*service).port) };
            Some(Self {
                socket: unsafe { Mutex::new(TcpStream::from_raw_socket(sfd as _)) },
                packet_num: Mutex::new(0),
                sfd,
                track: false,
            })
        } else {
            None
        }
    }

    pub fn close(&mut self) -> i32 {
        if self.sfd > 0 {
            let result = unsafe { usbmuxd_disconnect(self.sfd as _) };
            self.sfd = -1;
            result
        } else {
            self.sfd
        }
    }

    pub fn get_file_info(&mut self, path: *const i8) -> Option<AfcResponse> {
        self.operate(
            afc_opcode_t::GET_FILE_INFO,
            afc_stat_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn read_directory(&mut self, path: *const i8) -> Option<AfcResponse> {
        self.operate(
            afc_opcode_t::READ_DIR,
            afc_stat_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn get_device_info(&mut self) -> Option<AfcResponse> {
        self.operate(afc_opcode_t::GET_DEVINFO, Vec::new())
    }

    pub fn file_open(&mut self, path: *const i8, mode: u64) -> Option<AfcResponse> {
        self.operate(
            afc_opcode_t::FILE_OPEN,
            afc_fopen_t {
                mode: mode as _,
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn file_close(&mut self, handle: u64) -> Option<AfcResponse> {
        self.operate(afc_opcode_t::FILE_CLOSE, afc_fclose_t { handle }.to_bytes())
    }

    pub fn file_seek(&mut self, handle: u64, offset: u64, whence: u64) -> Option<AfcResponse> {
        self.operate(
            afc_opcode_t::FILE_SEEK,
            afc_seek_t {
                handle,
                offset,
                whence,
            }
            .to_bytes(),
        )
    }

    pub fn file_read(&mut self, handle: u64, size: u64) -> Option<AfcResponse> {
        self.operate(afc_opcode_t::READ, afc_fread_t { handle, size }.to_bytes())
    }

    pub fn file_write(&mut self, handle: u64, data: *const i8, size: u32) -> Option<AfcResponse> {
        let buf: &[u8] = unsafe { from_raw_parts(data as *const __uint8_t, size as _) };
        self.operate(
            afc_opcode_t::WRITE,
            afc_fwrite_t {
                handle,
                data: buf.to_vec(),
            }
            .to_bytes(),
        )
    }

    pub fn truncate(&mut self, path: *const i8, size: u64) -> Option<AfcResponse> {
        self.operate(
            afc_opcode_t::TRUNCATE,
            afc_truncate_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
                newsize: size,
            }
            .to_bytes(),
        )
    }

    pub fn remove_path(&mut self, path: *const i8) -> Option<AfcResponse> {
        self.operate(
            afc_opcode_t::REMOVE_PATH,
            afc_rm_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn operate(&mut self, operation: afc_opcode_t, data: Vec<u8>) -> Option<AfcResponse> {
        let socket = self.socket.lock().unwrap();
        let mut num = self.packet_num.lock().unwrap();
        *num += 1;
        self.sendall(socket, operation, data, *num)
    }

    fn sendall(
        &self,
        mut socket: std::sync::MutexGuard<TcpStream>,
        operation: afc_opcode_t,
        data: Vec<u8>,
        packet_num: u64,
    ) -> Option<AfcResponse> {
        let request_header = AfcHeader {
            magic: *AFCMAGIC,
            entire_length: 40 + data.len() as u64,
            this_length: 40 + data.len() as u64,
            packet_num,
            operation: operation as u64,
        };

        let mut packet = request_header.to_bytes();
        packet.extend_from_slice(&data);

        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        socket
            .set_write_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();

        socket.write_all(&packet).unwrap();

        let mut header = vec![0; std::mem::size_of::<AfcHeader>() as _];
        loop {
            match socket.read(&mut header) {
                Ok(size) => {
                    if let Some(response_header) = parse_header(&header) {
                        let response_magic = response_header.magic;
                        let response_packet_pnum = response_header.packet_num;
                        if AFCMAGIC != &response_magic || packet_num != response_packet_pnum {
                            continue;
                        } else {
                            break;
                        }
                    } else {
                        return None;
                    }
                }
                Err(e) => {
                    if e.kind() == ErrorKind::TimedOut {
                        println!("TCP timeout");
                        return None;
                    } else {
                        println!("TCP error: {:?}", e);
                        return None;
                    }
                }
            }
        }

        if let Some(response_header) = parse_header(&header) {
            let entire_len =
                response_header.entire_length - std::mem::size_of::<AfcHeader>() as u64;

            let mut status_only = false;
            if response_header.operation == 1 && entire_len <= 0 {
                return None;
            }

            if response_header.operation == 1 {
                status_only = true
            }

            let mut response = vec![0; entire_len as _];

            socket.read_exact(&mut response).unwrap();

            let afc_data = parse_afc(operation, &response, status_only);
            Some(AfcResponse {
                header: response_header,
                data: afc_data,
            })
        } else {
            None
        }
    }
}

fn parse_header(data: &[u8]) -> Option<AfcHeader> {
    if data.len() < std::mem::size_of::<AfcHeader>() {
        return None;
    }

    let mut header = AfcHeader {
        magic: [0; 8],
        entire_length: LittleEndian::read_u64(&data[8..16]),
        this_length: LittleEndian::read_u64(&data[16..24]),
        packet_num: LittleEndian::read_u64(&data[24..32]),
        operation: LittleEndian::read_u64(&data[32..40]),
    };
    header.magic.copy_from_slice(&data[0..8]);
    Some(header)
}

fn parse_afc(operation: afc_opcode_t, data: &[u8], status_only: bool) -> Response {
    if status_only {
        return Response::Status(to_i32(data));
    }

    match operation {
        afc_opcode_t::FILE_OPEN => Response::Number(to_u64(data)),
        afc_opcode_t::READ | afc_opcode_t::WRITE => Response::Byte(data.to_vec()),
        _ => {
            let parts: Vec<Vec<u8>> = data
                .split(|&b| b == 0)
                .filter(|p| !p.is_empty())
                .map(|p| p.to_vec())
                .collect();
            Response::List(to_vec_string(parts))
        }
    }
}

pub trait t_afc_struct {
    fn to_bytes(&self) -> Vec<u8>;
}

pub struct afc_stat_t {
    pub filename: Vec<u8>,
}
impl t_afc_struct for afc_stat_t {
    fn to_bytes(&self) -> Vec<u8> {
        self.filename.clone()
    }
}
pub struct afc_readdir_t {
    pub filename: Vec<u8>,
}
impl t_afc_struct for afc_readdir_t {
    fn to_bytes(&self) -> Vec<u8> {
        self.filename.clone()
    }
}

pub struct afc_fopen_t {
    pub mode: afc_file_mode_t,
    pub filename: Vec<u8>,
}
impl t_afc_struct for afc_fopen_t {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = self.mode.to_le_bytes().to_vec();
        buf.extend(&self.filename);
        buf
    }
}

pub struct afc_fread_t {
    pub handle: u64,
    pub size: u64,
}
impl t_afc_struct for afc_fread_t {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = self.handle.to_le_bytes().to_vec();
        buf.extend_from_slice(&self.size.to_le_bytes());
        buf
    }
}

pub struct afc_fwrite_t {
    pub handle: u64,
    pub data: Vec<u8>,
}
impl t_afc_struct for afc_fwrite_t {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = self.handle.to_le_bytes().to_vec();
        buf.extend(&self.data);
        buf
    }
}

pub struct afc_fclose_t {
    pub handle: u64,
}
impl t_afc_struct for afc_fclose_t {
    fn to_bytes(&self) -> Vec<u8> {
        self.handle.to_le_bytes().to_vec()
    }
}

pub struct afc_seek_t {
    pub handle: u64,
    pub whence: u64,
    pub offset: u64,
}
impl t_afc_struct for afc_seek_t {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = self.handle.to_le_bytes().to_vec();
        buf.extend_from_slice(&self.whence.to_le_bytes());
        buf.extend_from_slice(&self.offset.to_le_bytes());
        buf
    }
}

pub struct afc_truncate_t {
    pub filename: Vec<u8>,
    pub newsize: u64,
}
impl t_afc_struct for afc_truncate_t {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = self.filename.clone();
        buf.extend_from_slice(&self.newsize.to_le_bytes());
        buf
    }
}

pub struct afc_rm_t {
    pub filename: Vec<u8>,
}
impl t_afc_struct for afc_rm_t {
    fn to_bytes(&self) -> Vec<u8> {
        self.filename.clone()
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct AfcHeader {
    pub magic: [u8; 8],
    pub entire_length: u64,
    pub this_length: u64,
    pub packet_num: u64,
    pub operation: u64,
}

impl t_afc_struct for AfcHeader {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(40); // 5 × u64 + magic
        buf.extend_from_slice(&self.magic);
        buf.write_u64::<LittleEndian>(self.entire_length).unwrap();
        buf.write_u64::<LittleEndian>(self.this_length).unwrap();
        buf.write_u64::<LittleEndian>(self.packet_num).unwrap();
        buf.write_u64::<LittleEndian>(self.operation).unwrap();
        buf
    }
}

#[derive(Debug, Clone)]
pub struct AfcResponse {
    pub header: AfcHeader,
    pub data: Response,
}

#[derive(Debug, Clone)]
pub enum Response {
    List(Vec<String>),
    Number(u64),
    Byte(Vec<u8>),
    Status(i32),
}

pub fn extract_list(res: Option<AfcResponse>) -> Option<Vec<String>> {
    if let Some(AfcResponse {
        data: Response::List(list),
        ..
    }) = res
    {
        Some(list)
    } else {
        None
    }
}

pub fn extract_num(res: Option<AfcResponse>) -> Option<u64> {
    if let Some(AfcResponse {
        data: Response::Number(num),
        ..
    }) = res
    {
        Some(num)
    } else {
        None
    }
}

pub fn extract_byte(res: Option<AfcResponse>) -> Option<Vec<u8>> {
    if let Some(AfcResponse {
        data: Response::Byte(byte),
        ..
    }) = res
    {
        Some(byte)
    } else {
        None
    }
}

pub fn parse_status(res: Option<AfcResponse>) -> i32 {
    if let Some(AfcResponse {
        data: Response::Status(num),
        ..
    }) = res
    {
        num
    } else {
        -1
    }
}

pub fn to_vec_string(parts: Vec<Vec<u8>>) -> Vec<String> {
    let mut values = Vec::new();
    for part in parts {
        if let Ok(k) = core::str::from_utf8(&part) {
            values.push(k.to_string());
        }
    }
    values
}

pub fn to_u64(parts: &[u8]) -> u64 {
    parts[0] as _
}

pub fn to_i32(parts: &[u8]) -> i32 {
    parts[0] as _
}

pub fn to_map(data: Vec<String>) -> HashMap<String, String> {
    data.chunks_exact(2)
        .map(|chunk| (chunk[0].clone(), chunk[1].clone()))
        .collect::<HashMap<_, _>>()
}

pub const EPERM: i32 = 1; /* Operation not permitted */
pub const ENOENT: i32 = 2; /* No such file or directory */
pub const ESRCH: i32 = 3; /* No such process */
pub const EINTR: i32 = 4; /* Interrupted system call */
pub const EIO: i32 = 5; /* I/O error */
pub const ENXIO: i32 = 6; /* No such device or address */
pub const E2BIG: i32 = 7; /* Argument list too long */
pub const ENOEXEC: i32 = 8; /* Exec format error */
pub const EBADF: i32 = 9; /* Bad file number */
pub const ECHILD: i32 = 10; /* No child processes */
pub const EAGAIN: i32 = 11; /* Try again */
pub const ENOMEM: i32 = 12; /* Out of memory */
pub const EACCES: i32 = 13; /* Permission denied */
pub const EFAULT: i32 = 14; /* Bad address */
pub const ENOTBLK: i32 = 15; /* Block device required */
pub const EBUSY: i32 = 16; /* Device or resource busy */
pub const EEXIST: i32 = 17; /* File exists */
pub const EXDEV: i32 = 18; /* Cross-device link */
pub const ENODEV: i32 = 19; /* No such device */
pub const ENOTDIR: i32 = 20; /* Not a directory */
pub const EISDIR: i32 = 21; /* Is a directory */
pub const EINVAL: i32 = 22; /* Invalid argument */
pub const ENFILE: i32 = 23; /* File table overflow */
pub const EMFILE: i32 = 24; /* Too many open files */
pub const ENOTTY: i32 = 25; /* Not a typewriter */
pub const ETXTBSY: i32 = 26; /* Text file busy */
pub const EFBIG: i32 = 27; /* File too large */
pub const ENOSPC: i32 = 28; /* No space left on device */
pub const ESPIPE: i32 = 29; /* Illegal seek */
pub const EROFS: i32 = 30; /* Read-only file system */
pub const EMLINK: i32 = 31; /* Too many links */
pub const EPIPE: i32 = 32; /* Broken pipe */
pub const EDOM: i32 = 33; /* Math argument out of domain of func */
pub const ERANGE: i32 = 34; /* Math result not representable */
