use std::path::Path;

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        pub type Stream = tokio::net::UnixStream;
        pub type Listener = tokio::net::UnixListener;

        pub async fn connect(path: &Path) -> crate::Result<ClientStream> {
            // let socket = std::os::unix::net::UnixStream::connect(path)?;
            // socket.set_nonblocking(true)?;
            let socket = ClientStream::connect(path).await?;
            Ok(socket)
        }

        pub fn bind(path: &Path) -> crate::Result<Listener> {
            Listener::bind(path).map_err(Into::into)
        }
    } else if #[cfg(target_os = "windows")] {
        use std::{mem, slice};
        use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions, NamedPipeServer, NamedPipeClient, PipeMode};

        // TODO @fabianlars give these proper types
        pub type ClientStream = NamedPipeClient; // must implement `io::Read + io::Write` -> Edit Fabian-Lars: I assume this will be AsyncRead and AsyncWrite for the tokio variant
        pub type Stream = NamedPipeServer;
        pub type Listener = NamedPipeServer; // must implement an `accept()` method that returns Result<(Stream, Address)> like the unix counterpart

        pub async fn connect(path: &Path) -> crate::Result<ClientStream> {
            let client = ClientOptions::new().pipe_mode(PipeMode::Message).open(path)?;
            Ok(client)
        }

        pub fn bind(path: &Path) -> crate::Result<Listener> {
            let server = ServerOptions::new().first_pipe_instance(true).pipe_mode(PipeMode::Message).create(path)?;

            Ok(server)
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
