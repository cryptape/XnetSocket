
extern crate XnetSocket;
#[macro_use]
extern crate log;
extern crate env_logger;

use ws::listen;

fn main() {
	// Setup logging
	env_logger::init().unwrap();

	// Listen on an address and call the closure for each connection
	if let Err(error) = listen("127.0.0.1:3012".to_string(), |out| {
		// The handler needs to take ownership of out, so we use move
		move |msg| {
			// Handle messages received on this connection
			info!("Server got message '{}'. ", msg);

			// Use the out channel to send messages back
			out.send(msg)
		}
	})
	{
		// Inform the user of failure
		println!("Failed to create Socket due to {:?}", error);
	}
}
