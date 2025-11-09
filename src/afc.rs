#![allow(non_camel_case_types, non_upper_case_globals, dead_code)]
#![allow(clippy::upper_case_acronyms)]
use crate::bindings::*;
use byteorder::{ByteOrder, LittleEndian, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use std::{collections::HashMap, ffi::CStr, mem::MaybeUninit, slice::from_raw_parts, sync::Mutex};

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

pub struct Client {
    pub(crate) socket: Mutex<IDeviceConnection>,
    pub(crate) packet_num: Mutex<u64>,
}

pub(crate) struct IDeviceConnection {
    pub(crate) conn: usize,
}

impl IDeviceConnection {
    pub(crate) fn new(connection: idevice_connection_t) -> Self {
        Self {
            conn: connection as usize,
        }
    }

    pub(crate) fn none() -> Self {
        Self { conn: 0 }
    }

    pub(crate) fn connection(&self) -> idevice_connection_t {
        self.conn as idevice_connection_t
    }
}

impl Client {
    pub fn default() -> Self {
        Self {
            socket: Mutex::new(IDeviceConnection::none()),
            packet_num: Mutex::new(0),
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

        if unsafe { usbmuxd_get_device(udid, usbdev_ptr, idevice_options_IDEVICE_LOOKUP_USBMUX) }
            == 0
        {
            return None;
        }

        let mut device_connection = MaybeUninit::<idevice_connection_t>::zeroed();
        let device_connection_ptr = device_connection.as_mut_ptr();
        if unsafe { idevice_connect(device, (*service).port, device_connection_ptr) }
            == idevice_error_t_IDEVICE_E_SUCCESS
        {
            let connection = unsafe { device_connection.assume_init() };
            if (unsafe { *service }).ssl_enabled == 1 {
                unsafe { idevice_connection_enable_ssl(connection) };
            }

            Some(Self {
                socket: Mutex::new(IDeviceConnection::new(connection)),
                packet_num: Mutex::new(0),
            })
        } else {
            None
        }
    }

    pub fn close(&self) -> i32 {
        unsafe { idevice_disconnect(self.socket.lock().unwrap().connection()) }
    }

    pub fn get_file_info(&self, path: *const i8) -> AfcResponse {
        self.operate(
            afc_opcode_t::GET_FILE_INFO,
            0,
            afc_stat_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn read_directory(&self, path: *const i8) -> AfcResponse {
        self.operate(
            afc_opcode_t::READ_DIR,
            0,
            afc_readdir_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn get_device_info(&self) -> AfcResponse {
        self.operate(afc_opcode_t::GET_DEVINFO, 0, Vec::new())
    }

    pub fn file_open(&self, path: *const i8, mode: u64) -> AfcResponse {
        self.operate(
            afc_opcode_t::FILE_OPEN,
            0,
            afc_fopen_t {
                mode: mode as _,
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn file_close(&self, handle: u64) -> AfcResponse {
        self.operate(
            afc_opcode_t::FILE_CLOSE,
            0,
            afc_fclose_t { handle }.to_bytes(),
        )
    }

    pub fn file_seek(&self, handle: u64, offset: i64, whence: u64) -> AfcResponse {
        self.operate(
            afc_opcode_t::FILE_SEEK,
            0,
            afc_seek_t {
                handle,
                offset,
                whence,
            }
            .to_bytes(),
        )
    }

    pub fn file_read(&self, handle: u64, size: u64) -> AfcResponse {
        self.operate(
            afc_opcode_t::READ,
            0,
            afc_fread_t { handle, size }.to_bytes(),
        )
    }

    pub fn file_write(&self, handle: u64, data: *const i8, size: u32) -> AfcResponse {
        let buf: &[u8] = unsafe { from_raw_parts(data as *const u8, size as _) };
        self.operate(
            afc_opcode_t::WRITE,
            size_of::<u64>() as u64,
            afc_fwrite_t {
                handle,
                data: buf.to_vec(),
            }
            .to_bytes(),
        )
    }

    pub fn truncate(&self, path: *const i8, size: u64) -> AfcResponse {
        self.operate(
            afc_opcode_t::TRUNCATE,
            0,
            afc_truncate_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
                newsize: size,
            }
            .to_bytes(),
        )
    }

    pub fn remove_path(&self, path: *const i8) -> AfcResponse {
        self.operate(
            afc_opcode_t::REMOVE_PATH,
            0,
            afc_rm_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn make_directory(&self, path: *const i8) -> AfcResponse {
        self.operate(
            afc_opcode_t::MAKE_DIR,
            0,
            afc_mkdir_t {
                filename: unsafe { CStr::from_ptr(path).to_bytes_with_nul().to_vec() },
            }
            .to_bytes(),
        )
    }

    pub fn operate(&self, operation: afc_opcode_t, data_len: u64, payload: Vec<u8>) -> AfcResponse {
        let socket = self.socket.lock().unwrap();
        let mut num = self.packet_num.lock().unwrap();
        *num += 1;

        let payload_len = payload.len() as u64;
        let data_len = if data_len > 0 { data_len } else { payload_len };
        let afc_header_size = size_of::<AfcHeader>() as u64;
        let request_header = AfcHeader {
            magic: *AFCMAGIC,
            entire_length: afc_header_size + payload_len,
            this_length: afc_header_size + data_len,
            packet_num: *num,
            operation: operation as u64,
        };

        let (res, _) = self.send_packet(&socket, request_header.to_bytes(), afc_header_size as u32);
        if res != idevice_error_t_IDEVICE_E_SUCCESS {
            return AfcResponse::error_with(res);
        }

        let (res, _) = self.send_packet(&socket, payload, payload_len as _);
        if res != idevice_error_t_IDEVICE_E_SUCCESS {
            return AfcResponse::error_with(res);
        }

        self.receive_packet(&socket, operation, *num)
    }

    pub(crate) fn send_packet(
        &self,
        connection: &std::sync::MutexGuard<IDeviceConnection>,
        data: Vec<u8>,
        len: u32,
    ) -> (idevice_error_t, u32) {
        let mut sent_bytes = 0;
        let res = unsafe {
            idevice_connection_send(
                connection.connection(),
                data.as_ptr() as *const i8,
                len,
                &mut sent_bytes,
            )
        };
        (res, sent_bytes)
    }

    fn receive_packet(
        &self,
        connection: &std::sync::MutexGuard<IDeviceConnection>,
        operation: afc_opcode_t,
        packet_num: u64,
    ) -> AfcResponse {
        let mut header = vec![0u8; std::mem::size_of::<AfcHeader>() as _];

        let mut recv_bytes = 0;
        let response_header = match unsafe {
            idevice_connection_receive_timeout(
                connection.connection(),
                header.as_mut_ptr() as *mut i8,
                header.len() as u32,
                &mut recv_bytes,
                5000,
            )
        } {
            idevice_error_t_IDEVICE_E_SUCCESS => {
                if let Some(response_header) = parse_header(&header) {
                    let response_magic = response_header.magic;
                    let response_packet_pnum = response_header.packet_num;
                    /* check if it's a valid AFC header */
                    /* check if it has the correct packet number */
                    if AFCMAGIC != &response_magic || packet_num != response_packet_pnum {
                        println!("Invalid response header");
                        None
                    } else {
                        Some(response_header)
                    }
                } else {
                    None
                }
            }
            idevice_error_t_IDEVICE_E_TIMEOUT => {
                println!("TCP timeout");
                None
            }
            val => {
                println!("TCP error: {:?}", val);
                None
            }
        };

        /* then, read the attached packet */
        if let Some(response_header) = response_header {
            if response_header.this_length < std::mem::size_of::<AfcHeader>() as u64 {
                println!("Invalid AFCPacket header received!");
                return AfcResponse::error();
            }

            if response_header.this_length == response_header.entire_length
                && response_header.entire_length == std::mem::size_of::<AfcHeader>() as u64
            {
                println!("Empty AFCPacket received!");
                return AfcResponse::error();
            }

            let entire_len =
                response_header.entire_length - std::mem::size_of::<AfcHeader>() as u64;
            let this_len = response_header.this_length - std::mem::size_of::<AfcHeader>() as u64;

            let status_only = response_header.operation == 1;

            let mut recv_bytes = 0;
            let mut response = Vec::new();

            if this_len > 0 {
                let mut buf = vec![0u8; this_len as _];
                unsafe {
                    idevice_connection_receive_timeout(
                        connection.connection(),
                        buf.as_mut_ptr() as *mut i8,
                        this_len as u32,
                        &mut recv_bytes,
                        5000,
                    )
                };

                if recv_bytes == 0 {
                    println!("Did not get packet contents!");
                    return AfcResponse::error();
                }

                if recv_bytes < this_len as _ {
                    println!("Could not receive this_len={:?} bytes", this_len);
                    return AfcResponse::error();
                }
                response.extend_from_slice(&buf[0..recv_bytes as usize]);
            }

            let mut current_count = this_len;

            if entire_len > this_len {
                while current_count < entire_len {
                    let mut buf = vec![0u8; (entire_len - current_count) as _];
                    unsafe {
                        idevice_connection_receive_timeout(
                            connection.connection(),
                            buf.as_mut_ptr() as *mut i8,
                            (entire_len - current_count) as u32,
                            &mut recv_bytes,
                            5000,
                        )
                    };
                    if recv_bytes == 0 {
                        println!("Error receiving data (recv returned {:?})", recv_bytes);
                        break;
                    }

                    response.extend_from_slice(&buf[0..recv_bytes as usize]);
                    current_count += recv_bytes as u64;
                }

                if current_count < entire_len {
                    println!(
                        "WARNING: could not receive full packet (read {:?}, size {:?})",
                        current_count, entire_len
                    );
                }
            }

            let afc_data = parse_afc(operation, &response, status_only);
            AfcResponse {
                status: afc_data.0,
                header: response_header,
                data: afc_data.1,
            }
        } else {
            AfcResponse::error()
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

fn parse_afc(operation: afc_opcode_t, data: &[u8], status_only: bool) -> (afc_error_t, Response) {
    let status = if status_only { to_i32(data) } else { 0 };

    let response = match operation {
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
    };
    (status, response)
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
    pub offset: i64,
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

pub struct afc_mkdir_t {
    pub filename: Vec<u8>,
}
impl t_afc_struct for afc_mkdir_t {
    fn to_bytes(&self) -> Vec<u8> {
        self.filename.clone()
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AfcHeader {
    pub magic: [u8; 8],
    pub entire_length: u64,
    pub this_length: u64,
    pub packet_num: u64,
    pub operation: u64,
}

impl t_afc_struct for AfcHeader {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(size_of::<AfcHeader>());
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
    pub status: afc_error_t,
    pub header: AfcHeader,
    pub data: Response,
}

impl AfcResponse {
    pub(crate) fn error() -> Self {
        Self {
            status: afc_error_t_AFC_E_UNKNOWN_ERROR,
            header: AfcHeader::default(),
            data: Response::None,
        }
    }

    pub(crate) fn error_with(status: afc_error_t) -> Self {
        Self {
            status,
            header: AfcHeader::default(),
            data: Response::None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Response {
    List(Vec<String>),
    Number(u64),
    Byte(Vec<u8>),
    None,
}

pub fn extract_list(res: AfcResponse) -> Option<Vec<String>> {
    match res.data {
        Response::List(list) => Some(list),
        _ => None,
    }
}

pub fn extract_num(res: AfcResponse) -> Option<u64> {
    match res.data {
        Response::Number(num) => Some(num),
        _ => None,
    }
}

pub fn extract_byte(res: AfcResponse) -> Option<Vec<u8>> {
    match res.data {
        Response::Byte(byte) => Some(byte),
        _ => None,
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
