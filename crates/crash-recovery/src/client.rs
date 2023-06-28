use crate::{Error, MessageHeader, MessageKind};
use std::{io::IoSlice, mem, path::Path, time::Duration};

#[cfg(windows)]
use interprocess::reliable_recv_msg::AsyncReliableRecvMsgExt;

pub struct Client {
    socket: crate::os::Stream,
    #[cfg(target_os = "macos")]
    port: crash_context::ipc::Client,
}

impl Client {
    pub fn connect(path: &Path) -> crate::Result<Self> {
        let socket = crate::os::connect(path)?;

        #[cfg(target_os = "macos")]
        let port = {
            // Note that sun_path is limited to 108 characters including null,
            // while a mach port name is limited to 128 including null, so
            // the length is already effectively checked here
            let port_name = std::ffi::CString::new(path.to_str().ok_or(Error::InvalidPortName)?)
                .map_err(|_err| Error::InvalidPortName)?;
            crash_context::ipc::Client::create(&port_name)?
        };

        Ok(Self {
            socket,
            #[cfg(target_os = "macos")]
            port,
        })
    }

    pub async fn send_crash_context(
        &mut self,
        ctx: &crash_context::CrashContext,
    ) -> crate::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let crash_ctx_buf = ctx.as_bytes();
        #[cfg(target_os = "macos")]
        let crash_ctx_buf = {
            self.port.send_crash_context(
                ctx,
                Some(Duration::from_secs(2)),
                Some(Duration::from_secs(5)),
            )?;

            &std::process::id().to_ne_bytes()
        };
        #[cfg(windows)]
        let req = crate::os::DumpRequest {
            exception_pointers: ctx.exception_pointers as usize,
            process_id: ctx.process_id,
            thread_id: ctx.thread_id,
            exception_code: ctx.exception_code,
        };
        #[cfg(windows)]
        let crash_ctx_buf = req.as_bytes();

        self.send_impl(MessageKind::Crash, &crash_ctx_buf).await?;

        #[cfg(not(target_os = "macos"))]
        {
            /* let mut ack = [0u8; std::mem::size_of::<MessageHeader>()];
            self.socket.recv(&mut ack).await?; */
            use tokio::io::AsyncReadExt;

            let mut ack = [0u8; std::mem::size_of::<MessageHeader>()];
            self.socket.try_read(&mut ack)?;

            let header = MessageHeader::from_bytes(&ack);

            if header
                .filter(|hdr| hdr.kind == MessageKind::CrashAck)
                .is_none()
            {
                return Err(Error::ProtocolError("received invalid response to crash"));
            }
        }

        Ok(())
    }

    async fn send_impl(&mut self, kind: MessageKind, buf: &[u8]) -> crate::Result<()> {
        println!("sending message {kind:?} with buf {buf:?}");
        let header = MessageHeader {
            kind,
            len: buf.len(),
        };

        let hdr_buf = header.as_bytes();

        #[cfg(not(windows))]
        {
            let bytes_written = self
                .socket
                .send_vectored(&[IoSlice::new(hdr_buf), IoSlice::new(buf)])?;

            assert_eq!(bytes_written, buf.len() + mem::size_of::<MessageHeader>());
        }

        #[cfg(windows)]
        {
            let res1 = self.socket.send(hdr_buf).await;
            println!("send res header {res:?}");
            self.socket.flush().await.unwrap();

            let res2 = self.socket.send(buf).await;
            println!("send res body {res:?}");
            self.socket.flush().await.unwrap();

            assert_eq!(res1 + res2, buf.len() + mem::size_of::<MessageHeader>());
        };

        Ok(())
    }
}
