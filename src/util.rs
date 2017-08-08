//! The util module rexports some tools from mio in order to facilitate handling timeouts.


/// Used to identify some timed-out event.
pub use mio::Token;
/// A handle to a specific timeout.
pub use mio::timer::Timeout;
use slab;

/// A Slab allocator for associating tokens to data.
pub type Slab<T> = slab::Slab<T, Token>;

pub use mio::tcp::TcpStream;
