#![allow(deprecated)]
#![deny(
missing_copy_implementations,
trivial_casts, trivial_numeric_casts,
unstable_features,
unused_import_braces)]

extern crate httparse;
extern crate mio;
extern crate sha1;
extern crate rand;
extern crate url;
extern crate slab;
extern crate bytes;
extern crate byteorder;
#[macro_use]
extern crate log;

mod result;
mod connection;
mod handler;
mod factory;
mod message;
mod protocol;
mod communication;
mod io;
mod stream;
pub mod util;
pub use communication::Sender;
pub use factory::Factory;
pub use handler::Handler;
pub use message::Message;

use mio::Poll;
pub use protocol::{CloseCode, OpCode};
pub use result::{Result, Error};
pub use result::Kind as ErrorKind;
use std::borrow::Borrow;
use std::default::Default;
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};


pub fn listen<A, F, H>(addr: A, factory: F) -> Result<()>
where
    A: ToSocketAddrs + fmt::Debug,
    F: FnMut(Sender) -> H,
    H: Handler,
{
    let ws = XnetSocket::new(factory)?;
    ws.listen(addr)?;
    Ok(())
}


pub fn connect<F, H>(url: String, factory: F) -> Result<()>
where
    F: FnMut(Sender) -> H,
    H: Handler,
{
    let mut ws = XnetSocket::new(factory)?;
    //    let parsed =
    //        url::Url::parse(url.borrow())
    //            .map_err(|err| Error::new(ErrorKind::Internal, format!("Unable to parse {} as url due to {:?}", url.borrow(), err)))?;
    //    trace!("----{:?}---", url.borrow());
    ws.connect(url)?;
    ws.run()?;
    Ok(())
}

/// Socket settings
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Default: 100
    pub max_connections: usize,

    /// Default: 5
    pub queue_size: usize,

    /// Default: false
    pub panic_on_new_connection: bool,

    /// Default: false
    pub panic_on_shutdown: bool,

    /// Default: 10
    pub fragments_capacity: usize,

    /// Default: true
    pub fragments_grow: bool,

    /// Default: 65,535
    pub fragment_size: usize,

    /// Default: 2048
    pub in_buffer_capacity: usize,

    /// Default: true
    pub in_buffer_grow: bool,

    /// Default: 2048
    pub out_buffer_capacity: usize,

    /// Default: true
    pub out_buffer_grow: bool,

    /// Default: true
    pub panic_on_internal: bool,

    /// Default: false
    pub panic_on_capacity: bool,

    /// Default: false
    pub panic_on_protocol: bool,

    /// Default: false
    pub panic_on_encoding: bool,

    /// Default: false
    pub panic_on_queue: bool,

    /// Default: false
    pub panic_on_io: bool,

    /// Default: false
    pub panic_on_timeout: bool,

    /// Default: true
    pub shutdown_on_interrupt: bool,

    /// Default: false
    pub tcp_nodelay: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            max_connections: 100,
            queue_size: 5,
            panic_on_new_connection: false,
            panic_on_shutdown: false,
            fragments_capacity: 10,
            fragments_grow: true,
            fragment_size: u16::max_value() as usize,
            in_buffer_capacity: 2048,
            in_buffer_grow: true,
            out_buffer_capacity: 2048,
            out_buffer_grow: true,
            panic_on_internal: true,
            panic_on_capacity: false,
            panic_on_protocol: false,
            panic_on_encoding: false,
            panic_on_queue: false,
            panic_on_io: false,
            panic_on_timeout: false,
            shutdown_on_interrupt: true,
            tcp_nodelay: false,
        }
    }
}



pub struct XnetSocket<F>
where
    F: Factory,
{
    poll: Poll,
    handler: io::Handler<F>,
}

impl<F> XnetSocket<F>
where
    F: Factory,
{
    pub fn new(factory: F) -> Result<XnetSocket<F>> {
        Builder::new().build(factory)
    }


    pub fn bind<A>(mut self, addr_spec: A) -> Result<XnetSocket<F>>
    where
        A: ToSocketAddrs,
    {
        let mut last_error = Error::new(ErrorKind::Internal, "No address given");

        for addr in addr_spec.to_socket_addrs()? {
            if let Err(e) = self.handler.listen(&mut self.poll, &addr) {
                error!("Unable to listen on {}", addr);
                last_error = e;
            } else {
                let actual_addr = self.handler.local_addr().unwrap_or(addr);
                info!("Listening for new connections on {}.", actual_addr);
                return Ok(self);
            }
        }

        Err(last_error)
    }


    pub fn listen<A>(self, addr_spec: A) -> Result<XnetSocket<F>>
    where
        A: ToSocketAddrs,
    {
        self.bind(addr_spec).and_then(|server| server.run())
    }



    pub fn connect(&mut self, addr_spec: String) -> Result<&mut XnetSocket<F>> {
        let sender = self.handler.sender();
        info!("Queuing connection to {}", addr_spec);
        sender.connect(addr_spec)?;
        Ok(self)
    }

    pub fn run(mut self) -> Result<XnetSocket<F>> {
        self.handler.run(&mut self.poll)?;
        Ok(self)
    }


    /// as server use
    pub fn broadcaster(&self) -> Sender {
        self.handler.sender()
    }


    pub fn local_addr(&self) -> ::std::io::Result<SocketAddr> {
        self.handler.local_addr()
    }
}


#[derive(Debug, Clone, Copy)]
pub struct Builder {
    settings: Settings,
}

// TODO: add convenience methods for each setting
impl Builder {
    pub fn new() -> Builder {
        Builder { settings: Settings::default() }
    }


    pub fn build<F>(&self, factory: F) -> Result<XnetSocket<F>>
    where
        F: Factory,
    {
        Ok(XnetSocket {
               poll: Poll::new()?,
               handler: io::Handler::new(factory, self.settings),
           })
    }


    pub fn with_settings(&mut self, settings: Settings) -> &mut Builder {
        self.settings = settings;
        self
    }
}
