extern crate XnetSocket;
#[macro_use]
extern crate log;
extern crate env_logger;

use ws::{connect, CloseCode};

fn main() {
	// Setup logging
	env_logger::init().unwrap();

	// Connect to the url and call the closure
	if let Err(error) = connect("127.0.0.1:3012".to_string(), |out| {

		if let Err(_) = out.send("Hello") {
			println!("socket couldn't queue an initial message.")
		} else {
			println!("Client sent message 'Hello Socket'. ")
		}

		// The handler needs to take ownership of out, so we use move
		move |msg| {
			// Handle messages received on this connection
			println!("Client got message '{}'. ", msg);

			// Close the connection
			out.close(CloseCode::Normal)
		}
	})
	{

		println!("Failed to create Socket due to: {:?}", error);
	}
}
