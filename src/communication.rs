use io::ALL;
use message;
use mio;
use mio::Token;
use protocol::CloseCode;
use result::{Result, Error};
use std::borrow::Cow;
use std::convert::Into;
use std::net::{SocketAddr, ToSocketAddrs};
use url;


#[derive(Debug, Clone)]
pub enum Signal {
    Message(message::Message),
    Close(CloseCode, Cow<'static, str>),
    Connect(String),
    Shutdown,
    Timeout { delay: u64, token: Token },
    Cancel(mio::timer::Timeout),
}

#[derive(Debug, Clone)]
pub struct Command {
    token: Token,
    signal: Signal,
    connection_id: u32,
}

impl Command {
    pub fn token(&self) -> Token {
        self.token
    }

    pub fn signal(self) -> Signal {
        self.signal
    }

    pub fn connection_id(&self) -> u32 {
        self.connection_id
    }
}


#[derive(Clone)]
pub struct Sender {
    token: Token,
    channel: mio::channel::SyncSender<Command>,
    connection_id: u32,
}

impl Sender {
    pub fn new(token: Token, channel: mio::channel::SyncSender<Command>, connection_id: u32) -> Sender {
        Sender {
            token: token,
            channel: channel,
            connection_id: connection_id,
        }
    }


    pub fn token(&self) -> Token {
        self.token
    }


    pub fn send<M>(&self, msg: M) -> Result<()>
    where
        M: Into<message::Message>,
    {
        self.channel
            .send(Command {
                      token: self.token,
                      signal: Signal::Message(msg.into()),
                      connection_id: self.connection_id,
                  })
            .map_err(Error::from)
    }


    pub fn broadcast<M>(&self, msg: M) -> Result<()>
    where
        M: Into<message::Message>,
    {
        self.channel
            .send(Command {
                      token: ALL,
                      signal: Signal::Message(msg.into()),
                      connection_id: self.connection_id,
                  })
            .map_err(Error::from)
    }

    /// Send a close code to the other endpoint.
    #[inline]
    pub fn close(&self, code: CloseCode) -> Result<()> {
        self.channel
            .send(Command {
                      token: self.token,
                      signal: Signal::Close(code, "".into()),
                      connection_id: self.connection_id,
                  })
            .map_err(Error::from)
    }

    /// Send a close code and provide a descriptive reason for closing.
    #[inline]
    pub fn close_with_reason<S>(&self, code: CloseCode, reason: S) -> Result<()>
    where
        S: Into<Cow<'static, str>>,
    {
        self.channel
            .send(Command {
                      token: self.token,
                      signal: Signal::Close(code, reason.into()),
                      connection_id: self.connection_id,
                  })
            .map_err(Error::from)
    }


    #[inline]
    pub fn connect(&self, url: String) -> Result<()> {
        self.channel
            .send(Command {
                      token: self.token,
                      signal: Signal::Connect(url),
                      connection_id: self.connection_id,
                  })
            .map_err(Error::from)
    }


    #[inline]
    pub fn shutdown(&self) -> Result<()> {
        self.channel
            .send(Command {
                      token: self.token,
                      signal: Signal::Shutdown,
                      connection_id: self.connection_id,
                  })
            .map_err(Error::from)
    }

    /// Schedule a `token` to be sent to the Socket Handler's `on_timeout` method
    /// after `ms` milliseconds
    #[inline]
    pub fn timeout(&self, ms: u64, token: Token) -> Result<()> {
        self.channel
            .send(Command {
                      token: self.token,
                      signal: Signal::Timeout { delay: ms, token: token },
                      connection_id: self.connection_id,
                  })
            .map_err(Error::from)
    }


    #[inline]
    pub fn cancel(&self, timeout: mio::timer::Timeout) -> Result<()> {
        self.channel
            .send(Command {
                      token: self.token,
                      signal: Signal::Cancel(timeout),
                      connection_id: self.connection_id,
                  })
            .map_err(Error::from)
    }
}
