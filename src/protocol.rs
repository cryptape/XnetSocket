

use self::OpCode::*;
use std::convert::{Into, From};
use std::fmt;
/// Operation codes as part of rfc6455.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum OpCode {
	/// Indicates a continuation frame of a fragmented message.
	Continue,
	/// Indicates a text data frame.
	Text,
	/// Indicates a binary data frame.
	Binary,
	/// Indicates a close control frame.
	Close,
	/// Indicates a ping control frame.
	Ping,
	/// Indicates a pong control frame.
	Pong,
	/// Indicates an invalid opcode was received.
	Bad,
}

impl OpCode {
	/// Test whether the opcode indicates a control frame.
	pub fn is_control(&self) -> bool {
		match *self {
			Text | Binary | Continue => false,
			_ => true,
		}
	}
}

impl fmt::Display for OpCode {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			Continue => write!(f, "CONTINUE"),
			Text => write!(f, "TEXT"),
			Binary => write!(f, "BINARY"),
			Close => write!(f, "CLOSE"),
			Ping => write!(f, "PING"),
			Pong => write!(f, "PONG"),
			Bad => write!(f, "BAD"),
		}
	}
}

impl Into<u8> for OpCode {
	fn into(self) -> u8 {
		match self {
			Continue => 0,
			Text => 1,
			Binary => 2,
			Close => 8,
			Ping => 9,
			Pong => 10,
			Bad => {
				debug_assert!(false, "Attempted to convert invalid opcode to u8. This is a bug.");
				8 // if this somehow happens, a close frame will help us tear down quickly
			}
		}
	}
}

impl From<u8> for OpCode {
	fn from(byte: u8) -> OpCode {
		match byte {
			0 => Continue,
			1 => Text,
			2 => Binary,
			8 => Close,
			9 => Ping,
			10 => Pong,
			_ => Bad,
		}
	}
}

use self::CloseCode::*;

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum CloseCode {
	Normal,
	Away,
	Protocol,
	Unsupported,
	Status,
	Abnormal,
	Invalid,
	Policy,
	Size,
	Extension,
	Error,
	Restart,
	Again,
	#[doc(hidden)]
	Tls,
	#[doc(hidden)]
	Empty,
	#[doc(hidden)]
	Other(u16),
}

impl Into<u16> for CloseCode {
	fn into(self) -> u16 {
		match self {
			Normal => 1000,
			Away => 1001,
			Protocol => 1002,
			Unsupported => 1003,
			Status => 1005,
			Abnormal => 1006,
			Invalid => 1007,
			Policy => 1008,
			Size => 1009,
			Extension => 1010,
			Error => 1011,
			Restart => 1012,
			Again => 1013,
			Tls => 1015,
			Empty => 0,
			Other(code) => code,
		}
	}
}

impl From<u16> for CloseCode {
	fn from(code: u16) -> CloseCode {
		match code {
			1000 => Normal,
			1001 => Away,
			1002 => Protocol,
			1003 => Unsupported,
			1005 => Status,
			1006 => Abnormal,
			1007 => Invalid,
			1008 => Policy,
			1009 => Size,
			1010 => Extension,
			1011 => Error,
			1012 => Restart,
			1013 => Again,
			1015 => Tls,
			0 => Empty,
			_ => Other(code),
		}
	}
}


mod test {
#![allow(unused_imports, unused_variables, dead_code)]
	use super::*;

	#[test]
	fn opcode_from_u8() {
		let byte = 2u8;
		assert_eq!(OpCode::from(byte), OpCode::Binary);
	}

	#[test]
	fn opcode_into_u8() {
		let text = OpCode::Text;
		let byte: u8 = text.into();
		assert_eq!(byte, 1u8);
	}

	#[test]
	fn closecode_from_u16() {
		let byte = 1008u16;
		assert_eq!(CloseCode::from(byte), CloseCode::Policy);
	}

	#[test]
	fn closecode_into_u16() {
		let text = CloseCode::Away;
		let byte: u16 = text.into();
		assert_eq!(byte, 1001u16);
	}
}
