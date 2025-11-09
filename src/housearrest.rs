use crate::{
    afc::{Client, IDeviceConnection},
    idevice_connection_receive_timeout, idevice_error_t_IDEVICE_E_SUCCESS,
};
use byteorder::{BigEndian, ByteOrder};
use plist::Value;

impl Client {
    pub fn start_house_arrest(&self, app_id: String) -> i32 {
        let socket = self.socket.lock().unwrap();

        let mut command_plist = plist::Dictionary::new();

        command_plist.insert("Command".into(), Value::String("VendDocuments".into()));
        command_plist.insert("Identifier".into(), Value::String(app_id));

        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, &command_plist).unwrap();
        let len = payload.len() as u32;
        let prefix = len.to_be_bytes();

        let (res, _) = self.send_packet(&socket, prefix.to_vec(), prefix.len() as _);
        if res != idevice_error_t_IDEVICE_E_SUCCESS {
            return -1;
        }
        let (res, _) = self.send_packet(&socket, payload, len);
        if res != idevice_error_t_IDEVICE_E_SUCCESS {
            return -1;
        }

        if let Ok(dict) = receive_plist(&socket) {
            if let Some(status) = dict.get("Status") {
                if status.as_string().unwrap() == "Complete" {
                    return 0;
                }
            }
        }

        -1
    }
}

fn receive_plist(
    connection: &std::sync::MutexGuard<IDeviceConnection>,
) -> Result<plist::Dictionary, String> {
    unsafe {
        let mut pktlen = vec![0u8; size_of::<u32>()];
        let mut recv_bytes = 0;
        let res = idevice_connection_receive_timeout(
            connection.connection(),
            pktlen.as_mut_ptr() as *mut i8,
            pktlen.len() as _,
            &mut recv_bytes,
            5000,
        );
        if res != 0 {
            return Err(format!("initial read failed! status={:?}", res));
        }

        let mut curlen = 0;
        let pktlen = BigEndian::read_u32(&pktlen);

        let mut content = Vec::new();

        while curlen < pktlen {
            let mut buf = vec![0u8; (pktlen - curlen) as usize];
            let res = idevice_connection_receive_timeout(
                connection.connection(),
                buf.as_mut_ptr() as *mut i8,
                buf.len() as _,
                &mut recv_bytes,
                5000,
            );
            if res != idevice_error_t_IDEVICE_E_SUCCESS {
                return Err(format!("Read failed! status={:?}", res));
            }
            content.extend_from_slice(&buf[0..recv_bytes as usize]);
            curlen += recv_bytes;
        }

        if curlen < pktlen {
            return Err(format!(
                "received incomplete packet ({:?} of {:?} bytes)",
                curlen, pktlen
            ));
        }

        let reader = std::io::Cursor::new(content);
        if let Ok(xml) = plist::Value::from_reader_xml(reader) {
            if let Some(dict) = xml.into_dictionary() {
                return Ok(dict);
            }
        }

        Err("Received unexpected non-plist content".to_string())
    }
}
