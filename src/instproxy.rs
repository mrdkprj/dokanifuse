use crate::{afc::Client, idevice_connection_receive_timeout, idevice_error_t_IDEVICE_E_SUCCESS};
use plist::Value;
use std::collections::HashMap;

impl Client {
    pub fn list_apps(&self) -> Option<Vec<HashMap<String, String>>> {
        let socket = self.socket.lock().unwrap();
        let mut num = self.packet_num.lock().unwrap();
        *num += 1;

        let mut client_options = plist::Dictionary::new();
        client_options.insert("ApplicationType".into(), Value::String("Any".into()));

        let mut command_plist = plist::Dictionary::new();

        let attrs = vec![
            Value::String("CFBundleIdentifier".into()),
            Value::String("CFBundleDisplayName".into()),
            Value::String("CFBundleVersion".into()),
            Value::String("UIFileSharingEnabled".into()),
        ];
        let attrs_val = Value::from(attrs);
        client_options.insert("ReturnAttributes".into(), attrs_val);
        let opts = Value::from(client_options);
        command_plist.insert("Command".into(), Value::String("Browse".into()));
        command_plist.insert("ClientOptions".into(), opts);

        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, &command_plist).unwrap();
        let len = payload.len() as u32;
        let prefix = len.to_be_bytes();

        let (res, _) = self.send_packet(&socket, prefix.to_vec(), prefix.len() as _);
        if res != idevice_error_t_IDEVICE_E_SUCCESS {
            return None;
        }
        let (res, _) = self.send_packet(&socket, payload, len);
        if res != idevice_error_t_IDEVICE_E_SUCCESS {
            return None;
        }

        let mut len_buf = [0u8; 4];
        let mut recv_bytes = 0;
        unsafe {
            idevice_connection_receive_timeout(
                socket.connection(),
                len_buf.as_mut_ptr() as *mut i8,
                4,
                &mut recv_bytes,
                5000,
            )
        };

        let mut reply_len = u32::from_be_bytes(len_buf) as usize;
        let mut results: Vec<HashMap<String, String>> = Vec::new();
        loop {
            let mut reply_buf = vec![0u8; reply_len];
            let res = unsafe {
                idevice_connection_receive_timeout(
                    socket.connection(),
                    reply_buf.as_mut_ptr() as *mut i8,
                    reply_len as _,
                    &mut recv_bytes,
                    5000,
                )
            };
            if res != 0 {
                break;
            }

            if reply_buf.is_empty() {
                break;
            }

            let reply = plist::from_bytes::<plist::Dictionary>(&reply_buf).unwrap();
            let status = reply.get("Status").unwrap().as_string().unwrap();
            if status == "Complete" {
                break;
            }

            for x in reply
                .get("CurrentList")
                .unwrap()
                .as_array()
                .unwrap()
                .to_vec()
            {
                let mut map = HashMap::new();
                for (k, v) in x.as_dictionary().unwrap() {
                    map.insert(k.to_string(), v.as_string().unwrap_or("").to_string());
                }
                // println!("{:?}", map);
                results.push(map);
            }

            let mut len_buf = [0u8; 4];

            unsafe {
                idevice_connection_receive_timeout(
                    socket.connection(),
                    len_buf.as_mut_ptr() as *mut i8,
                    4,
                    &mut recv_bytes,
                    5000,
                )
            };
            reply_len = u32::from_be_bytes(len_buf) as usize;
        }

        Some(results)
    }
}
