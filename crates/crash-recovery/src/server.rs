use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(not(windows))]
use tokio::io::AsyncReadExt;

#[cfg(windows)]
use futures::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(windows)]
use interprocess::reliable_recv_msg::AsyncReliableRecvMsgExt;

use crate::{Error, MessageHeader, MessageKind};

pub struct Server {
    listener: crate::os::Listener,
    #[cfg(target_os = "macos")]
    port: crash_context::ipc::Server,
    socket_path: PathBuf,
}

struct ClientConnection {
    socket: crate::os::Stream,
}

impl Server {
    pub fn bind(path: &Path) -> crate::Result<Self> {
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }

        let listener = crate::os::bind(path)?;

        #[cfg(target_os = "macos")]
        let port = {
            // Note that sun_path is limited to 108 characters including null,
            // while a mach port name is limited to 128 including null, so
            // the length is already effectively checked here
            let port_name = std::ffi::CString::new(path.to_str().ok_or(Error::InvalidPortName)?)
                .map_err(|_err| Error::InvalidPortName)?;
            crash_context::ipc::Server::create(&port_name)?
        };

        Ok(Self {
            listener,
            #[cfg(target_os = "macos")]
            port,
            socket_path: path.to_path_buf(),
        })
    }

    pub async fn run(self) -> crate::Result<()> {
        #[cfg(not(windows))]
        if let Ok((socket, addr)) = self.listener.accept().await {
            let conn = ClientConnection { socket };

            println!("client connected {addr:?}");

            self.handle_connection(conn).await?;
        }

        #[cfg(windows)]
        if let Ok(socket) = self.listener.accept().await {
            let conn = ClientConnection { socket };

            println!("client connected");

            self.handle_connection(conn).await?;
        }

        Ok(())
    }

    async fn handle_connection(mut self, mut conn: ClientConnection) -> crate::Result<()> {
        while let Some((kind, body)) = conn.recv().await {
            println!("got {kind:?} message");

            #[allow(unreachable_patterns)]
            match kind {
                MessageKind::Crash => {
                    println!("received crash message body: {body:?}");
                    self.handle_crash_message(body)?;

                    #[cfg(not(target_os = "macos"))]
                    {
                        let ack = MessageHeader {
                            kind: MessageKind::CrashAck,
                            len: 0,
                        };
                        #[cfg(not(windows))]
                        conn.socket.write_all(ack.as_bytes())?;
                        #[cfg(windows)]
                        {
                            conn.socket.send(ack.as_bytes()).await?;
                            conn.socket.flush().await.unwrap();
                        }
                    }

                    return Ok(());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_crash_message(&mut self, body: Vec<u8>) -> crate::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let crash_context = {
            crash_context::CrashContext::from_bytes(&body).ok_or_else(|| {
                Error::from(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "client sent an incorrectly sized buffer",
                ))
            })?
        };
        #[cfg(target_os = "macos")]
        let crash_context = {
            let Some(mut rcc) = self.port.try_recv_crash_context(None)? else {
                return Ok(());
            };

            if let Err(e) = rcc.acker.send_ack(1, Some(Duration::from_secs(2))) {
                eprintln!("failed to send ack: {}", e);
            }

            rcc.crash_context
        };

        #[cfg(target_os = "windows")]
        let crash_context = {
            let dump_request = crate::os::DumpRequest::from_bytes(&body).unwrap();

            let exception_pointers =
                dump_request.exception_pointers as *const crash_context::EXCEPTION_POINTERS;

            crash_context::CrashContext {
                exception_pointers,
                process_id: dump_request.process_id,
                thread_id: dump_request.thread_id,
                exception_code: dump_request.exception_code,
            }
        };

        todo!("process crash context");

        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Note we don't check for the existence of the path since there
        // appears to be a bug on MacOS and Windows, or at least an oversight
        // in std, where checking the existence of the path always fails
        let _res = fs::remove_file(&self.socket_path);
    }
}

impl ClientConnection {
    pub async fn recv(&mut self) -> Option<(MessageKind, Vec<u8>)> {
        let mut hdr_buf = [0u8; std::mem::size_of::<MessageHeader>()];
        #[cfg(not(windows))]
        let bytes_read = self.socket.read(&mut hdr_buf).await.ok()?;

        #[cfg(windows)]
        let bytes_read = self.socket.recv(&mut hdr_buf).await.ok()?.size();

        println!("read bytes {bytes_read}");

        if bytes_read == 0 {
            return None;
        }

        let header = MessageHeader::from_bytes(&hdr_buf)?;

        println!("read header {header:?}");

        if header.len == 0 {
            Some((header.kind, Vec::new()))
        } else {
            #[cfg(not(windows))]
            {
                let mut buf = Vec::with_capacity(header.len);

                let bytes_read = self.socket.read_buf(&mut buf).await.unwrap();
                assert_eq!(bytes_read, header.len);

                return Some((header.kind, buf));
            }
            #[cfg(windows)]
            {
                // init the vec with zeros because an empty vec will cause the read to not resolve.
                // might as well fill it with $header.len entries so that recv won't alloc a new vec.
                let mut buf = vec![0u8; header.len];

                let recv_res = self.socket.recv(&mut buf).await.ok()?;
                assert!(recv_res.fit());
                assert_eq!(recv_res.size(), header.len);

                return Some((header.kind, buf[..header.len].to_vec()));
            }
        }
    }
}
