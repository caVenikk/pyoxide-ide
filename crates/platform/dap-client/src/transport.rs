//! How the client reaches an adapter: an [`Outlet`] for the frames it
//! writes and an [`Inlet`] for what comes back. Standard streams of an
//! adapter process ([`Transport::stdio`]) or a TCP connection to an
//! adapter that listens ([`Transport::connect`]: the child session of a
//! subprocess).

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::Duration;

use process::{CommandLine, ExitStatus, ProcessEvent, ProcessHandle};

/// What the adapter's side produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// Bytes of the message stream, in any chunks.
    Data(Vec<u8>),
    /// A chunk of the adapter's standard error (its log).
    Stderr(Vec<u8>),
    /// The stream ended; the exit status when it was a process. The last
    /// item.
    Closed(Option<ExitStatus>),
}

/// Where the client's frames go. Writes queue and return at once.
pub trait Outlet: Send + 'static {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// No more input: the adapter reads end of file.
    fn close(&mut self);
}

/// What the adapter sends, read on the client's reader thread.
pub trait Inlet: Send + 'static {
    /// Blocks until something comes; after [`Incoming::Closed`] it is not
    /// called again.
    fn recv(&mut self) -> Incoming;
}

/// A connection to one adapter.
pub struct Transport {
    pub(crate) outlet: Box<dyn Outlet>,
    pub(crate) inlet: Box<dyn Inlet>,
    pub(crate) process: Option<ProcessHandle>,
}

impl Transport {
    pub fn new(outlet: impl Outlet, inlet: impl Inlet) -> Self {
        Self {
            outlet: Box::new(outlet),
            inlet: Box::new(inlet),
            process: None,
        }
    }

    /// Starts an adapter process and talks to it over its standard input
    /// and output. Its standard error is its log.
    pub fn stdio(command: &CommandLine) -> io::Result<Self> {
        let (handle, events) = process::spawn(command)?;
        Ok(Self {
            outlet: Box::new(ProcessOutlet(handle.clone())),
            inlet: Box::new(ProcessInlet(events)),
            process: Some(handle),
        })
    }

    /// Connects to an adapter that listens at `host`:`port` (the first of
    /// its addresses that answers within `timeout`). Blocking: called off
    /// the UI thread.
    pub fn connect(host: &str, port: u16, timeout: Duration) -> io::Result<Self> {
        let mut last = None;
        for address in (host, port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(stream) => return Self::tcp(stream),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("no address for {host}:{port}"),
            )
        }))
    }

    /// Talks to an adapter over a connected socket.
    pub fn tcp(stream: TcpStream) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        let reader = stream.try_clone()?;
        Ok(Self::new(TcpOutlet(stream), TcpInlet(reader)))
    }

    /// The adapter's process, when it is one.
    pub fn process(&self) -> Option<&ProcessHandle> {
        self.process.as_ref()
    }

    /// A transport in memory and its other end, for tests and adapters in
    /// the same process.
    pub fn memory() -> (Transport, MemoryPeer) {
        let (written_tx, written) = flume::unbounded();
        let (to_client, from_peer) = flume::unbounded();
        let transport = Transport::new(
            MemoryOutlet {
                written: Some(written_tx),
            },
            MemoryInlet(from_peer),
        );
        (transport, MemoryPeer { written, to_client })
    }
}

/// The adapter's end of [`Transport::memory`].
pub struct MemoryPeer {
    /// What the client wrote; disconnected when it closed its output.
    pub written: flume::Receiver<Vec<u8>>,
    /// What the client reads.
    pub to_client: flume::Sender<Incoming>,
}

struct ProcessOutlet(ProcessHandle);

impl Outlet for ProcessOutlet {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_stdin(bytes)
    }

    fn close(&mut self) {
        self.0.close_stdin();
    }
}

struct ProcessInlet(flume::Receiver<ProcessEvent>);

impl Inlet for ProcessInlet {
    fn recv(&mut self) -> Incoming {
        match self.0.recv() {
            Ok(ProcessEvent::Stdout(chunk)) => Incoming::Data(chunk),
            Ok(ProcessEvent::Stderr(chunk)) => Incoming::Stderr(chunk),
            Ok(ProcessEvent::Exited(status)) => Incoming::Closed(Some(status)),
            Err(_) => Incoming::Closed(None),
        }
    }
}

struct TcpOutlet(TcpStream);

impl Outlet for TcpOutlet {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    /// Closes the connection both ways: nothing is sent after the last
    /// message (`disconnect`) and nothing more is waited for, so the reader
    /// ends even when the adapter keeps the socket open.
    fn close(&mut self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

struct TcpInlet(TcpStream);

impl Inlet for TcpInlet {
    fn recv(&mut self) -> Incoming {
        let mut buffer = vec![0; 64 * 1024];
        loop {
            match self.0.read(&mut buffer) {
                Ok(0) => return Incoming::Closed(None),
                Ok(n) => {
                    buffer.truncate(n);
                    return Incoming::Data(buffer);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Incoming::Closed(None),
            }
        }
    }
}

struct MemoryOutlet {
    written: Option<flume::Sender<Vec<u8>>>,
}

impl Outlet for MemoryOutlet {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match &self.written {
            Some(tx) => tx
                .send(bytes.to_vec())
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe)),
            None => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    fn close(&mut self) {
        self.written = None;
    }
}

struct MemoryInlet(flume::Receiver<Incoming>);

impl Inlet for MemoryInlet {
    fn recv(&mut self) -> Incoming {
        self.0.recv().unwrap_or(Incoming::Closed(None))
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn a_socket_carries_both_ways_and_closes() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut transport = Transport::connect("localhost", port, Duration::from_secs(5)).unwrap();
        assert!(transport.process().is_none());
        let (mut adapter, _) = listener.accept().unwrap();

        transport.outlet.write(b"to the adapter").unwrap();
        let mut got = [0; 14];
        adapter.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"to the adapter");

        adapter.write_all(b"to the client").unwrap();
        let mut read = Vec::new();
        while read.len() < 13 {
            match transport.inlet.recv() {
                Incoming::Data(chunk) => read.extend(chunk),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(read, b"to the client");

        // The client closes: the adapter reads the end, the client's
        // reader ends too.
        transport.outlet.close();
        let mut rest = Vec::new();
        adapter.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty());
        assert_eq!(transport.inlet.recv(), Incoming::Closed(None));
    }

    #[test]
    fn the_adapter_closing_ends_the_stream() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut transport = Transport::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        let (adapter, _) = listener.accept().unwrap();
        drop(adapter);
        assert_eq!(transport.inlet.recv(), Incoming::Closed(None));
    }

    #[test]
    fn nothing_listening_is_an_error() {
        let port = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(Transport::connect("127.0.0.1", port, Duration::from_secs(2)).is_err());
    }
}
