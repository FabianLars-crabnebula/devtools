use std::{io::IoSlice, path::Path, time::Duration};

use tokio::io::Interest;

use crate::{os, Error, MessageHeader, MessageKind};

pub struct Client {
    #[cfg(windows)]
    socket: crate::os::ClientStream,
    #[cfg(not(windows))]
    socket: crate::os::Stream,
    #[cfg(target_os = "macos")]
    port: crash_context::ipc::Client,
}

impl Client {
    pub async fn connect(path: &Path) -> crate::Result<Self> {
        let socket = crate::os::connect(path).await?;
        /* #[cfg(windows)]
        socket
            .ready(Interest::READABLE | Interest::WRITABLE)
            .await?; */

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
        #[cfg(target_os = "windows")]
        let req = os::DumpRequest {
            exception_pointers: ctx.exception_pointers as usize,
            process_id: ctx.process_id,
            thread_id: ctx.thread_id,
            exception_code: ctx.exception_code,
        };
        #[cfg(target_os = "windows")]
        let crash_ctx_buf = req.as_bytes();

        println!("crash_ctx_buf {crash_ctx_buf:?}");

        self.send_impl(MessageKind::Crash, crash_ctx_buf).await?;

        #[cfg(not(target_os = "macos"))]
        {
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

        println!(
            "trying to send header {hdr_buf:?} with length: {}",
            hdr_buf.len()
        );

        self.socket.writable().await?;
        let res = self.socket.try_write(hdr_buf);
        self.socket.writable().await?;
        let res = self.socket.try_write(buf);

        println!("send res {res:?}");

        Ok(())
    }
}
