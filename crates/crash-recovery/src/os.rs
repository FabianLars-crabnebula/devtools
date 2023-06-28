use std::path::Path;

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        pub type Stream = tokio::net::UnixStream;
        pub type Listener = tokio::net::UnixListener;

        pub async fn connect(path: &Path) -> crate::Result<Stream> {
            // let socket = std::os::unix::net::UnixStream::connect(path)?;
            // socket.set_nonblocking(true)?;
            let socket = Stream::connect(path).await?;
            Ok(socket)
        }

        pub fn bind(path: &Path) -> crate::Result<Listener> {
            Listener::bind(path).map_err(Into::into)
        }
    } else if #[cfg(target_os = "windows")] {
        use std::{mem, slice};
        use interprocess::os::windows::named_pipe::{PipeListenerOptions, pipe_mode,PipeMode, tokio::{DuplexPipeStream, PipeListenerOptionsExt, PipeListener}};

        pub type Stream = DuplexPipeStream<pipe_mode::Messages>; // must implement `io::Read + io::Write`
        pub type Listener = PipeListener<pipe_mode::Messages, pipe_mode::Messages>; // must implement an `accept()` method that returns Result<(Stream, Address)> like the unix counterpart

        pub async fn connect(path: &Path) -> crate::Result<Stream> {
            let socket = DuplexPipeStream::<pipe_mode::Messages>::connect(path).await?;
            Ok(socket)
        }

        pub fn bind(path: &Path) -> crate::Result<Listener> {
            let listener = PipeListenerOptions::new()
                .name(path.as_os_str())
                .mode(PipeMode::Messages)
                .create_tokio_duplex::<pipe_mode::Messages>()?;

            Ok(listener)
        }

        #[repr(C)]
        pub struct DumpRequest {
            /// The address of an `EXCEPTION_POINTERS` in the client's memory
            pub exception_pointers: usize,
            /// The process id of the client process
            pub process_id: u32,
            /// The id of the thread in the client process in which the crash originated
            pub thread_id: u32,
            /// The top level exception code, also found in the `EXCEPTION_POINTERS.ExceptionRecord.ExceptionCode`
            pub exception_code: i32,
        }

        impl DumpRequest {
            pub fn as_bytes(&self) -> &[u8] {
                #[allow(unsafe_code)]
                unsafe {
                    let size = mem::size_of::<Self>();
                    let ptr = (self as *const Self).cast();
                    slice::from_raw_parts(ptr, size)
                }
            }

            pub fn from_bytes(buf: &[u8]) -> Option<&Self> {
                if buf.len() != mem::size_of::<Self>() {
                    return None;
                }

                #[allow(unsafe_code)]
                unsafe {
                    let (_head, body, _tail) = buf.align_to::<Self>();

                    Some(&body[0])
                }
            }
        }
    } else {
        compile_error!("unsupported target platform")
    }
}
