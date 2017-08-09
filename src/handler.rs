use log::LogLevel::Error as ErrorLevel;

use message::Message;
use protocol::CloseCode;
use result::{Result, Error, Kind};
use url;
use util::{Token, Timeout};

//callback trait
pub trait Handler {
	
	/// Called when a request to shutdown all connections has been received.
	#[inline]
	fn on_shutdown(&mut self) {
		debug!("Handler received Socket shutdown request.");
	}

	// Socket events
	fn on_open(&mut self) -> Result<()> {
		Ok(())
	}

	/// Called on incoming messages.
	fn on_message(&mut self, msg: Message) -> Result<()> {
		debug!("Received message {:?}", msg);
		Ok(())
	}


	fn on_close(&mut self, code: CloseCode, reason: &str) {
		debug!("Connection closing due to ({:?}) {}", code, reason);
	}

	/// Called when an error occurs on the Socket.
	fn on_error(&mut self, err: Error) {
		if let Kind::Io(ref err) = err.kind {
			if let Some(104) = err.raw_os_error() {
				return;
			}
		}

		error!("{:?}", err);
		if !log_enabled!(ErrorLevel) {
			println!("Encountered an error: {}\nEnable a logger to see more information.", err);
		}
	}


	// timeout events
	#[inline]
	fn on_timeout(&mut self, event: Token) -> Result<()> {
		debug!("Handler received timeout token: {:?}", event);
		Ok(())
	}


	#[inline]
	fn on_new_timeout(&mut self, _: Token, _: Timeout) -> Result<()> {
		Ok(())
	}
}

impl<F> Handler for F
where
	F: Fn(Message) -> Result<()>,
{
	fn on_message(&mut self, msg: Message) -> Result<()> {
		trace!("Fn on_message {:?}", msg);
		self(msg)
	}
}

mod test {
    #![allow(unused_imports, unused_variables, dead_code)]

	use super::*;
	use message;
	use mio;
	use protocol::CloseCode;
	use result::Result;
	use url;

	#[derive(Debug, Eq, PartialEq)]
	struct M;

	impl Handler for M {
		fn on_message(&mut self, _: message::Message) -> Result<()> {
			Ok(println!("test"))
		}
	}

	#[test]
	fn handler() {
		struct H;

		impl Handler for H {
			fn on_open(&mut self) -> Result<()> {
				Ok(())
			}

			fn on_message(&mut self, msg: message::Message) -> Result<()> {
				Ok(assert_eq!(msg, message::Message::Text(String::from("testme"))))
			}

			fn on_close(&mut self, code: CloseCode, _: &str) {
				assert_eq!(code, CloseCode::Normal)
			}
		}

		let mut h = H;
		let url = url::Url::parse("127.0.0.1:3012").unwrap();
		h.on_open().unwrap();
		h.on_message(message::Message::Text("testme".to_owned())).unwrap();
		h.on_close(CloseCode::Normal, "");
	}

	#[test]
	fn closure_handler() {
		let mut close = |msg| {
			assert_eq!(msg, message::Message::Binary(vec![1, 2, 3]));
			Ok(())
		};

		close.on_message(message::Message::Binary(vec![1, 2, 3])).unwrap();
	}
}
