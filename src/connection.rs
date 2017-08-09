use self::Endpoint::*;

use self::State::*;

use super::Settings;
use handler::Handler;

use message::Message;
use mio::{Token, Ready};
use mio::tcp::TcpStream;
use mio::timer::Timeout;
use protocol::{CloseCode, OpCode};
use result::{Result, Error, Kind};
use std::borrow::Borrow;
use std::collections::VecDeque;
use std::io::{Write, Read, Cursor, Seek, SeekFrom};
use std::mem::replace;
use std::net::SocketAddr;
use std::str::from_utf8;
use stream::{Stream, TryReadBuf, TryWriteBuf};

use url;

#[derive(Debug)]
pub enum State {
	// Tcp connection accepted, waiting for handshake to complete
	Connecting(Cursor<Vec<u8>>, Cursor<Vec<u8>>),
	// Ready to send/receive messages
	Open,
	AwaitingClose,
	RespondingClose,
	FinishedClose,
}

/// A little more semantic than a boolean
#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Endpoint {
	/// Will mask outgoing frames
	Client(String),
	/// Won't mask outgoing frames
	Server,
}

impl State {
	#[inline]
	pub fn is_connecting(&self) -> bool {
		match *self {
			State::Connecting(..) => true,
			_ => false,
		}
	}

	#[allow(dead_code)]
	#[inline]
	pub fn is_open(&self) -> bool {
		match *self {
			State::Open => true,
			_ => false,
		}
	}

	#[inline]
	pub fn is_closing(&self) -> bool {
		match *self {
			State::AwaitingClose => true,
			State::FinishedClose => true,
			_ => false,
		}
	}
}

pub struct Connection<H>
where
	H: Handler,
{
	token: Token,
	socket: Stream,
	//当前连接的活动状态。
	state: State,
	//对端的信息
	endpoint: Endpoint,
	events: Ready,
	in_buffer: Cursor<Vec<u8>>,
	out_buffer: Cursor<Vec<u8>>,
	//这个是重要的，不同的协议需要实现不同的Handler
	handler: H,
	//连接的对端地址。
	addresses: Vec<SocketAddr>,
	//配置情况
	settings: Settings,
	//连接id,可能会出现同一个socket，不同id的情况。
	connection_id: u32,
}

impl<H> Connection<H>
where
	H: Handler,
{
	pub fn new(tok: Token, sock: TcpStream, handler: H, settings: Settings, connection_id: u32) -> Connection<H> {
		Connection {
			token: tok,
			socket: Stream::tcp(sock),
			state: Connecting(Cursor::new(Vec::with_capacity(2048)), Cursor::new(Vec::with_capacity(2048))),
			endpoint: Endpoint::Server,
			events: Ready::empty(),
			in_buffer: Cursor::new(Vec::with_capacity(settings.in_buffer_capacity)),
			out_buffer: Cursor::new(Vec::with_capacity(settings.out_buffer_capacity)),
			handler: handler,
			addresses: Vec::new(),
			settings: settings,
			connection_id: connection_id,
		}
	}

	//socket successed callback the function
	pub fn open(&mut self) -> Result<()> {
		trace!("accept socket{:?}", self.token);
		if let Connecting(ref req, ref res) = replace(&mut self.state, Open) {
			trace!("accept new socket change state connecting  to open {}", self.peer_addr());
			return Ok(());
		} else {
			Err(Error::new(Kind::Internal, "Tried to write socket while not in connecting state!"))
		}
	}

	pub fn as_server(&mut self) -> Result<()> {
		trace!("new server socket half ");
		Ok(self.events.insert(Ready::readable()))
	}

	pub fn as_client(&mut self, url: String, addrs: Vec<SocketAddr>) -> Result<()> {
		trace!("new client socket half ");
		match self.state {
			State::Open => {
				self.addresses = addrs;
				self.events.insert(Ready::readable()); //监听可读
				self.endpoint = Endpoint::Client(url);
				Ok(())
			}

			_ => {
				Err(Error::new(Kind::Internal, "Tried to set connection to client while not connecting."))
			}
		}
	}

	pub fn token(&self) -> Token {
		self.token
	}

	pub fn socket(&self) -> &TcpStream {
		self.socket.evented()
	}

	pub fn connection_id(&self) -> u32 {
		self.connection_id
	}

	fn peer_addr(&self) -> String {
		if let Ok(addr) = self.socket.peer_addr() { addr.to_string() } else { "UNKNOWN".into() }
	}


	//TODO
	pub fn reset(&mut self) -> Result<()> {

		if self.is_client() {
			if let Connecting(ref mut req, ref mut res) = self.state {
				req.set_position(0);
				res.set_position(0);
				self.events.remove(Ready::writable());
				self.events.insert(Ready::readable());
				if let Some(ref addr) = self.addresses.pop() {
					let sock = try!(TcpStream::connect(addr));
					Ok(self.socket = Stream::tcp(sock))
				} else {
					if self.settings.panic_on_new_connection {
						panic!("Unable to connect to server.");
					}
					Err(Error::new(Kind::Internal, "Exhausted possible addresses."))
				}
			} else {
				Err(Error::new(Kind::Internal, "Unable to reset client connection because it is active."))
			}
		} else {
			Err(Error::new(Kind::Internal, "Server connections cannot be reset."))
		}
	}

	pub fn events(&self) -> Ready {
		self.events
	}

	pub fn is_client(&self) -> bool {
		match self.endpoint {
			Client(_) => true,
			Server => false,
		}
	}

	pub fn is_server(&self) -> bool {
		match self.endpoint {
			Client(_) => false,
			Server => true,
		}
	}

	pub fn shutdown(&mut self) {
		self.handler.on_shutdown();
		if let Err(err) = self.send_close(CloseCode::Away, "Shutting down.") {
			self.handler.on_error(err);
			self.disconnect()
		}
	}

	#[inline]
	pub fn new_timeout(&mut self, event: Token, timeout: Timeout) -> Result<()> {
		self.handler.on_new_timeout(event, timeout)
	}

	#[inline]
	pub fn timeout_triggered(&mut self, event: Token) -> Result<()> {
		self.handler.on_timeout(event)
	}

	pub fn error(&mut self, err: Error) {
		match self.state {
			Connecting(_, ref mut res) => {
				match err.kind {
					Kind::Protocol => {
						let msg = err.to_string();
						self.handler.on_error(err);
						if let Server = self.endpoint {
							res.get_mut().clear();
							if let Err(err) = write!(res.get_mut(), "Bad Request\r\n\r\n{}", msg) {
								self.handler.on_error(Error::from(err));
								self.events = Ready::empty();
							} else {
								self.events.remove(Ready::readable());
								self.events.insert(Ready::writable());
							}
						} else {
							self.events = Ready::empty();
						}
					}
					_ => {
						let msg = err.to_string();
						self.handler.on_error(err);
						if let Server = self.endpoint {
							res.get_mut().clear();
							if let Err(err) = write!(res.get_mut(), "Internal Server Error\r\n\r\n{}", msg) {
								self.handler.on_error(Error::from(err));
								self.events = Ready::empty();
							} else {
								self.events.remove(Ready::readable());
								self.events.insert(Ready::writable());
							}
						} else {
							self.events = Ready::empty();
						}
					}
				}
			}
			_ => {
				match err.kind {
					Kind::Internal => {
						if self.settings.panic_on_internal {
							panic!("Panicking on internal error -- {}", err);
						}
						let reason = format!("{}", err);

						self.handler.on_error(err);
						if let Err(err) = self.send_close(CloseCode::Error, reason) {
							self.handler.on_error(err);
							self.disconnect()
						}
					}
					Kind::Capacity => {
						if self.settings.panic_on_capacity {
							panic!("Panicking on capacity error -- {}", err);
						}
						let reason = format!("{}", err);

						self.handler.on_error(err);
						if let Err(err) = self.send_close(CloseCode::Size, reason) {
							self.handler.on_error(err);
							self.disconnect()
						}
					}
					Kind::Protocol => {
						if self.settings.panic_on_protocol {
							panic!("Panicking on protocol error -- {}", err);
						}
						let reason = format!("{}", err);

						self.handler.on_error(err);
						if let Err(err) = self.send_close(CloseCode::Protocol, reason) {
							self.handler.on_error(err);
							self.disconnect()
						}
					}
					Kind::Encoding(_) => {
						if self.settings.panic_on_encoding {
							panic!("Panicking on encoding error -- {}", err);
						}
						let reason = format!("{}", err);

						self.handler.on_error(err);
						if let Err(err) = self.send_close(CloseCode::Invalid, reason) {
							self.handler.on_error(err);
							self.disconnect()
						}
					}
					Kind::Http(_) => {
						// This may happen if some handler writes a bad response
						self.handler.on_error(err);
						error!("Disconnecting socket.");
						self.disconnect()
					}
					Kind::Custom(_) => {
						self.handler.on_error(err);
					}
					Kind::Timer(_) => {
						if self.settings.panic_on_timeout {
							panic!("Panicking on timer failure -- {}", err);
						}
						self.handler.on_error(err);
					}
					Kind::Queue(_) => {
						if self.settings.panic_on_queue {
							panic!("Panicking on queue error -- {}", err);
						}
						self.handler.on_error(err);
					}
					_ => {
						if self.settings.panic_on_io {
							panic!("Panicking on io error  {}", err);
						}
						self.handler.on_error(err);
						self.disconnect()
					}
				}
			}
		}
	}

	pub fn disconnect(&mut self) {
		match self.state {
			RespondingClose | FinishedClose | Connecting(_, _) => (),
			_ => {
				self.handler.on_close(CloseCode::Abnormal, "");
			}
		}
		self.events = Ready::empty()
	}

	///
	pub fn consume(self) -> H {
		self.handler
	}


	pub fn read(&mut self) -> Result<()> {
		if self.socket.is_negotiating() {
			trace!("Performing TLS negotiation on {}.", self.peer_addr());
			self.socket.clear_negotiating()?;
			Ok(())
		} else {
			if self.state.is_connecting() {
				trace!("connect state not change {}.", self.peer_addr());
				Err(Error::new(Kind::Internal, "connect state not change"))
			} else {
				trace!("Ready to read messages from {}.", self.peer_addr());
				while let Some(len) = self.buffer_in()? {
					trace!("read data {}", len);
					self.read_data()?; //read data in in_buffer
					if len == 0 {
						if self.events.is_writable() {
							self.events.remove(Ready::readable());
						} else {
							self.disconnect()
						}
						break;
					}
				}
				Ok(())
			}
		}
	}

	fn read_data(&mut self) -> Result<()> {
		//读取数据。
		let mut buffer = Vec::with_capacity(self.in_buffer.get_ref().len());
		match self.in_buffer.read_to_end(&mut buffer) {
			Ok(data_size) => {
				let msg = Message::text((String::from_utf8(buffer).map_err(|err| err.utf8_error()))?);
				self.handler.on_message(msg)?;
				Ok(())
			}
			Err(err) => Err(Error::from(err)),
		}
	}

	pub fn write(&mut self) -> Result<()> {
		if self.socket.is_negotiating() {
			trace!("Performing TLS negotiation on {}.", self.peer_addr());
			self.socket.clear_negotiating()
		} else {
			let res = if self.state.is_connecting() {
				trace!("connect state not change {}.", self.peer_addr());
				Err(Error::new(Kind::Internal, "connect state not change"))
			} else {
				trace!("Ready to write messages to {}.", self.peer_addr());

				// Start out assuming that this write will clear the whole buffer
				self.events.remove(Ready::writable());
				trace!("postions {:?}", self.out_buffer.position());
				if let Some(len) = try!(self.socket.try_write_buf(&mut self.out_buffer)) {
					trace!("Wrote {} bytes to {}", len, self.peer_addr());
					if len == 0 {
						match self.state {
							// we are are a server that is closing and just wrote out our confirming
							// close frame, let's disconnect
							FinishedClose if self.is_server() => return Ok(self.events = Ready::empty()),
							_ => (),
						}
					}
				}

				// Check if there is more to write so that the connection will be rescheduled
				Ok(self.check_events())
			};

			if self.socket.is_negotiating() && res.is_ok() {
				self.events.remove(Ready::writable());
				self.events.insert(Ready::readable());
			}
			res
		}
	}

	pub fn send_message(&mut self, msg: Message) -> Result<()> {
		if self.state.is_closing() {
			trace!("Connection is closing. Ignoring request to send message {:?} to {}.", msg, self.peer_addr());
			return Ok(());
		}

		let opcode = msg.opcode();
		trace!("Message opcode {:?}", opcode);
		let data = msg.into_data();

		self.check_buffer_out(&data)?;
		trace!("Buffering frame to {} : {:?}", self.peer_addr(), data);
		//TODO 写数据。
		let pos = self.out_buffer.position();
		self.out_buffer.seek(SeekFrom::End(0))?;
		match self.out_buffer.write(&data) {
			Ok(buffer_size) => {
				//TODO
				self.out_buffer.seek(SeekFrom::Start(pos))?;
				Ok(self.check_events())
			}
			Err(err) => Err(Error::from(err)),
		}
	}


	#[inline]
	pub fn send_close<R>(&mut self, code: CloseCode, reason: R) -> Result<()>
	where
		R: Borrow<str>,
	{
		match self.state {
			// We are responding to a close frame the other endpoint, when this frame goes out, we
			// are done.
			RespondingClose => self.state = FinishedClose,
			// Multiple close frames are being sent from our end, ignore the later frames
			AwaitingClose | FinishedClose => {
				trace!("Connection is already closing. Ignoring close {:?} -- {:?} to {}.", code, reason.borrow(), self.peer_addr());
				return Ok(self.check_events());
			}
			// We are initiating a closing handshake.
			Open => self.state = AwaitingClose,
			Connecting(_, _) => {
				debug_assert!(false, "Attempted to close connection while not yet open.")
			}
		}

		trace!("Sending close {:?} -- {:?} to {}.", code, reason.borrow(), self.peer_addr());

		//TODO
		//        if let Some(frame) = try!(self.handler.buffer_frame(Frame::close(code, reason.borrow()))) {
		//            try!(self.buffer_frame(frame));
		//        }

		trace!("Connection to {} is now closing.", self.peer_addr());

		Ok(self.check_events())
	}
	
	fn check_events(&mut self) {
		if !self.state.is_connecting() {
			self.events.insert(Ready::readable());
			if self.out_buffer.position() < self.out_buffer.get_ref().len() as u64 {
				trace!("check_event{:?}- {:?}-", self.out_buffer.get_ref().len(), self.out_buffer.get_ref());
				self.events.insert(Ready::writable());
			}
		}
	}


	fn check_buffer_out(&mut self, frame: &Vec<u8>) -> Result<()> {
		if self.out_buffer.get_ref().capacity() <= self.out_buffer.get_ref().len() + frame.len() {
			// extend
			let mut new = Vec::with_capacity(self.out_buffer.get_ref().capacity());
			new.extend(&self.out_buffer.get_ref()[self.out_buffer.position() as usize..]);
			if new.len() == new.capacity() {
				if self.settings.out_buffer_grow {
					new.reserve(self.settings.out_buffer_capacity)
				} else {
					return Err(Error::new(Kind::Capacity, "Maxed out output buffer for connection."));
				}
			}
			self.out_buffer = Cursor::new(new);
		}
		Ok(())
	}

	fn buffer_in(&mut self) -> Result<Option<usize>> {
		//input buffer
		trace!("Reading buffer for connection to {}.", self.peer_addr());
		if let Some(len) = self.socket.try_read_buf(self.in_buffer.get_mut())? {
			trace!("try read buffer len {:?}, data {:?}", len, self.in_buffer.get_ref());
			if self.in_buffer.get_ref().len() == self.in_buffer.get_ref().capacity() {
				// extend
				let mut new = Vec::with_capacity(self.in_buffer.get_ref().capacity());
				new.extend(&self.in_buffer.get_ref()[self.in_buffer.position() as usize..]);

				if new.len() == new.capacity() {
					if self.settings.in_buffer_grow {
						new.reserve(self.settings.in_buffer_capacity);
					} else {
						return Err(Error::new(Kind::Capacity, "Maxed out input buffer for connection."));
					}
				}
				self.in_buffer = Cursor::new(new);
			}
			Ok(Some(len))
		} else {
			Ok(None)
		}
	}
}
